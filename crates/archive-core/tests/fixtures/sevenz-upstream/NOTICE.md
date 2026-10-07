# Upstream Compatibility Fixtures

These resources are copied unchanged from `sevenz-rust2` 0.23.0,
https://github.com/hasenbanck/sevenz-rust2, `tests/resources/`.
The accompanying Apache-2.0 license is retained in `LICENSE-APACHE-2.0`.

`../../sevenz_upstream.rs` adapts assertions from upstream
`tests/decompression_tests.rs` and `tests/decompress_encrypted_tests.rs`
to the archive-core API. `../../sevenz_upstream_security.rs` adapts malformed
header, coder graph, AES short-read and Delta regression cases from upstream
`tests/security_tests.rs`. Neither test target depends on the upstream runtime.
The malformed coder-stream fixture preserves the regression for upstream
issue 127. The executable files are reference byte payloads and are never run.

The corpus also retains BCJ2, PPMd, BZip2, and Zstandard-wrapped codec fixtures
for future coverage. Retaining a fixture does not claim that its codec is
supported or that every upstream test has been ported.
