//! Synthetic unsigned MSI media fixtures; run with the pinned dependencies in README.md.
use ms_package::authoring::{
    InstallationContext, InstallerArchitecture, InstallerBuilder, InstallerCabinetSpec,
    InstallerEditor, InstallerIdentity, InstallerMediaLayout, InstallerMediaSink, WriteOptions,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{self, Cursor},
    path::Path,
};

#[derive(Default)]
struct Media(BTreeMap<String, Vec<u8>>);
impl InstallerMediaSink for Media {
    type Writer = Vec<u8>;
    fn create(&mut self, _: &str) -> io::Result<Self::Writer> {
        Ok(Vec::new())
    }
    fn finish(&mut self, name: &str, bytes: Self::Writer) -> io::Result<()> {
        self.0.insert(name.into(), bytes);
        Ok(())
    }
}
impl ms_package::MediaResolver for Media {
    fn resolve(&mut self, name: &str, max: u64) -> ms_package::Result<Vec<u8>> {
        let bytes = self
            .0
            .get(name)
            .ok_or_else(|| ms_package::Error::MissingMedia(name.into()))?;
        if bytes.len() as u64 > max {
            return Err(ms_package::Error::Limit("synthetic media bytes"));
        }
        Ok(bytes.clone())
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn artifact(path: &Path, bytes: &[u8]) -> io::Result<serde_json::Value> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, bytes)?;
    Ok(serde_json::json!({"bytes":bytes.len(),"sha256":hash(bytes)}))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let destination = std::env::args_os()
        .nth(1)
        .ok_or("expected output directory")?;
    let destination = Path::new(&destination);
    let mut artifacts = serde_json::Map::new();
    let mut profiles = serde_json::Map::new();
    for profile in [
        "embedded",
        "external",
        "loose",
        "partitioned",
        "mixed",
        "mixed-loose",
    ] {
        let identity = InstallerIdentity {
            product_code: "{A09C9465-494B-4C24-BEFC-BCF975E23526}".into(),
            package_code: "{CE6F227A-D92D-45F9-BCAA-F432D2A2BFC6}".into(),
            upgrade_code: "{29079377-0EF5-4891-B138-B27804FAB594}".into(),
            name: "Synthetic media migration fixture".into(),
            manufacturer: "archive-rs tests".into(),
            version: "1.0.0".into(),
            directory_name: "MediaFixture".into(),
            architecture: InstallerArchitecture::X64,
            context: InstallationContext::PerUser,
        };
        let mut builder = InstallerBuilder::new(identity, WriteOptions::default())?;
        for (id, name, guid, bytes) in [
            (
                "First",
                "first.txt",
                "{BC2C8871-6A34-44B4-A757-19A564CEAD66}",
                b"first media payload\n".repeat(47),
            ),
            (
                "Second",
                "second.txt",
                "{19D5683B-80A8-4A7A-A5FB-C34BC77032F1}",
                b"second media payload\n".repeat(53),
            ),
            (
                "Third",
                "third.txt",
                "{39D5683B-80A8-4A7A-A5FB-C34BC77032F1}",
                b"third media payload\n".repeat(59),
            ),
        ] {
            builder.add_file(id, name, guid, Cursor::new(bytes))?;
        }
        let cabinet = |name: &str, count: u64, embedded: bool| InstallerCabinetSpec {
            name: name.into(),
            file_count: count,
            embedded,
        };
        let layout = match profile {
            "embedded" => InstallerMediaLayout::Embedded,
            "external" => InstallerMediaLayout::ExternalCabinet {
                name: "external.cab".into(),
            },
            "loose" => InstallerMediaLayout::Loose,
            "partitioned" => InstallerMediaLayout::Cabinets {
                cabinets: vec![
                    cabinet("first.cab", 1, false),
                    cabinet("later.cab", 2, false),
                ],
            },
            _ => InstallerMediaLayout::Cabinets {
                cabinets: vec![
                    cabinet("inside.cab", 1, true),
                    cabinet("outside.cab", 2, false),
                ],
            },
        };
        let mut media = Media::default();
        let mut bytes = Vec::new();
        builder.write_with_media(layout, &mut media, &mut bytes)?;
        if profile == "mixed-loose" {
            // Explicit uncompressed-file attributes override the compressed
            // package default; source-tree names differ from target paths.
            let mut editor = InstallerEditor::open(Cursor::new(&bytes), WriteOptions::default())?;
            editor.edit_summary(|summary| {
                summary.set_word_count(summary.word_count().unwrap_or(0) & !2)
            })?;
            editor.update_rows(
                "File",
                vec![("Attributes".into(), msi::Value::Int(0x4000))],
                Some(msi::Expr::col("File").ne(msi::Expr::string("Third"))),
            )?;
            editor.update_rows(
                "File",
                vec![("Attributes".into(), msi::Value::Int(0x2000))],
                Some(msi::Expr::col("File").eq(msi::Expr::string("Third"))),
            )?;
            editor.update_rows(
                "Directory",
                vec![(
                    "DefaultDir".into(),
                    msi::Value::Str("MediaFixture:SourceMedia".into()),
                )],
                Some(msi::Expr::col("Directory").eq(msi::Expr::string("INSTALLDIR"))),
            )?;
            bytes.clear();
            editor.write(&mut bytes)?;
            media.0.insert(
                "SourceMedia/third.txt".into(),
                b"third media payload\n".repeat(59),
            );
        }
        let msi_artifact = format!("{profile}/installer.msi");
        artifacts.insert(
            msi_artifact.clone(),
            artifact(&destination.join(&msi_artifact), &bytes)?,
        );
        let mut sidecars = Vec::new();
        for (name, bytes) in &media.0 {
            let path = format!("{profile}/sidecars/{name}");
            let info = artifact(&destination.join(&path), bytes)?;
            artifacts.insert(path.clone(), info.clone());
            sidecars.push(serde_json::json!({"name":name,"artifact":path,"bytes":info["bytes"],"sha256":info["sha256"]}));
        }
        let mut package = ms_package::InstallerPackage::open(Cursor::new(&bytes), 10_000)?;
        let mut files = Vec::new();
        for file in package.files()? {
            let payload = package.read_file(&file, &mut media, 1 << 20)?;
            files.push(
                serde_json::json!({"id":file.id,"path":file.path,"source_path":file.source_path,
                "size":file.size,"sequence":file.sequence,"cabinet":file.cabinet,
                "payload_hex":hex(&payload),"sha256":hash(&payload)}),
            );
        }
        profiles.insert(
            profile.into(),
            serde_json::json!({"msi":msi_artifact,"media":sidecars,"files":files}),
        );
    }
    let manifest = serde_json::json!({"schema_version":1,
        "provenance":"generated unsigned test-only data; published registry writers and readers; no installer execution or trust claim",
        "dependencies":{"ms-package":"0.2.2","caddy-msi":"0.10.2","caddy-archive-core":"0.2.1","ms-cabinet":"0.1.4"},
        "profiles":profiles,"artifacts":artifacts});
    std::fs::write(
        destination.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    Ok(())
}
