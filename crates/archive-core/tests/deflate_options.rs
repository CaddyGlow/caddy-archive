#![cfg(feature = "streams")]
use archive_core::{
    Format, Limits, deflate_stream_with_options, inflate_stream, options::DeflateOptions,
};

#[test]
fn effort_reaches_the_backend_and_all_wrappers_round_trip() {
    let input = b"bounded configurable compression\n".repeat(4096);
    for format in [Format::Deflate, Format::Gzip, Format::Zlib] {
        let mut sizes = Vec::new();
        for level in [0, 1, 6, 9] {
            let mut packed = Vec::new();
            deflate_stream_with_options(
                &mut input.as_slice(),
                &mut packed,
                format,
                Limits::default(),
                DeflateOptions::default().with_level(level).unwrap(),
            )
            .unwrap();
            sizes.push(packed.len());
            let mut restored = Vec::new();
            inflate_stream(
                &mut packed.as_slice(),
                &mut restored,
                format,
                Limits::default(),
            )
            .unwrap();
            assert_eq!(restored, input);
        }
        assert!(
            sizes[0] > sizes[1] * 10,
            "stored and compressed levels must have different effects"
        );
    }
}

#[test]
fn unsupported_wrappers_and_workspace_fail_before_output() {
    for (format, limits) in [
        (Format::Zip, Limits::default()),
        (
            Format::Gzip,
            Limits {
                max_active_workspace_bytes: 1,
                ..Limits::default()
            },
        ),
    ] {
        let mut output = Vec::new();
        assert!(
            deflate_stream_with_options(
                &mut &b"payload"[..],
                &mut output,
                format,
                limits,
                DeflateOptions::default()
            )
            .is_err()
        );
        assert!(output.is_empty());
    }
}
