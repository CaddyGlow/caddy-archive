#[cfg(feature = "parallel")]
use crate::BatchReport;
use crate::{CreateEntry, CreateOptions, Entry, EntryId, EntryKind, Error, Limits, Result};
#[path = "sevenz_format.rs"]
mod container;

fn selected_folders<R>(
    backend: &SevenZip<R>,
    ids: &[EntryId],
) -> Result<(BTreeSet<usize>, BTreeSet<usize>, u64)> {
    let mut selected = BTreeSet::new();
    let mut folders = BTreeSet::new();
    let mut bytes = 0u64;
    for id in ids {
        let file = backend
            .archive
            .files
            .get(id.0)
            .ok_or_else(|| Error::Malformed("7z entry id".into()))?;
        if !selected.insert(id.0) {
            return Err(Error::Malformed("duplicate selected 7z entry".into()));
        }
        bytes = bytes
            .checked_add(file.size)
            .ok_or(Error::ResourceLimit("selected 7z bytes"))?;
        if let Some(folder) = backend.archive.stream_map.file_block_index[id.0] {
            folders.insert(folder);
        }
    }
    let required_output = folders.iter().try_fold(0u64, |total, &folder| {
        total
            .checked_add(backend.archive.blocks[folder].get_unpack_size())
            .ok_or(Error::ResourceLimit("7z selected folder work"))
    })?;
    if required_output > backend.limits.max_total_bytes {
        return Err(Error::ResourceLimit("7z selected folder work"));
    }
    Ok((selected, folders, bytes))
}

struct DecodeContext<'a> {
    archive: &'a SevenArchive,
    password: &'a Password,
    selected: &'a BTreeSet<usize>,
    limit: u64,
}
fn decode_folder<R: Read + Seek>(
    source: &mut R,
    context: DecodeContext<'_>,
    folder: usize,
    cancelled: &impl Fn() -> bool,
    sink: &mut impl FnMut(EntryId, &[u8]) -> Result<()>,
) -> Result<u64> {
    let DecodeContext {
        archive,
        password,
        selected,
        limit,
    } = context;
    let first = archive.stream_map.block_first_file_index[folder];
    let mut next = first;
    let mut total = 0u64;
    container::for_folder(source, archive, folder, password, &mut |file, reader| {
        let id = EntryId(next);
        next += 1;
        let mut actual = 0u64;
        let mut chunk = [0; 65536];
        loop {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            let n = reader.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            total = total
                .checked_add(n as u64)
                .ok_or(Error::ResourceLimit("7z folder decoded bytes"))?;
            actual += n as u64;
            if total > limit || actual > file.size {
                return Err(Error::ResourceLimit("7z decoded folder bytes"));
            }
            if selected.contains(&id.0)
                && let Err(error) = sink(id, &chunk[..n])
            {
                return Err(error);
            }
        }
        if actual != file.size {
            return Err(Error::Integrity("decoded file size mismatch".into()));
        }
        Ok(true)
    })?;
    Ok(total)
}

pub(crate) fn extract_selected<R: Read + Seek>(
    backend: &mut SevenZip<R>,
    ids: &[EntryId],
    sink: &mut impl FnMut(EntryId, &[u8]) -> Result<()>,
) -> Result<crate::ExtractReport> {
    extract_selected_cancellable(backend, ids, &|| false, sink)
}
pub(crate) fn extract_selected_cancellable<R: Read + Seek>(
    backend: &mut SevenZip<R>,
    ids: &[EntryId],
    cancelled: &impl Fn() -> bool,
    sink: &mut impl FnMut(EntryId, &[u8]) -> Result<()>,
) -> Result<crate::ExtractReport> {
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let (selected, folders, bytes) = selected_folders(backend, ids)?;
    let mut total = 0u64;
    for folder in folders {
        total = total
            .checked_add(decode_folder(
                &mut backend.source,
                DecodeContext {
                    archive: &backend.archive,
                    password: &backend.password,
                    selected: &selected,
                    limit: backend.limits.max_total_bytes.saturating_sub(total),
                },
                folder,
                cancelled,
                sink,
            )?)
            .ok_or(Error::ResourceLimit("7z decoded operation bytes"))?;
    }
    Ok(crate::ExtractReport {
        bytes,
        entries: ids.len() as u64,
        verified: true,
    })
}

