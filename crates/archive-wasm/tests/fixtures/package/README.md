# Synthetic native/browser package parity fixtures

Preserved bytes and `expected.json` were generated with the `browser_fixtures`
example maintained in MIT-licensed CaddyGlow/ms-package. The corresponding
generator source is retained upstream at
[commit 207d7d80e418098c437ecdf478e72e5ac43b6b57](https://github.com/CaddyGlow/ms-package/blob/207d7d80e418098c437ecdf478e72e5ac43b6b57/examples/browser_fixtures.rs).
The upstream MIT license is retained in `LICENSE-MIT`.
The source creates and reads the fixtures natively before recording expected
payloads, database tables/streams, and integrity counts. Browser Worker tests
compare their results with this preserved native expectation; the corrupt
fixtures must continue to fail verification. These are synthetic packages,
not production package samples or signature verification evidence.

`SHA256SUMS` pins the preserved snapshot. MSI generation can include varying
container metadata; regenerate into a new directory and review expected output
and byte changes instead of silently replacing existing regression evidence.

To regenerate using a separate checkout of the pinned generator:

```sh
git clone https://github.com/CaddyGlow/ms-package /tmp/archive-fixture-generator
git -C /tmp/archive-fixture-generator checkout 207d7d80e418098c437ecdf478e72e5ac43b6b57
cargo run --manifest-path /tmp/archive-fixture-generator/Cargo.toml --locked \
  --example browser_fixtures -- /tmp/archive-new-package-fixtures
```

CI uses this self-contained snapshot and does not require sibling checkouts.
