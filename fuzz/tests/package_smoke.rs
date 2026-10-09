use ms_package::{Error, InstallerPackage, MediaResolver};
use std::io::Cursor;

struct ExplicitMedia(Option<&'static [u8]>);
impl MediaResolver for ExplicitMedia {
    fn resolve(&mut self, name: &str, maximum: u64) -> ms_package::Result<Vec<u8>> {
        let bytes = self.0.ok_or_else(|| Error::MissingMedia(name.into()))?;
        if bytes.len() as u64 > maximum {
            return Err(Error::Limit("test caller media bytes"));
        }
        Ok(bytes.to_vec())
    }
}

#[test]
fn retained_msi_media_reaches_the_published_bounded_backend() {
    let embedded = include_bytes!("../../crates/archive-wasm/tests/fixtures/package/embedded.msi");
    let external = include_bytes!("../../crates/archive-wasm/tests/fixtures/package/external.msi");
    let cabinet = include_bytes!("../../crates/archive-wasm/tests/fixtures/package/data.cab");
    for (source, media) in [
        (embedded.as_slice(), None),
        (external.as_slice(), Some(cabinet.as_slice())),
    ] {
        let mut package =
            InstallerPackage::open_bounded(Cursor::new(source), 128, 1 << 20).unwrap();
        let file = package.files().unwrap().remove(0);
        assert_eq!(
            package
                .read_file(&file, &mut ExplicitMedia(media), 1 << 20)
                .unwrap(),
            b"portable package payload\n"
        );
        assert!(matches!(
            package.read_file(&file, &mut ExplicitMedia(media), 1),
            Err(Error::Limit(_))
        ));
        if media.is_some() {
            assert!(matches!(
                package.read_file(&file, &mut ExplicitMedia(None), 1 << 20),
                Err(Error::MissingMedia(_))
            ));
        }
        archive_fuzz::archives::package(source);
        let mut truncated = source.to_vec();
        truncated.truncate(source.len() / 2);
        archive_fuzz::archives::package(&truncated);
    }
}