fn folder_workspace(block: &container::Folder) -> Result<u64> {
    let mut bytes = 65536u64;
    for coder in &block.coders {
        let props = coder.properties();
        let memory = match coder.encoder_method_id() {
            container::LZMA => {
                if props.len() < 5 {
                    return Err(Error::Malformed("LZMA properties".into()));
                }
                ms_compress::lzma::lzma_get_memory_usage_by_props(
                    u32::from_le_bytes(
                        props[1..5]
                            .try_into()
                            .map_err(|_| Error::Malformed("dictionary".into()))?,
                    ),
                    props[0],
                )? as u64
                    * 1024
            }
            container::LZMA2 => {
                let p = *props
                    .first()
                    .ok_or_else(|| Error::Malformed("LZMA2 properties".into()))?;
                if p > 40 {
                    return Err(Error::Malformed("LZMA2 dictionary".into()));
                }
                let dict = if p == 40 {
                    u32::MAX
                } else {
                    (2u32 | u32::from(p & 1)) << (u32::from(p) / 2 + 11)
                };
                ms_compress::lzma::lzma2_get_memory_usage(dict) as u64 * 1024
            }
            [3, 3, 1, 0x1b] => 4 * 65536,
            container::BZIP2 => 16 * 1024 * 1024,
            container::BROTLI => container::BROTLI_WORKSPACE,
            container::DEFLATE => 1 << 20,
            _ => 65536,
        };
        bytes = bytes
            .checked_add(memory)
            .ok_or(Error::ResourceLimit("7z workspace bytes"))?;
    }
    Ok(bytes)
}

