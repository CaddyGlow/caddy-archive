#![cfg(all(feature = "crypto", feature = "zip"))]
use archive_core::{
    Archive, CreateEntry, CreateOptions, EntryId, EntryKind, Error, Format, Limits, RandomSource,
    create_with_options,
};
use std::io::Cursor;
struct TestRandom(u8);
impl RandomSource for TestRandom {
    fn fill(&mut self, bytes: &mut [u8]) -> archive_core::Result<()> {
        for byte in bytes {
            self.0 = self.0.wrapping_add(1);
            *byte = self.0;
        }
        Ok(())
    }
}
fn fixture() -> Vec<u8> {
    let entries = [CreateEntry {
        name: "secret.txt".into(),
        kind: EntryKind::File,
        data: b"private payload".to_vec(),
    }];
    let mut writer = Cursor::new(Vec::new());
    create_with_options(
        Format::Zip,
        &entries,
        &mut writer,
        Limits::default(),
        CreateOptions {
            password: Some(b"correct horse"),
            randomness: Some(&mut TestRandom(0)),
            encrypt_headers: false,
            zip_encryption: Default::default(),
            sevenz_compression: Default::default(),
            entry_metadata: None,
            zip_compression: Default::default(),
            cab_compression: Default::default(),
        },
    )
    .unwrap();
    writer.into_inner()
}
#[test]
fn aes_roundtrip_requires_password() {
    let data = fixture();
    let mut archive = Archive::open(Cursor::new(&data), Limits::default()).unwrap();
    assert!(archive.entries()[0].encrypted);
    assert!(matches!(archive.test(), Err(Error::PasswordRequired)));
    let mut archive =
        Archive::open_with_password(Cursor::new(data), Limits::default(), b"correct horse")
            .unwrap();
    assert_eq!(
        archive.read_entry(EntryId(0), 100).unwrap(),
        b"private payload"
    );
}
#[test]
fn wrong_password_publishes_no_plaintext() {
    let mut archive =
        Archive::open_with_password(Cursor::new(fixture()), Limits::default(), b"incorrect")
            .unwrap();
    let mut output = Vec::new();
    assert!(matches!(
        archive.extract(EntryId(0), &mut output),
        Err(Error::Integrity(_))
    ));
    assert!(output.is_empty());
}
#[test]
fn corrupted_authentication_publishes_no_plaintext() {
    let mut data = fixture();
    let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    data[central - 1] ^= 1;
    let mut archive =
        Archive::open_with_password(Cursor::new(data), Limits::default(), b"correct horse")
            .unwrap();
    let mut output = Vec::new();
    assert!(matches!(
        archive.extract(EntryId(0), &mut output),
        Err(Error::Integrity(_))
    ));
    assert!(output.is_empty());
}
#[test]
fn independent_7zip_decrypts_aes_zip() {
    if std::process::Command::new("7z").arg("i").output().is_err() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("archive.zip");
    std::fs::write(&source, fixture()).unwrap();
    let output = std::process::Command::new("7z")
        .args(["x", "-so", "-pcorrect horse"])
        .arg(source)
        .arg("secret.txt")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"private payload");
}
#[test]
fn explicit_legacy_zipcrypto_roundtrip_and_independent_reader() {
    let entry = CreateEntry {
        name: "secret.txt".into(),
        kind: EntryKind::File,
        data: b"legacy payload".to_vec(),
    };
    let mut writer = Cursor::new(Vec::new());
    create_with_options(
        Format::Zip,
        &[entry],
        &mut writer,
        Limits::default(),
        CreateOptions {
            password: Some(b"legacy password"),
            randomness: Some(&mut TestRandom(7)),
            zip_encryption: archive_core::ZipEncryption::ZipCrypto,
            ..Default::default()
        },
    )
    .unwrap();
    let data = writer.into_inner();
    let mut archive =
        Archive::open_with_password(Cursor::new(&data), Limits::default(), b"legacy password")
            .unwrap();
    assert_eq!(
        archive.read_entry(EntryId(0), 100).unwrap(),
        b"legacy payload"
    );
    if std::process::Command::new("7z").arg("i").output().is_err() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("archive.zip");
    std::fs::write(&source, data).unwrap();
    let output = std::process::Command::new("7z")
        .args(["x", "-so", "-plegacy password"])
        .arg(source)
        .arg("secret.txt")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"legacy payload");
}
#[test]
fn independent_aes_strengths_and_legacy_are_readable() {
    if std::process::Command::new("7z").arg("i").output().is_err() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("payload.txt"), b"independent payload").unwrap();
    for mode in ["AES128", "AES192", "AES256", "ZipCrypto"] {
        let source = temp.path().join(format!("{mode}.zip"));
        let status = std::process::Command::new("7z")
            .current_dir(temp.path())
            .args(["a", "-tzip", "-psecret", "-mm=Deflate"])
            .arg(format!("-mem={mode}"))
            .arg(&source)
            .arg("payload.txt")
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        let mut archive = Archive::open_with_password(
            std::fs::File::open(source).unwrap(),
            Limits::default(),
            b"secret",
        )
        .unwrap();
        assert_eq!(
            archive.read_entry(EntryId(0), 100).unwrap(),
            b"independent payload"
        );
    }
}
