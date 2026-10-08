# Independently generated LZMS solid ESD

`wimlib-lzms-solid.esd` was captured by wimlib-imagex 1.14.4 (GPLv3+ reference
tool) from this repository's MIT `libmkiso/src` files on 2026-10-05:

```sh
wimlib-imagex capture crates/libmkiso/src archive-lzms-solid.esd archive-fixture --compress=LZMS --solid --threads=1
```

Archive SHA-256:
`d7a02b19478bdae73c7259cf2ef6521f81291bfa106dc8ce18a528ad0d9ee41f`.
The archive contains one image, WIM version 3584, LZMS with 131072-byte chunks,
three files and 39365 decoded bytes. `arc --image 1 list/test/extract` passed;
`diff -r` verified extraction against the input directory.

Payload SHA-256 values:

| Path | Bytes | SHA-256 |
| --- | --- | --- |
| iso9660.rs | 11857 | 8785ec3d646795652fc04968d9b596a56ff4b43541a60d951b1d83ecfa87422a |
| lib.rs | 242 | ae8e8fcc5b6949a0c8fcf2706d107ba155db42c68c3f4b6b29dab10ef9583d47 |
| writer.rs | 27266 | e8579e109ca6b3fae6a63122f1679bf513208b1962bc02bb9b0c58cc17cc87b5 |

This generated interoperability fixture does not establish Microsoft production
ESD compatibility, split-volume handling, or Windows installation correctness.

# Independently generated XPRESS WIM

`xpress-resource.wim` is an unmodified copy of
`crates/wim-format/tests/fixtures/xpress-resource.wim` from
[CaddyGlow/wim-rs](https://github.com/CaddyGlow/wim-rs/tree/54603df479e61e898ffaf8b731a3f8791acc708b),
commit `54603df479e61e898ffaf8b731a3f8791acc708b`. Its
`crates/wim-format/tests/RESOURCE-EVIDENCE.md` records generation using the
upstream C capture tool and independent C apply/cmp verification. The synthetic
payload is bytes 0 through 255 repeated 300 times, followed by `last chunk`
(76,810 bytes).

SHA-256: `e65fba091ffffbdc80f5196e6010393fcb79260b117a0d0de5c2b154aa415b96`.
The upstream project offers LGPL-2.1-or-later or GPL-3.0-or-later; its LGPL
notice is retained in `WIM-LICENSE-LGPL-2.1`. The original fixture is unchanged.