#[cfg(feature = "parallel")]
pub(crate) fn extract_selected_parallel<R: Read + Seek + Send>(
    backend: &mut SevenZip<R>,
    ids: &[EntryId],
    requested_workers: usize,
    max_workspace: u64,
    cancelled: &(impl Fn() -> bool + Sync),
    sink: &mut impl FnMut(EntryId, &[u8]) -> Result<()>,
) -> Result<BatchReport> {
    let (selected, folders, bytes) = selected_folders(backend, ids)?;
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let tasks = folders.len();
    let largest_workspace = folders
        .iter()
        .map(|&f| folder_workspace(&backend.archive.blocks[f]))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .unwrap_or(0);
    let pending_per_worker = 3 * 65536u64;
    let worker_memory = largest_workspace
        .checked_add(pending_per_worker)
        .ok_or(Error::ResourceLimit("7z workspace bytes"))?;
    let memory_workers = usize::try_from(max_workspace.checked_div(worker_memory).unwrap_or(1))
        .unwrap_or(usize::MAX);
    let pending_workers =
        usize::try_from(backend.limits.max_pending_output_bytes / pending_per_worker)
            .unwrap_or(usize::MAX);
    let mut workers = requested_workers
        .max(1)
        .min(backend.limits.max_workers)
        .min(tasks.max(1))
        .min(memory_workers)
        .min(pending_workers);
    if tasks > 0 && workers == 0 {
        return Err(Error::ResourceLimit(
            "7z aggregate workspace or pending output",
        ));
    }
    if !cfg!(all(feature = "parallel", not(target_arch = "wasm32"))) {
        workers = 1;
    }
    let fallback_reason = if requested_workers > workers {
        Some(
            if tasks <= 1 && backend.archive.is_solid {
                "selected entries share a dependent solid folder"
            } else if tasks <= 1 {
                "only one independent folder selected"
            } else if !cfg!(all(feature = "parallel", not(target_arch = "wasm32"))) {
                "native parallel feature unavailable"
            } else {
                "operation worker, workspace, or pending-output budget"
            }
            .to_owned(),
        )
    } else {
        None
    };
    let mut decoded_bytes = 0u64;
    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    if workers > 1 {
        use std::sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
            mpsc::sync_channel,
        };
        enum Event {
            Data(EntryId, Vec<u8>),
            Done(Result<u64>),
        }
        let source = Mutex::new(&mut backend.source);
        let queue = Mutex::new(std::collections::VecDeque::from_iter(
            folders.iter().copied(),
        ));
        let failed = AtomicBool::new(false);
        let (tx, rx) = sync_channel::<Event>(workers * 2);
        let archive = &backend.archive;
        let password = &backend.password;
        let limit = backend.limits.max_total_bytes;
        let mut operation_error = None;
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..workers {
                let tx = tx.clone();
                let selected = &selected;
                let source = &source;
                let queue = &queue;
                let failed = &failed;
                handles.push(scope.spawn(move || {
                    loop {
                        if failed.load(Ordering::Relaxed) || cancelled() {
                            break;
                        }
                        let task = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
                        let Some(folder) = task else {
                            break;
                        };
                        let mut reader = SharedSource {
                            source,
                            position: 0,
                        };
                        let result = decode_folder(
                            &mut reader,
                            DecodeContext {
                                archive,
                                password,
                                selected,
                                limit,
                            },
                            folder,
                            &|| failed.load(Ordering::Relaxed) || cancelled(),
                            &mut |id, data| {
                                tx.send(Event::Data(id, data.to_vec()))
                                    .map_err(|_| Error::Cancelled)
                            },
                        );
                        let is_error = result.is_err();
                        if is_error {
                            failed.store(true, Ordering::Relaxed);
                        }
                        if tx.send(Event::Done(result)).is_err() || is_error {
                            break;
                        }
                    }
                }));
            }
            drop(tx);
            for event in rx {
                match event {
                    Event::Data(id, data) => {
                        if operation_error.is_none()
                            && let Err(error) = sink(id, &data)
                        {
                            operation_error = Some(error);
                            failed.store(true, Ordering::Relaxed);
                        }
                    }
                    Event::Done(result) => match result {
                        Ok(count) => {
                            decoded_bytes = decoded_bytes.saturating_add(count);
                            if decoded_bytes > limit && operation_error.is_none() {
                                operation_error =
                                    Some(Error::ResourceLimit("7z decoded operation bytes"));
                                failed.store(true, Ordering::Relaxed);
                            }
                        }
                        Err(error) => {
                            if operation_error.is_none() {
                                operation_error = Some(error);
                            }
                        }
                    },
                }
            }
            for handle in handles {
                if handle.join().is_err() && operation_error.is_none() {
                    operation_error = Some(Error::Malformed("7z worker panicked".into()));
                }
            }
        });
        if let Some(error) = operation_error {
            return Err(error);
        }
    } else {
        for &folder in &folders {
            decoded_bytes += decode_folder(
                &mut backend.source,
                DecodeContext {
                    archive: &backend.archive,
                    password: &backend.password,
                    selected: &selected,
                    limit: backend.limits.max_total_bytes.saturating_sub(decoded_bytes),
                },
                folder,
                cancelled,
                sink,
            )?;
        }
    }
    #[cfg(not(all(feature = "parallel", not(target_arch = "wasm32"))))]
    for &folder in &folders {
        decoded_bytes += decode_folder(
            &mut backend.source,
            DecodeContext {
                archive: &backend.archive,
                password: &backend.password,
                selected: &selected,
                limit: backend.limits.max_total_bytes.saturating_sub(decoded_bytes),
            },
            folder,
            cancelled,
            sink,
        )?;
    }
    if cancelled() {
        return Err(Error::Cancelled);
    }
    Ok(BatchReport {
        report: crate::ExtractReport {
            bytes,
            entries: ids.len() as u64,
            verified: true,
        },
        workers_used: workers,
        folder_tasks: tasks,
        decoded_bytes,
        fallback_reason,
    })
}

#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
struct SharedSource<'a, R> {
    source: &'a std::sync::Mutex<&'a mut R>,
    position: u64,
}
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
impl<R: Read + Seek> Read for SharedSource<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut reader = self.source.lock().unwrap_or_else(|e| e.into_inner());
        reader.seek(SeekFrom::Start(self.position))?;
        let n = reader.read(buf)?;
        self.position = self
            .position
            .checked_add(n as u64)
            .ok_or_else(|| io::Error::other("range position overflow"))?;
        Ok(n)
    }
}
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
impl<R: Read + Seek> Seek for SharedSource<'_, R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.position = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(n) => self
                .position
                .checked_add_signed(n)
                .ok_or_else(|| io::Error::other("range seek overflow"))?,
            SeekFrom::End(n) => self
                .source
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .seek(SeekFrom::End(n))?,
        };
        Ok(self.position)
    }
}
use container::{Password, SevenArchive};
#[cfg(all(feature = "parallel", any(test, not(target_arch = "wasm32"))))]
use std::io;
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom, Write},
};

