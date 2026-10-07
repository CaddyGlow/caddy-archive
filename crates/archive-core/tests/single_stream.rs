#![cfg(feature = "streams")]
use archive_core::{
    Error, Limits,
    single_stream::{self, Codec, Options},
};
#[test]
fn raw_size_and_workspace_limits_reject_before_output() {
    for (codec, options, limits) in [
        (Codec::Xpress, Options::default(), Limits::default()),
        (
            Codec::Xpress,
            Options {
                output_size: Some(65537),
                ..Options::default()
            },
            Limits::default(),
        ),
        (
            Codec::Lzma2,
            Options {
                dictionary_bytes: 1,
                ..Options::default()
            },
            Limits::default(),
        ),
        (
            Codec::Lznt1,
            Options {
                output_size: Some(1 << 20),
                ..Options::default()
            },
            Limits {
                max_active_workspace_bytes: 64,
                ..Limits::default()
            },
        ),
    ] {
        let mut output = Vec::new();
        assert!(
            single_stream::decompress(&mut &b"invalid"[..], &mut output, codec, options, limits)
                .is_err()
        );
        assert!(output.is_empty());
    }
}
#[test]
fn single_file_compression_honors_entry_and_input_limits() {
    for codec in [Codec::Gzip, Codec::Lzma, Codec::XpressPlain] {
        let limits = Limits {
            max_entry_bytes: 3,
            ..Limits::default()
        };
        let result = single_stream::compress(
            &mut &b"four"[..],
            &mut Vec::new(),
            codec,
            Options::default(),
            limits,
        );
        assert!(matches!(result, Err(Error::ResourceLimit(_))));
    }
}
#[test]
fn xpress_raw_blocks_can_encode_short_inputs() {
    for input in [&b"x"[..], &b"abc"[..], &b"xxxxxxxxxxxxxxxxxxxxxxxx"[..]] {
        let mut packed = Vec::new();
        single_stream::compress(
            &mut &input[..],
            &mut packed,
            Codec::Xpress,
            Options::default(),
            Limits::default(),
        )
        .unwrap();
        let mut output = Vec::new();
        single_stream::decompress(
            &mut &packed[..],
            &mut output,
            Codec::Xpress,
            Options {
                output_size: Some(input.len() as u64),
                ..Options::default()
            },
            Limits::default(),
        )
        .unwrap();
        assert_eq!(output, input);
    }
}
