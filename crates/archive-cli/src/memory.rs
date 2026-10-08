use std::ffi::OsString;

/// Normalize 7-Zip's method-property spelling before clap interprets short flags.
/// A literal `--` protects subsequent filenames from option normalization.
pub(crate) fn normalize_args(mut args: Vec<OsString>) -> Vec<OsString> {
    for argument in args.iter_mut().skip(1) {
        if argument == "--" {
            break;
        }
        if let Some(value) = argument
            .to_str()
            .and_then(|text| text.strip_prefix("-mmemuse="))
        {
            *argument = format!("--memuse={value}").into();
        }
    }
    args
}

/// Native RAM discovery; core policy applies its portable fallback on failure.
pub(crate) fn physical_memory() -> Option<u64> {
    native_physical_memory()
        .filter(|bytes| *bytes != 0)
        .map(|bytes| {
            #[cfg(not(windows))]
            {
                // Match 7-Zip's POSIX usable RAM base before applying percentages.
                bytes.min(1u64 << (usize::BITS - 1))
            }
            #[cfg(windows)]
            {
                bytes
            }
        })
}

pub(crate) fn available_workers() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn native_physical_memory() -> Option<u64> {
    // These sysconf keys return positive values or -1 on failure.
    let pages = u64::try_from(unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) }).ok()?;
    let page_size = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
    pages.checked_mul(page_size)
}

#[cfg(any(target_os = "macos", target_os = "freebsd"))]
fn native_physical_memory() -> Option<u64> {
    #[cfg(target_os = "macos")]
    let key = c"hw.memsize";
    #[cfg(target_os = "freebsd")]
    let key = c"hw.physmem";
    let mut bytes = 0u64;
    let mut length = std::mem::size_of_val(&bytes);
    let result = unsafe {
        libc::sysctlbyname(
            key.as_ptr(),
            (&mut bytes as *mut u64).cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0 && length == std::mem::size_of_val(&bytes)).then_some(bytes)
}

#[cfg(windows)]
fn native_physical_memory() -> Option<u64> {
    #[repr(C)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        total_physical: u64,
        available_physical: u64,
        total_page_file: u64,
        available_page_file: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
    }
    let mut status = MemoryStatus {
        length: std::mem::size_of::<MemoryStatus>() as u32,
        load: 0,
        total_physical: 0,
        available_physical: 0,
        total_page_file: 0,
        available_page_file: 0,
        total_virtual: 0,
        available_virtual: 0,
        available_extended_virtual: 0,
    };
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0)
        .then_some(status.total_physical.min(status.total_virtual))
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "freebsd",
    windows
)))]
fn native_physical_memory() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sevenzip_spelling_preserves_literal_paths() {
        let args = ["arc", "-mmemuse=p80", "list", "--", "-mmemuse=filename"];
        let normalized = normalize_args(args.into_iter().map(OsString::from).collect());
        assert_eq!(
            normalized,
            ["arc", "--memuse=p80", "list", "--", "-mmemuse=filename"].map(OsString::from)
        );
        let args = ["arc", "create", "--input=-mmemuse=name", "--output=x.tar"];
        assert_eq!(
            normalize_args(args.into_iter().map(OsString::from).collect()),
            args.map(OsString::from)
        );
    }
}