pub(crate) struct SevenZip<R> {
    source: R,
    archive: SevenArchive,
    password: Password,
    limits: Limits,
}

pub(crate) fn index<R: Read + Seek>(
    mut source: R,
    limits: Limits,
    password: Option<&[u8]>,
) -> Result<(SevenZip<R>, Vec<Entry>)> {
    let size = source.seek(SeekFrom::End(0))?;
    if size > limits.max_input_bytes {
        return Err(Error::ResourceLimit("7z input bytes"));
    }
    source.seek(SeekFrom::Start(0))?;
    let mut password = match password {
        Some(bytes) => Password::new(
            std::str::from_utf8(bytes)
                .map_err(|_| Error::Unsupported("7z password must be UTF-8 text".into()))?,
        ),
        None => Password::empty(),
    };
    if password.bytes.is_some() && limits.max_password_iterations == 0 {
        return Err(Error::ResourceLimit("7z password derivation work"));
    }
    password.limits = limits;
    let archive = SevenArchive::read(&mut source, &password)?;
    for block in &archive.blocks {
        if folder_workspace(block)? > limits.max_active_workspace_bytes {
            return Err(Error::ResourceLimit("7z decoder workspace"));
        }
        for i in 0..block.coders.len() {
            if block.get_unpack_size_at_index(i) > limits.max_total_bytes
                || usize::try_from(block.get_unpack_size_at_index(i)).is_err()
            {
                return Err(Error::ResourceLimit("7z intermediate coder output"));
            }
        }
    }
    if archive.files.len() as u64 > limits.max_entries {
        return Err(Error::ResourceLimit("7z entries"));
    }
    let mut names = BTreeSet::new();
    let mut entries = Vec::new();
    let mut decoded = 0u64;
    let mut metadata = 0u64;
    for (i, file) in archive.files.iter().enumerate() {
        if !names.insert(file.name.as_str()) {
            return Err(Error::Malformed("duplicate 7z name".into()));
        }
        if file.size > limits.max_entry_bytes || usize::try_from(file.size).is_err() {
            return Err(Error::ResourceLimit("7z entry bytes"));
        }
        decoded = decoded
            .checked_add(file.size)
            .ok_or(Error::ResourceLimit("7z decoded bytes"))?;
        metadata = metadata
            .checked_add(file.name.len() as u64)
            .ok_or(Error::ResourceLimit("7z metadata"))?;
        if decoded > limits.max_total_bytes || metadata > limits.max_metadata_bytes {
            return Err(Error::ResourceLimit("7z decoded bytes or metadata"));
        }
        let block = archive
            .stream_map
            .file_block_index
            .get(i)
            .copied()
            .flatten()
            .map(|b| &archive.blocks[b]);
        let mut methods = Vec::new();
        let mut encrypted = false;
        if let Some(block) = block {
            for coder in &block.coders {
                let method = container::method_name(coder.encoder_method_id())?;
                encrypted |= coder.encoder_method_id() == container::AES;
                methods.push(method);
            }
            if block.get_unpack_size() > limits.max_total_bytes {
                return Err(Error::ResourceLimit("7z folder output"));
            }
        }
        entries.push(Entry {
            id: EntryId(i),
            name: file.name.clone(),
            raw_name: file
                .name
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect(),
            kind: if file.is_directory {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: file.size,
            compressed_size: Some(file.compressed_size),
            compression: methods.join("+"),
            encrypted,
        });
    }
    Ok((
        SevenZip {
            source,
            archive,
            password,
            limits,
        },
        entries,
    ))
}

pub(crate) fn extract<R: Read + Seek>(
    backend: &mut SevenZip<R>,
    id: EntryId,
    writer: &mut impl Write,
) -> Result<u64> {
    let mut bytes = 0u64;
    extract_selected(backend, &[id], &mut |_, chunk| {
        writer.write_all(chunk)?;
        bytes = bytes
            .checked_add(chunk.len() as u64)
            .ok_or(Error::ResourceLimit("7z extracted bytes"))?;
        Ok(())
    })?;
    Ok(bytes)
}

pub(crate) fn metadata<R>(backend: &SevenZip<R>, id: EntryId) -> Result<crate::EntryMetadata> {
    backend
        .archive
        .files
        .get(id.0)
        .map(|file| file.metadata.clone())
        .ok_or_else(|| Error::Malformed("unknown 7z entry ID".into()))
}
pub(crate) fn create<W: Write + Seek>(
    entries: &[CreateEntry],
    output: W,
    options: &mut CreateOptions<'_>,
    limits: Limits,
) -> Result<()> {
    container::write(entries, output, options, limits)
}

pub(crate) fn create_readers<'a, E: crate::CreationEntry>(
    entries: &[E],
    open: &mut impl FnMut(usize) -> Result<Box<dyn Read + 'a>>,
    output: impl Write + Seek,
    options: &mut CreateOptions<'_>,
    limits: Limits,
) -> Result<()> {
    container::write_readers(entries, open, output, options, limits)
}

