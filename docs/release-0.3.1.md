# caddy-archive 0.3.1

This patch repairs release qualification for the MSI media regression fixtures.
The 0.3.0 browser CI job stopped before running its Workers because the new
media fixture directory lacked the `SHA256SUMS` file required by the workflow.
The patch supplies that file and updates the fixture generator to reproduce it.
The recorded checksums cover all 15 MSI and caller-owned media artifacts.

Git attributes now preserve caller-owned sidecars as byte fixtures, including
files named `.txt`. Windows autocrlf checkouts previously converted those
payloads, changing their declared sizes and hashes and causing the loose-media
regression to fail its input budget. The patch preserves the original bytes on
Windows as well as Unix.

Windows extraction now opens directory metadata handles with
`FILE_WRITE_ATTRIBUTES`, which Windows requires when setting directory
timestamps. Previously, applying a timestamp through a read-only pinned handle
could fail with access denied. Directory pinning and reparse-point checks remain
in place.

The public archive APIs remain those of [0.3.0](release-0.3.0.md).
The four workspace crates move together to 0.3.1;
ms-package remains 0.2.2, caddy-msi remains 0.10.2, and the registry package-core
bridge remains exactly caddy-archive-core 0.2.1. The qualified upstream authoring
test source and locked authoring dependencies are unchanged.
