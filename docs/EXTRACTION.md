# Repository extraction — 2026-10-06

archive-rs owns archive-core, archive-fs, archive-cli, archive-wasm, caby,
windows-package, and the patched MSI reader. Compression, WIM, and libmkiso
remain sibling dependencies. UUP integration scripts retain their owning CLI.

The staged workspace passed all-feature host tests and Clippy, archive/CAB/UDF
fuzz regression tests, formatting and workflow linting. The WASM build and
Chromium Worker suite passed, including archive round trips, MSI/MSIX inspection,
ZIP/7z encryption and ZIP Blob cancellation. Historical fuzz campaigns and
receipts remain at their original artifact locations. Windows-native gates and
new sustained fuzz campaigns were not run for this extraction.

Installed repositories were revalidated after recovering files removed by concurrent
cleanup: host tests, Clippy, fuzz regressions and browser Worker tests passed.
All 75 unchanged fixture/artifact hashes checked matched their pre-move values.
UUP integration tests, full consumer compilation and WIM compilation passed.
Full UUP Clippy remains blocked by existing CBS dead-code warnings. Defender
workspace Clippy now passes with all targets and features: Zstandard is integrated
in ms-compress/src/zstd, resolving the earlier missing vendor dependency.

On 2026-10-07, caby was renamed cabinet and extracted to the sibling
`../cabinet` repository, including licenses, fixtures and benchmark evidence.
Archive consumers now use that standalone path dependency.

On 2026-10-07, windows-package and its patched MSI reader were extracted to
`../ms-package`, preserving crate names, public APIs, licenses and fixtures.
CLI, WASM, fuzz and CI consumers now use the sibling repository.

After the package extraction, workspace all-feature locked tests, all-target
Clippy with warnings denied, fuzz regression tests, standalone package tests
and Clippy, formatting, the WASM package build and Chromium Worker checks
passed. Worker checks included MSIX, bundles, MSI and Blob cancellation.

On 2026-10-07, the extracted windows-package crate was renamed ms-package;
CLI, WASM and fuzz consumers now import ms_package.