pub(crate) fn edit_archive<R: Read + Seek, W: Write + Seek>(
    source: &mut R,
    output: &mut W,
    operations: &[crate::sevenz_edit::EditOperation],
    options: crate::sevenz_edit::EditOptions<'_>,
    limits: Limits,
) -> Result<crate::sevenz_edit::EditReport> {
    container::edit_archive(source, output, operations, options, limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn create<W: Write + Seek>(
        entries: &[CreateEntry],
        output: W,
        options: &mut CreateOptions<'_>,
    ) -> Result<()> {
        super::create(entries, output, options, Limits::default())
    }

    #[test]
    fn selectable_writer_codecs_roundtrip_and_report_exact_method() {
        use crate::SevenZipCompression;
        let methods = [
            SevenZipCompression::Copy,
            SevenZipCompression::Deflate,
            SevenZipCompression::Lzma,
            SevenZipCompression::Lzma2,
            SevenZipCompression::Bzip2,
            SevenZipCompression::Brotli,
        ];
        for method in methods {
            if method == SevenZipCompression::Bzip2 && !cfg!(feature = "bzip2")
                || method == SevenZipCompression::Brotli && !cfg!(feature = "brotli")
            {
                continue;
            }
            let mut bytes = Cursor::new(Vec::new());
            let entries = inputs();
            create(
                &entries,
                &mut bytes,
                &mut CreateOptions {
                    sevenz_compression: method,
                    ..Default::default()
                },
            )
            .unwrap();
            let (mut archive, listed) =
                index(Cursor::new(bytes.into_inner()), Limits::default(), None).unwrap();
            assert!(!listed[0].compression.is_empty());
            for (i, expected) in entries.iter().enumerate() {
                let mut actual = Vec::new();
                extract(&mut archive, EntryId(i), &mut actual).unwrap();
                assert_eq!(actual, expected.data, "{method:?}");
            }
        }
    }
    fn inputs() -> Vec<CreateEntry> {
        vec![
            CreateEntry {
                name: "first.txt".into(),
                data: b"first payload".to_vec(),
                kind: EntryKind::File,
            },
            CreateEntry {
                name: "last.txt".into(),
                data: b"last payload".to_vec(),
                kind: EntryKind::File,
            },
        ]
    }
    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    #[test]
    fn independent_workers_one_two_four_have_identical_verified_bytes() {
        let input: Vec<_> = (0..4)
            .map(|i| CreateEntry {
                name: format!("{i}.bin"),
                data: vec![i as u8; 256 * 1024],
                kind: EntryKind::File,
            })
            .collect();
        let mut bytes = io::Cursor::new(Vec::new());
        create(&input, &mut bytes, &mut CreateOptions::default()).unwrap();
        let bytes = bytes.into_inner();
        for workers in [1, 2, 4] {
            let (mut backend, entries) =
                index(io::Cursor::new(&bytes), Limits::default(), None).unwrap();
            let mut output = std::collections::BTreeMap::<usize, Vec<u8>>::new();
            let report = extract_selected_parallel(
                &mut backend,
                &entries.iter().map(|e| e.id).collect::<Vec<_>>(),
                workers,
                Limits::default().max_active_workspace_bytes,
                &|| false,
                &mut |id, bytes| {
                    output.entry(id.0).or_default().extend_from_slice(bytes);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(report.workers_used, workers);
            assert_eq!(report.folder_tasks, 4);
            assert_eq!(report.decoded_bytes, 1024 * 1024);
            assert!(report.report.verified);
            for (i, entry) in input.iter().enumerate() {
                assert_eq!(output[&i], entry.data);
            }
        }
    }
    #[cfg(feature = "parallel")]
    #[test]
    fn solid_filter_pipeline_decodes_once_and_reports_sequential_reason() {
        let mut input = inputs();
        for entry in &mut input {
            entry.data.push(b'\n');
        }
        let (mut backend, entries) = index(
            io::Cursor::new(
                include_bytes!("../tests/fixtures/sevenz-independent/solid-bcj-lzma2.7z")
                    .as_slice(),
            ),
            Limits::default(),
            None,
        )
        .unwrap();
        let mut output = std::collections::BTreeMap::<usize, Vec<u8>>::new();
        let report = extract_selected_parallel(
            &mut backend,
            &entries.iter().map(|e| e.id).collect::<Vec<_>>(),
            4,
            Limits::default().max_active_workspace_bytes,
            &|| false,
            &mut |id, bytes| {
                output.entry(id.0).or_default().extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(report.workers_used, 1);
        assert_eq!(report.folder_tasks, 1);
        assert_eq!(
            report.decoded_bytes,
            input.iter().map(|e| e.data.len() as u64).sum::<u64>()
        );
        assert!(report.fallback_reason.unwrap().contains("solid"));
        for (i, entry) in input.iter().enumerate() {
            assert_eq!(output[&i], entry.data);
        }
    }
    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    #[test]
    fn cancellation_sink_drop_worker_failure_and_aggregate_memory_are_bounded() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let mut input = inputs();
        for e in &mut input {
            e.data = vec![42; 1024 * 1024];
        }
        let mut bytes = io::Cursor::new(Vec::new());
        create(&input, &mut bytes, &mut CreateOptions::default()).unwrap();
        let bytes = bytes.into_inner();
        let (mut backend, entries) =
            index(io::Cursor::new(&bytes), Limits::default(), None).unwrap();
        let ids: Vec<_> = entries.iter().map(|e| e.id).collect();
        let cancelled = AtomicBool::new(false);
        assert!(matches!(
            extract_selected_parallel(
                &mut backend,
                &ids,
                4,
                256 << 20,
                &|| cancelled.load(Ordering::Relaxed),
                &mut |_, _| {
                    cancelled.store(true, Ordering::Relaxed);
                    Ok(())
                }
            ),
            Err(Error::Cancelled)
        ));
        assert!(matches!(
            extract_selected_parallel(&mut backend, &ids, 4, 1, &|| false, &mut |_, _| Ok(())),
            Err(Error::ResourceLimit(_))
        ));
        assert!(
            extract_selected_parallel(
                &mut backend,
                &ids,
                2,
                256 << 20,
                &|| false,
                &mut |_, _| Err(Error::Io(io::ErrorKind::BrokenPipe.into()))
            )
            .is_err()
        );
        struct Failing {
            bytes: io::Cursor<Vec<u8>>,
            fail: Arc<AtomicBool>,
        }
        impl Read for Failing {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.fail.load(Ordering::Relaxed) {
                    return Err(io::Error::other("injected read failure"));
                }
                self.bytes.read(buf)
            }
        }
        impl Seek for Failing {
            fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
                self.bytes.seek(pos)
            }
        }
        let fail = Arc::new(AtomicBool::new(false));
        let (mut backend, _) = index(
            Failing {
                bytes: io::Cursor::new(bytes),
                fail: fail.clone(),
            },
            Limits::default(),
            None,
        )
        .unwrap();
        fail.store(true, Ordering::Relaxed);
        assert!(
            extract_selected_parallel(&mut backend, &ids, 2, 256 << 20, &|| false, &mut |_, _| Ok(
                ()
            ))
            .is_err()
        );
    }
    #[test]
    #[ignore = "requires independent 7z command"]
    fn independent_7zip_reads_output_and_produces_solid_encrypted_input() {
        use std::process::Command;
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("ours.7z");
        let mut file = std::fs::File::create(&output).unwrap();
        create(&inputs(), &mut file, &mut CreateOptions::default()).unwrap();
        assert!(
            Command::new("7z")
                .arg("t")
                .arg(&output)
                .output()
                .unwrap()
                .status
                .success()
        );
        let extracted = temp.path().join("extracted");
        assert!(
            Command::new("7z")
                .arg("x")
                .arg(&output)
                .arg(format!("-o{}", extracted.display()))
                .output()
                .unwrap()
                .status
                .success()
        );
        for input in inputs() {
            assert_eq!(
                std::fs::read(extracted.join(input.name)).unwrap(),
                input.data
            );
        }
        for method in [
            crate::SevenZipCompression::Copy,
            crate::SevenZipCompression::Lzma,
            crate::SevenZipCompression::Bzip2,
        ] {
            if method == crate::SevenZipCompression::Bzip2 && !cfg!(feature = "bzip2") {
                continue;
            }
            let archive = temp.path().join(format!("writer-{method:?}.7z"));
            create(
                &inputs(),
                std::fs::File::create(&archive).unwrap(),
                &mut CreateOptions {
                    sevenz_compression: method,
                    ..Default::default()
                },
            )
            .unwrap();
            let destination = temp.path().join(format!("writer-{method:?}"));
            assert!(
                Command::new("7z")
                    .arg("x")
                    .arg(&archive)
                    .arg(format!("-o{}", destination.display()))
                    .output()
                    .unwrap()
                    .status
                    .success(),
                "{method:?}"
            );
            for entry in inputs() {
                assert_eq!(
                    std::fs::read(destination.join(entry.name)).unwrap(),
                    entry.data
                );
            }
        }
        for input in inputs() {
            std::fs::write(temp.path().join(input.name), input.data).unwrap();
        }
        for method in ["Copy", "LZMA", "LZMA2"] {
            let name = format!("independent-{method}.7z");
            assert!(
                Command::new("7z")
                    .current_dir(temp.path())
                    .args([
                        "a",
                        "-ms=on",
                        &format!("-m0={method}"),
                        &name,
                        "first.txt",
                        "last.txt"
                    ])
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            let (mut reader, entries) = index(
                std::fs::File::open(temp.path().join(name)).unwrap(),
                Limits::default(),
                None,
            )
            .unwrap();
            for input in inputs() {
                let id = entries.iter().find(|e| e.name == input.name).unwrap().id;
                let mut out = Vec::new();
                extract(&mut reader, id, &mut out).unwrap();
                assert_eq!(out, input.data);
            }
        }
        #[cfg(feature = "crypto")]
        {
            // A public fixture password, not a user's credential.
            assert!(
                Command::new("7z")
                    .current_dir(temp.path())
                    .args([
                        "a",
                        "-ms=on",
                        "-m0=LZMA2",
                        "-pfixture",
                        "-mhe=on",
                        "encrypted.7z",
                        "first.txt",
                        "last.txt"
                    ])
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            let (mut reader, entries) = index(
                std::fs::File::open(temp.path().join("encrypted.7z")).unwrap(),
                Limits::default(),
                Some(b"fixture"),
            )
            .unwrap();
            for input in inputs() {
                let id = entries.iter().find(|e| e.name == input.name).unwrap().id;
                let mut out = Vec::new();
                extract(&mut reader, id, &mut out).unwrap();
                assert_eq!(out, input.data);
            }
        }
    }
    #[test]
    fn independent_lzma2_folders_roundtrip_and_truncations_fail() {
        let mut bytes = Cursor::new(Vec::new());
        create(&inputs(), &mut bytes, &mut CreateOptions::default()).unwrap();
        let bytes = bytes.into_inner();
        let (mut reader, entries) = index(Cursor::new(&bytes), Limits::default(), None).unwrap();
        assert_eq!(entries.len(), 2);
        for (i, expected) in inputs().iter().enumerate() {
            let mut out = Vec::new();
            extract(&mut reader, EntryId(i), &mut out).unwrap();
            assert_eq!(out, expected.data);
        }
        for cut in 0..bytes.len() {
            assert!(
                index(Cursor::new(&bytes[..cut]), Limits::default(), None).is_err(),
                "accepted truncation {cut}"
            );
        }
    }
    #[test]
    fn solid_copy_lzma_lzma2_and_bcj_decode_shared_folder() {
        for bytes in [
            include_bytes!("../tests/fixtures/sevenz-independent/solid-copy.7z").as_slice(),
            include_bytes!("../tests/fixtures/sevenz-independent/solid-lzma.7z").as_slice(),
            include_bytes!("../tests/fixtures/sevenz-independent/solid-lzma2.7z").as_slice(),
            include_bytes!("../tests/fixtures/sevenz-independent/solid-bcj-lzma2.7z").as_slice(),
        ] {
            let (mut reader, _) = index(Cursor::new(bytes), Limits::default(), None).unwrap();
            let mut out = Vec::new();
            extract(&mut reader, EntryId(1), &mut out).unwrap();
            assert_eq!(out, b"last payload\n");
        }
    }
    #[test]
    fn dictionary_and_metadata_limits_apply_before_decoding() {
        let mut bytes = Cursor::new(Vec::new());
        create(&inputs(), &mut bytes, &mut CreateOptions::default()).unwrap();
        let bytes = bytes.into_inner();
        let small = Limits {
            max_dictionary_bytes: 4096,
            ..Limits::default()
        };
        let result = index(Cursor::new(&bytes), small, None);
        if let Ok((mut reader, _)) = result {
            assert!(extract(&mut reader, EntryId(0), &mut Vec::new()).is_err());
        }
        assert!(
            index(
                Cursor::new(bytes),
                Limits {
                    max_metadata_bytes: 1,
                    ..Limits::default()
                },
                None
            )
            .is_err()
        );
    }
    #[cfg(feature = "crypto")]
    #[test]
    fn aes_payload_header_encryption_passwords_and_fresh_randomness() {
        struct Random(u8);
        impl crate::RandomSource for Random {
            fn fill(&mut self, out: &mut [u8]) -> Result<()> {
                for byte in out {
                    self.0 = self.0.wrapping_add(1);
                    *byte = self.0;
                }
                Ok(())
            }
        }
        for encrypt_headers in [false, true] {
            let mut random = Random(0);
            let mut bytes = Cursor::new(Vec::new());
            let mut options = CreateOptions {
                password: Some(b"test password"),
                randomness: Some(&mut random),
                encrypt_headers,
                ..CreateOptions::default()
            };
            create(&inputs(), &mut bytes, &mut options).unwrap();
            let bytes = bytes.into_inner();
            let (mut reader, entries) = index(
                Cursor::new(&bytes),
                Limits::default(),
                Some(b"test password"),
            )
            .unwrap();
            assert!(entries.iter().all(|e| e.encrypted));
            let properties: Vec<_> = reader
                .archive
                .blocks
                .iter()
                .map(|b| {
                    b.coders
                        .iter()
                        .find(|c| c.encoder_method_id() == container::AES)
                        .unwrap()
                        .properties()
                })
                .collect();
            assert_ne!(properties[0], properties[1]);
            let mut out = Vec::new();
            extract(&mut reader, EntryId(1), &mut out).unwrap();
            assert_eq!(out, inputs()[1].data);
            if let Ok((mut reader, _)) =
                index(Cursor::new(&bytes), Limits::default(), Some(b"wrong"))
            {
                assert!(extract(&mut reader, EntryId(0), &mut Vec::new()).is_err())
            }
            if encrypt_headers {
                assert!(matches!(
                    index(Cursor::new(bytes), Limits::default(), None),
                    Err(Error::PasswordRequired)
                ));
            }
        }
    }
}
