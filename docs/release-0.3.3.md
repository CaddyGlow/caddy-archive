# caddy-archive 0.3.3

This release carries the [0.3.2](release-0.3.2.md) runtime and corrects the
platform qualification of a CLI test that creates an invalid UTF-8 filename.
macOS APFS rejects that fixture before the CLI can run. The filesystem test
requires a platform supporting those filenames; parser coverage remains.

All four workspace crates move together to 0.3.3. Runtime APIs and qualified
registry backend dependencies are unchanged from 0.3.2.
