# caddy-archive 0.3.0

This release adds bounded ZIP rename/delete and ZIP/7z timestamp and encryption
editing, including per-entry ZIP passwords and encrypted 7z filenames. Native
edits support dry runs and guarded Unix publication; browser Worker edits return
new bounded artifacts. Solid 7z subsets and unsupported preservation profiles
fail explicitly. The release also adds shared name selection, a limited 7-Zip
read frontend, capability inventory, ZIP64 sizing and DEFLATE effort controls.

It integrates the published ms-package 0.2.2 MSI media backend and
caddy-msi 0.10.2. Explicit CLI media inputs and browser caller-supplied media
remain supported. New regressions cover external cabinets, loose files,
partitioned and mixed media, missing or truncated media, and decoded budgets.
Native/browser authoring qualification compares every generated sidecar and
reopens the result through the archive reader; it does not expose a new
production browser authoring API.

The four workspace crates move together to 0.3.0. The registry package-core
bridge remains exactly caddy-archive-core 0.2.1 so published ms-package retains
its qualified type identity. Cargo publication normalization requires a minor
version boundary: a 0.2.2 patch would conflict with that exact registry bridge.

CI records exact producer Git revisions and registry SHA-256 values, checks
workspace/fuzz/authoring locks, and runs baseline and authoring Workers. Source
release archives retain licenses, fixtures and lockfiles, use deterministic
metadata, and verify extracted locked workspace/fuzz builds. Separate upstream
authoring sources and Cargo registry access remain required for those gates.

See [the integration plan](msi-media-refactoring-plan.md) for qualification
scope and [the dependency record](../.github/release-dependencies.json) for exact
published source provenance. Windows installation lifecycle qualification
remains producer evidence rather than a browser extraction claim.
