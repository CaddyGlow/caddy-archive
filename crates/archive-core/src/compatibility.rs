//! Seed inventory for the pinned 7-Zip compatibility roadmap.
//!
//! This is an audited starting inventory, not an exhaustive option list or a
//! compatibility parser. Native spellings and planned compatibility spellings
//! are deliberately separate. `Implemented` describes the documented native
//! profile when its required features are enabled, not full upstream parity.

use serde::Serialize;

/// Pinned upstream implementation used by this inventory.
pub const REFERENCE_COMMIT: &str = "9128b80e3a471108678e2b3ec3c985861c8d7b0f";

/// Implementation classification, separate from switch recognition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ImplementationStatus {
    /// The documented scoped behavior exists.
    Implemented,
    /// Upstream accepts the property without an effective change.
    AcceptedNoEffect,
    /// Behavior remains a future deliverable.
    Planned,
    /// Native behavior is outside the upstream container contract.
    Extension,
    /// The operation is deliberately unavailable for the selected profile.
    Unavailable,
}

/// Roadmap area used to group capability/help output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum OptionArea {
    /// Commands and aliases.
    Commands,
    /// Encoder options and resource resolution.
    Compression,
    /// Name matching and source selection.
    Selection,
    /// Archive metadata and platform restoration.
    Metadata,
    /// Runtime I/O, passwords, diagnostics and publication.
    Operation,
    /// Solid groups, volumes, SFX and advanced output behavior.
    ExtendedAuthoring,
    /// Scoped format read/write profiles.
    Formats,
}

/// One scoped capability; aliases never imply compatible semantics.
#[derive(Debug, Serialize)]
pub struct OptionInventoryRow {
    /// Stable inventory identifier.
    pub id: &'static str,
    /// Roadmap grouping.
    pub area: OptionArea,
    /// Applicable format or profile names; `all` means shared command behavior.
    pub formats: &'static [&'static str],
    /// Existing native spellings, if any.
    pub native_spellings: &'static [&'static str],
    /// Planned upstream-compatible spellings, if any.
    pub compatibility_spellings: &'static [&'static str],
    /// Native behavior classification within the described scope.
    pub native_status: ImplementationStatus,
    /// Status of the future `arc 7z` frontend, independently of native support.
    pub compatibility_status: ImplementationStatus,
    /// Value syntax; unresolved upstream details are explicitly marked.
    pub grammar: &'static str,
    /// Units/ranges or explicit lack of an audited range.
    pub units_and_range: &'static str,
    /// Native default policy, not a claim about upstream resolution.
    pub native_default: &'static str,
    /// Audited upstream default, or `None` when reference probing remains open.
    pub reference_default: Option<&'static str>,
    /// Actual native effect and/or required future behavior.
    pub effect: &'static str,
    /// Platform/browser limits; this inventory does not enable a browser API.
    pub availability: &'static str,
    /// Feature prerequisites for the native profile, not active-build assertions.
    pub required_features: &'static [&'static str],
    /// Pinned primary source URL or native source path.
    pub source: &'static str,
    /// Existing regression suites; these do not establish differential parity.
    pub existing_tests: &'static [&'static str],
    /// Acceptance requirements still to implement.
    pub planned_tests: &'static [&'static str],
}

/// Versioned, serializable inventory returned without allocation.
#[derive(Debug, Serialize)]
pub struct CompatibilityInventory {
    /// JSON inventory schema version.
    pub schema_version: u32,
    /// Reference version claimed by the roadmap; executable verification is pending.
    pub reference_version: &'static str,
    /// Source pin, independently of executable availability.
    pub reference_commit: &'static str,
    /// Coverage explicitly avoids an exhaustive parity claim.
    pub coverage: &'static str,
    /// Whether a pinned executable/hash/build configuration has been verified.
    pub reference_executable_verified: bool,
    /// Registry entries.
    pub rows: &'static [OptionInventoryRow],
}

const COMMAND_SOURCE: &str = "https://github.com/ip7z/7zip/blob/9128b80e3a471108678e2b3ec3c985861c8d7b0f/CPP/7zip/UI/Common/ArchiveCommandLine.cpp";
const METHOD_SOURCE: &str = "https://github.com/ip7z/7zip/blob/9128b80e3a471108678e2b3ec3c985861c8d7b0f/CPP/7zip/Archive/Common/HandlerOut.cpp";
const ZIP_SOURCE: &str = "https://github.com/ip7z/7zip/blob/9128b80e3a471108678e2b3ec3c985861c8d7b0f/CPP/7zip/Archive/Zip/ZipHandlerOut.cpp";
const SEVENZ_SOURCE: &str = "https://github.com/ip7z/7zip/blob/9128b80e3a471108678e2b3ec3c985861c8d7b0f/CPP/7zip/Archive/7z/7zHandlerOut.cpp";
const TAR_SOURCE: &str = "https://github.com/ip7z/7zip/blob/9128b80e3a471108678e2b3ec3c985861c8d7b0f/CPP/7zip/Archive/Tar/TarHandler.cpp";
const NATIVE_SOURCE: &str = "crates/archive-cli/src/main.rs";

macro_rules! row {
    ($id:literal, $area:ident, [$($format:literal),*], [$($native:literal),*], [$($compat:literal),*], $status:ident, $compat_status:ident, $grammar:literal, $range:literal, $default:literal, $effect:literal, $availability:literal, [$($feature:literal),*], $source:ident, [$($test:literal),*], [$($planned:literal),+]) => {
        OptionInventoryRow {
            id: $id, area: OptionArea::$area, formats: &[$($format),*],
            native_spellings: &[$($native),*], compatibility_spellings: &[$($compat),*],
            native_status: ImplementationStatus::$status,
            compatibility_status: ImplementationStatus::$compat_status,
            grammar: $grammar, units_and_range: $range, native_default: $default,
            reference_default: None, effect: $effect, availability: $availability,
            required_features: &[$($feature),*], source: $source,
            existing_tests: &[$($test),*], planned_tests: &[$($planned),+],
        }
    };
}

static ROWS: &[OptionInventoryRow] = &[
    row!(
        "editing.timestamps",
        Metadata,
        ["zip", "7z"],
        ["edit --modified-unix-seconds SECONDS", "--name NAME"],
        [],
        Implemented,
        Planned,
        "exact names or every entry; directory descendants at path boundaries",
        "ZIP: 32-bit Unix seconds plus UTC-derived DOS fallback; 7z: checked FILETIME seconds",
        "untouched timestamp fields preserved",
        "Rewrite modification time without recompressing payloads",
        "Portable core; guarded Unix publication; bounded browser Worker",
        ["zip", "sevenz"],
        NATIVE_SOURCE,
        [
            "crates/archive-core/tests/zip_edit.rs",
            "crates/archive-core/tests/sevenz_edit.rs",
            "crates/archive-wasm/tests/worker.js"
        ],
        ["Broader timestamp ranges and explicit local DOS fallback policies"]
    ),
    row!(
        "editing.encryption",
        Operation,
        ["zip", "7z"],
        [
            "edit --encrypt --new-password-file FILE",
            "edit --decrypt",
            "--password-file FILE",
            "--encrypt-headers"
        ],
        [],
        Implemented,
        Planned,
        "selected ZIP entries or complete 7z compression groups; filename encryption is 7z-only",
        "bounded compressed-stream transforms with fresh randomness; passwords never in operations/reports",
        "retained ciphertext copied; transformed source payloads verified",
        "Add/remove encryption or replace a password without recompression",
        "7z solid subsets and encrypted-header partial password changes are gated; Unix publication; browser Worker",
        ["crypto", "zip", "sevenz"],
        NATIVE_SOURCE,
        [
            "crates/archive-core/tests/zip_edit.rs",
            "crates/archive-core/tests/sevenz_edit.rs",
            "crates/archive-wasm/tests/worker.js"
        ],
        ["Solid subset rebuild and separate 7z header/payload credentials"]
    ),
    row!(
        "editing.zip-names",
        Commands,
        ["zip"],
        [
            "rename --pair OLD NEW",
            "delete --name NAME",
            "--dry-run",
            "--verify"
        ],
        [],
        Implemented,
        Planned,
        "simultaneous exact-name edits; trailing / selects descendants; optional --output creates a new artifact",
        "metadata/input/decoded sizes bounded by Limits; packed payloads copied in bounded chunks",
        "structural validation; retained payload verification only with --verify",
        "Packed-copy rename/delete preserve ciphertext, comments and known metadata; addition/replacement remain unsupported",
        "Portable core; Unix guarded publication; synchronous bounded browser artifact; no package/signature editing",
        ["zip"],
        NATIVE_SOURCE,
        [
            "crates/archive-core/tests/zip_edit.rs",
            "crates/archive-cli/tests/zip_edit.rs",
            "crates/archive-wasm/tests/worker.js"
        ],
        ["Broader lossless extras/profiles, add/update/transcode and Windows replacement"]
    ),
    row!(
        "compression.deflate-effort",
        Compression,
        ["deflate", "gzip", "zlib"],
        ["deflate --compression-level"],
        [],
        Implemented,
        Planned,
        "integer 0..=9",
        "native backend effort; 0 uses stored blocks",
        "6",
        "Validated typed effort reaches the stream encoder; JSON reports effective level; no upstream level equivalence claim",
        "Portable stream API; native deflate command; creation and other codecs retain their existing settings",
        ["streams"],
        NATIVE_SOURCE,
        ["crates/archive-core/tests/deflate_options.rs"],
        ["Resolved upstream effort settings and cross-format option builders"]
    ),
    row!(
        "update.policy",
        ExtendedAuthoring,
        ["all"],
        ["archive_core::update"],
        ["-u"],
        Implemented,
        Planned,
        "pqrxyzw state/action pairs; actions 0..3; - disables base; !NAME adds output",
        "rejects p2/q2/r1; anti-items require backend support",
        "operation-specific add/update/delete/freshen/synchronize sets",
        "Pure policy parser/classifier; repeated sets start from command defaults; does not execute add/update or normalize timestamps",
        "Portable core; caller supplies normalized time ordering",
        [],
        COMMAND_SOURCE,
        ["crates/archive-core/src/update.rs::tests"],
        ["Integrated immutable plans, timestamp precision/ranges and executor preservation"]
    ),
    row!(
        "frontend.read-subset",
        Commands,
        ["all"],
        ["list", "test", "extract"],
        ["arc 7z l", "arc 7z t", "arc 7z x"],
        Implemented,
        Implemented,
        "l|t|x archive; -tFORMAT/-oDIR; -i!PAT/-x!PAT; -spd; -ssc/-ssc-; --; native --json/-j",
        "one archive; bounded native component wildcards; UTF-8 selection patterns",
        "no overwrite; case sensitive; native scoped limits",
        "Read-only frontend lowers into native operations; wildcard/overwrite/resource policies are explicit compatibility differences",
        "Native only; package/WIM/optical selection and stdin selection rejected",
        [],
        COMMAND_SOURCE,
        [
            "crates/archive-cli/src/sevenz_args.rs::tests",
            "crates/archive-cli/tests/sevenz_frontend.rs",
            "crates/archive-cli/tests/selection.rs"
        ],
        ["Pinned differential command/selector outcomes and broader upstream grammar"]
    ),
    row!(
        "command.add",
        Commands,
        ["all"],
        ["create", "a"],
        ["a", "add"],
        Implemented,
        Planned,
        "command archive [arguments]",
        "not applicable",
        "create-only/no-clobber",
        "Native creates a new archive with no overwrite; upstream add also updates existing archives",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        ["crates/archive-cli/tests/command_aliases.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "command.update",
        Commands,
        ["all"],
        [],
        ["u", "update"],
        Planned,
        Planned,
        "command archive [arguments]",
        "not applicable",
        "operation-specific",
        "Update decisions and transactional reconstruction remain unimplemented",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "command.delete",
        Commands,
        ["all"],
        [],
        ["d", "delete"],
        Planned,
        Planned,
        "command archive [arguments]",
        "not applicable",
        "operation-specific",
        "Selected removal remains unimplemented",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "command.rename",
        Commands,
        ["all"],
        [],
        ["rn", "rename"],
        Planned,
        Planned,
        "command archive [arguments]",
        "not applicable",
        "operation-specific",
        "Old/new entry pairs and directory boundary semantics remain unimplemented",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "command.list",
        Commands,
        ["all"],
        ["list", "l"],
        ["l", "list"],
        Implemented,
        Planned,
        "command archive [arguments]",
        "not applicable",
        "operation-specific",
        "List scoped native entries",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        ["crates/archive-cli/tests/command_aliases.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "command.test",
        Commands,
        ["all"],
        ["test", "t"],
        ["t", "test"],
        Implemented,
        Planned,
        "command archive [arguments]",
        "not applicable",
        "operation-specific",
        "Verify sizes and available integrity checks",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        ["crates/archive-cli/tests/command_aliases.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "command.extract",
        Commands,
        ["all"],
        ["extract", "x"],
        ["x", "extract"],
        Implemented,
        Planned,
        "command archive [arguments]",
        "not applicable",
        "operation-specific",
        "Safe native extraction with stored paths",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        ["crates/archive-cli/tests/command_aliases.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "command.extract-flat",
        Commands,
        ["all"],
        [],
        ["e"],
        Planned,
        Planned,
        "command archive [arguments]",
        "not applicable",
        "operation-specific",
        "Path-flattening collision policy remains unimplemented",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "command.hash",
        Commands,
        ["all"],
        [],
        ["h"],
        Planned,
        Planned,
        "command archive [arguments]",
        "not applicable",
        "operation-specific",
        "Standalone upstream hash command remains unimplemented",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "compression.format",
        Compression,
        ["all"],
        ["--format", "-f"],
        ["-t"],
        Implemented,
        Planned,
        "format alias",
        "feature-gated known aliases",
        "signature/suffix inference with explicit override",
        "Native -t selects threads; upstream -t selects container",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        ["crates/archive-cli/tests/format_detection.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "compression.method",
        Compression,
        ["zip", "7z", "cab"],
        ["--compression", "-c"],
        ["-m"],
        Implemented,
        Planned,
        "native codec name; upstream property grammar pending complete audit",
        "native method set varies by format",
        "ZIP DEFLATE; 7z LZMA2; CAB MSZIP",
        "Native -m maps MSI media; compatibility method parsing is separate",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        ["crates/archive-cli/tests/additional_compression.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "compression.level",
        Compression,
        ["7z", "zip", "xz", "lzma"],
        [],
        ["-mx"],
        Planned,
        Planned,
        "-mx<N>",
        "per-codec ranges pending audit",
        "fixed backend presets",
        "Expose effective codec tuning; native writers currently choose fixed levels",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        METHOD_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "compression.dictionary",
        Compression,
        ["7z", "zip", "xz", "lzma"],
        [],
        ["-m"],
        Planned,
        Planned,
        "dictionary units/property combinations require per-codec audit",
        "per-codec ranges pending audit",
        "fixed backend presets",
        "Forward validated dictionary controls into supported backends",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        METHOD_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "compression.lzma-properties",
        Compression,
        ["7z", "zip", "xz", "lzma"],
        [],
        ["-m"],
        Planned,
        Planned,
        "lc/lp/pb/fb/mf/mc property families; values require audit",
        "per-codec ranges pending audit",
        "fixed backend presets",
        "Validate combinations and prove backend effects",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        METHOD_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "compression.threads",
        Compression,
        ["all"],
        ["--threads", "-t"],
        ["-mmt"],
        Implemented,
        Planned,
        "upstream -mmt property grammar requires audit",
        "per-codec ranges pending audit",
        "OS worker ceiling",
        "Native worker ceiling applies to eligible independent folders",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        METHOD_SOURCE,
        ["crates/archive-core/tests/sevenz_parallel_interop.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "compression.memory",
        Compression,
        ["all"],
        ["--memuse", "--mmemuse", "-mmemuse=VALUE"],
        ["-mmemuse"],
        Implemented,
        Planned,
        "auto | integer[b|k|m|g|t] | N% | pN",
        "binary units; percentage of detected total RAM",
        "80% compression; 17/32 decompression; architecture/fallback limits apply",
        "Resolve codec workspace; does not bound process RSS",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        METHOD_SOURCE,
        [
            "crates/archive-core/tests/memory_policy.rs",
            "crates/archive-cli/tests/memory.rs"
        ],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "selection.include",
        Selection,
        ["all"],
        ["--include"],
        ["-i"],
        Implemented,
        Planned,
        "UTF-8 native CLI patterns; byte-based core; component-local * and ?",
        "default bounded selector limits; advanced grammar rejected",
        "all names included; no exclusions; case sensitive",
        "Native inclusion selector; excludes win; directory descendants match only slash boundaries",
        "Indexed archive adapters only; stdin/package/WIM/optical selection rejected",
        [],
        COMMAND_SOURCE,
        [
            "crates/archive-core/src/selection.rs::tests",
            "crates/archive-cli/tests/selection.rs"
        ],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "selection.exclude",
        Selection,
        ["all"],
        ["--exclude"],
        ["-x"],
        Implemented,
        Planned,
        "UTF-8 native CLI patterns; byte-based core; component-local * and ?",
        "default bounded selector limits; advanced grammar rejected",
        "all names included; no exclusions; case sensitive",
        "Native exclusion selector with bounded matching work",
        "Indexed archive adapters only; stdin/package/WIM/optical selection rejected",
        [],
        COMMAND_SOURCE,
        [
            "crates/archive-core/src/selection.rs::tests",
            "crates/archive-cli/tests/selection.rs"
        ],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "selection.recursion",
        Selection,
        ["all"],
        [],
        ["-r"],
        Planned,
        Planned,
        "recursion mode",
        "pending pinned boundary/encoding audit",
        "no shared native selection engine",
        "Explicit filesystem recursion rules",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "selection.wildcards",
        Selection,
        ["all"],
        ["--literal-names"],
        ["-spd"],
        Implemented,
        Planned,
        "explicit literal mode; wildcard mode rejects **, brackets, braces and escapes",
        "default bounded selector limits; advanced grammar rejected",
        "all names included; no exclusions; case sensitive",
        "Native literal patterns preserve metacharacters and leading dash/@ bytes",
        "Indexed archive adapters only; stdin/package/WIM/optical selection rejected",
        [],
        COMMAND_SOURCE,
        [
            "crates/archive-core/src/selection.rs::tests",
            "crates/archive-cli/tests/selection.rs"
        ],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "selection.case",
        Selection,
        ["all"],
        ["--ignore-ascii-case"],
        ["-ssc"],
        Implemented,
        Planned,
        "ASCII insensitive opt-in; non-ASCII bytes remain exact",
        "default bounded selector limits; advanced grammar rejected",
        "all names included; no exclusions; case sensitive",
        "Native default compares bytes exactly; no Unicode normalization/folding",
        "Indexed archive adapters only; stdin/package/WIM/optical selection rejected",
        [],
        COMMAND_SOURCE,
        [
            "crates/archive-core/src/selection.rs::tests",
            "crates/archive-cli/tests/selection.rs"
        ],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "selection.listfiles",
        Selection,
        ["all"],
        [],
        ["-scs"],
        Planned,
        Planned,
        "listfile encoding selector and @filename",
        "pending pinned boundary/encoding audit",
        "no shared native selection engine",
        "BOM/encoding and leading @ handling",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "selection.termination",
        Selection,
        ["all"],
        [],
        ["--"],
        Planned,
        Planned,
        "argument terminator",
        "pending pinned boundary/encoding audit",
        "no shared native selection engine",
        "Names beginning with dash must remain representable",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "metadata.timestamps",
        Metadata,
        ["all"],
        [],
        ["-m"],
        Implemented,
        Planned,
        "format/platform-specific fields",
        "timestamp precision and platform representability vary",
        "supported native fields only",
        "Stored modification/access/creation time, precision and timezone require per-format policy",
        "Native fields depend on format/OS; Windows runtime behavior is a separate gate",
        [],
        COMMAND_SOURCE,
        [
            "crates/archive-core/tests/entry_metadata.rs",
            "crates/archive-cli/tests/metadata.rs"
        ],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "metadata.names",
        Metadata,
        ["all"],
        [],
        ["-m"],
        Planned,
        Planned,
        "format/platform-specific fields",
        "timestamp precision and platform representability vary",
        "supported native fields only",
        "Preserve raw/stored encoding separately from source and destination paths",
        "Native fields depend on format/OS; Windows runtime behavior is a separate gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "metadata.permissions",
        Metadata,
        ["all"],
        [],
        ["-snoi", "-snon"],
        Implemented,
        Planned,
        "format/platform-specific fields",
        "timestamp precision and platform representability vary",
        "supported native fields only",
        "Native ordinary permission/time restoration; upstream ownership profiles require audit",
        "Native fields depend on format/OS; Windows runtime behavior is a separate gate",
        [],
        COMMAND_SOURCE,
        [
            "crates/archive-core/tests/entry_metadata.rs",
            "crates/archive-cli/tests/metadata.rs"
        ],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "metadata.links",
        Metadata,
        ["all"],
        [],
        ["-snl", "-snh"],
        Unavailable,
        Planned,
        "format/platform-specific fields",
        "timestamp precision and platform representability vary",
        "supported native fields only",
        "Links currently rejected during native extraction; authoring gates required",
        "Native fields depend on format/OS; Windows runtime behavior is a separate gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "metadata.security",
        Metadata,
        ["all"],
        [],
        ["-sni", "-sns"],
        Unavailable,
        Planned,
        "format/platform-specific fields",
        "timestamp precision and platform representability vary",
        "supported native fields only",
        "ACLs, alternate streams and platform security metadata remain unsupported",
        "Native fields depend on format/OS; Windows runtime behavior is a separate gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "operation.password",
        Operation,
        ["zip", "7z"],
        ["--password-file", "-p"],
        ["-p"],
        Implemented,
        Planned,
        "native password file; upstream inline/prompt grammar separately planned",
        "password bytes; never echo secrets",
        "ZIP native AES-256; upstream defaults unverified",
        "Keep native file input; do not inherit native encryption defaults in compatibility frontend",
        "Native; browser exposure requires its own API and runtime gate",
        ["crypto"],
        COMMAND_SOURCE,
        ["crates/archive-cli/tests/encryption.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "operation.output",
        Operation,
        ["all"],
        ["--output", "-o", "-d"],
        ["-o"],
        Implemented,
        Planned,
        "output path",
        "filesystem policy applies",
        "current directory for native extraction",
        "Native extraction rejects existing files and traversal; compatibility overwrite modes remain future work",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        ["crates/archive-cli/tests/command_aliases.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "operation.overwrite",
        Operation,
        ["all"],
        [],
        ["-ao"],
        Planned,
        Planned,
        "overwrite mode",
        "pending pinned audit",
        "unavailable",
        "Upstream overwrite modes need separate publication policy",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Backend support plus pinned grammar and failure/publication tests"]
    ),
    row!(
        "operation.working-directory",
        Operation,
        ["all"],
        [],
        ["-w"],
        Planned,
        Planned,
        "scratch/work directory",
        "pending pinned audit",
        "unavailable",
        "Caller-owned bounded scratch policy",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Backend support plus pinned grammar and failure/publication tests"]
    ),
    row!(
        "authoring.update-actions",
        ExtendedAuthoring,
        ["all"],
        [],
        ["-u"],
        Planned,
        Planned,
        "state/action set with optional output suffix",
        "pending pinned audit",
        "unavailable",
        "Implement seven states, invalid actions, repeated sets and multiple outputs",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Backend support plus pinned grammar and failure/publication tests"]
    ),
    row!(
        "authoring.volumes",
        ExtendedAuthoring,
        ["all"],
        [],
        ["-v"],
        Planned,
        Planned,
        "volume sizes",
        "pending pinned audit",
        "unavailable",
        "Require working split-volume backends",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Backend support plus pinned grammar and failure/publication tests"]
    ),
    row!(
        "authoring.sfx",
        ExtendedAuthoring,
        ["all"],
        [],
        ["-sfx"],
        Planned,
        Planned,
        "module/path",
        "pending pinned audit",
        "unavailable",
        "Require validated executable prefix/backend support",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        [],
        ["Backend support plus pinned grammar and failure/publication tests"]
    ),
    row!(
        "operation.streams",
        Operation,
        ["tar", "gzip", "zlib", "deflate"],
        ["-"],
        ["-si", "-so"],
        Implemented,
        Planned,
        "native dash path; upstream stream switches",
        "forward-only compatible profiles",
        "files; explicit format for stdout",
        "Binary output cannot roll back; indexed formats require seekable input",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        ["crates/archive-cli/tests/streams.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "operation.reporting",
        Operation,
        ["all"],
        ["--json", "--progress", "-P"],
        ["-bb", "-bso", "-bse", "-bsp"],
        Implemented,
        Planned,
        "native progress auto|always|never; upstream routing grammar separate",
        "native coalesced rendering 10 Hz",
        "auto for interactive non-JSON operations",
        "Native exit code/schema contract remains separate from upstream",
        "Native; browser exposure requires its own API and runtime gate",
        ["progress"],
        COMMAND_SOURCE,
        ["crates/archive-cli/tests/cli.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "operation.exit-codes",
        Operation,
        ["all"],
        [],
        [],
        Implemented,
        Planned,
        "numeric operation result",
        "0,1,2,3,4,5,6,130 native codes",
        "0 success",
        "Upstream exit outcomes require differential mapping",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        COMMAND_SOURCE,
        ["crates/archive-cli/tests/cli.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "authoring.solid",
        ExtendedAuthoring,
        ["7z"],
        [],
        ["-ms"],
        Planned,
        Planned,
        "format property combinations",
        "pending handler audit",
        "native independent entries; supported encryption explicit",
        "Independent-entry writer exists; multi-entry solid grouping remains future work",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        SEVENZ_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "authoring.filters",
        ExtendedAuthoring,
        ["7z"],
        [],
        ["-m"],
        Planned,
        Planned,
        "format property combinations",
        "pending handler audit",
        "native independent entries; supported encryption explicit",
        "Writer filter/coder graphs require exact backend support",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        SEVENZ_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "authoring.headers",
        ExtendedAuthoring,
        ["7z"],
        ["--encrypt-headers", "-H"],
        ["-mhc", "-mhe"],
        Implemented,
        Planned,
        "format property combinations",
        "pending handler audit",
        "native independent entries; supported encryption explicit",
        "Native encrypted headers supported; other header tuning requires audit",
        "Native; browser exposure requires its own API and runtime gate",
        ["sevenz", "crypto"],
        SEVENZ_SOURCE,
        ["crates/archive-core/tests/crypto.rs"],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "authoring.comments",
        ExtendedAuthoring,
        ["zip"],
        [],
        ["-m"],
        Planned,
        Planned,
        "format property combinations",
        "pending handler audit",
        "native independent entries; supported encryption explicit",
        "Archive comments and lossless preservation require format-specific contracts",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        ZIP_SOURCE,
        [],
        ["Pinned acceptance, effective defaults and error outcomes"]
    ),
    row!(
        "format.zip",
        Formats,
        ["zip"],
        ["--format"],
        ["-t"],
        Implemented,
        Planned,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "Copy/DEFLATE with automatic classic ZIP/ZIP64; AES/ZipCrypto with crypto",
        "Native; browser exposure requires its own API and runtime gate",
        ["zip"],
        ZIP_SOURCE,
        ["crates/archive-core/tests/roundtrip.rs"],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
    row!(
        "format.7z",
        Formats,
        ["7z"],
        ["--format"],
        ["-t"],
        Implemented,
        Planned,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "Independent-entry Copy/DEFLATE/LZMA/LZMA2; optional BZip2/Brotli",
        "Native; browser exposure requires its own API and runtime gate",
        ["sevenz"],
        SEVENZ_SOURCE,
        ["crates/archive-core/tests/roundtrip.rs"],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
    row!(
        "format.tar",
        Formats,
        ["tar"],
        ["--format"],
        ["-t"],
        Implemented,
        Planned,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "USTAR/PAX creation; full raw-extension preservation remains future work",
        "Native; browser exposure requires its own API and runtime gate",
        ["tar"],
        TAR_SOURCE,
        ["crates/archive-core/tests/roundtrip.rs"],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
    row!(
        "format.gzip",
        Formats,
        ["gzip", "tar.gz"],
        ["--format"],
        ["-t"],
        Implemented,
        Planned,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "Fixed DEFLATE; stream versus TAR wrapper remain distinct",
        "Native; browser exposure requires its own API and runtime gate",
        ["gzip"],
        METHOD_SOURCE,
        ["crates/archive-core/tests/roundtrip.rs"],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
    row!(
        "format.bzip2",
        Formats,
        ["bzip2", "tar.bz2"],
        ["--format"],
        ["-t"],
        Implemented,
        Planned,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "Fixed writer level 9; optional backend",
        "Native; browser exposure requires its own API and runtime gate",
        ["bzip2"],
        METHOD_SOURCE,
        [],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
    row!(
        "format.xz",
        Formats,
        ["xz", "tar.xz"],
        ["--format"],
        ["-t"],
        Implemented,
        Planned,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "Fixed LZMA2 preset 6 and CRC64; richer tuning remains future work",
        "Native; browser exposure requires its own API and runtime gate",
        ["xz"],
        METHOD_SOURCE,
        [],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
    row!(
        "format.lzma",
        Formats,
        ["lzma"],
        ["--format"],
        ["-t"],
        Extension,
        Planned,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "Standalone writer is an extension, not upstream container-writing parity",
        "Native; browser exposure requires its own API and runtime gate",
        ["streams"],
        NATIVE_SOURCE,
        [],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
    row!(
        "format.cab",
        Formats,
        ["cab"],
        ["--format"],
        ["-t"],
        Extension,
        Planned,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "Copy/MSZIP/LZX/Quantum authoring is an extension",
        "Native; browser exposure requires its own API and runtime gate",
        ["cab"],
        NATIVE_SOURCE,
        [],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
    row!(
        "format.brotli",
        Formats,
        ["brotli", "tar.br"],
        ["--format"],
        [],
        Extension,
        Unavailable,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "Quality 5/window 22 authoring is an extension",
        "Native; browser exposure requires its own API and runtime gate",
        ["brotli"],
        NATIVE_SOURCE,
        [],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
    row!(
        "format.raw",
        Formats,
        ["zlib", "deflate", "windows-codecs"],
        ["--format"],
        [],
        Extension,
        Unavailable,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "Single payload/raw blocks; no artificial multi-entry editing",
        "Native; browser exposure requires its own API and runtime gate",
        ["streams"],
        NATIVE_SOURCE,
        [],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
    row!(
        "format.wim",
        Formats,
        ["wim", "esd"],
        ["--format"],
        ["-t"],
        Implemented,
        Planned,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "Selected-image reading; WIM authoring/update requires backend publication",
        "Native; browser exposure requires its own API and runtime gate",
        ["wim"],
        NATIVE_SOURCE,
        [],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
    row!(
        "format.optical",
        Formats,
        ["iso", "udf"],
        ["--format"],
        ["-t"],
        Implemented,
        Planned,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "Read-only selected profiles; no editing contract",
        "Native; browser exposure requires its own API and runtime gate",
        ["iso"],
        NATIVE_SOURCE,
        [],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
    row!(
        "format.packages",
        Formats,
        ["msi", "appx", "msix", "bundles"],
        ["--format"],
        ["-t"],
        Implemented,
        Planned,
        "native aliases; upstream format/property set requires probes",
        "profile-specific limits",
        "see scoped effect",
        "Read-only package interpretation; no writer/signer",
        "Native; browser exposure requires its own API and runtime gate",
        [],
        NATIVE_SOURCE,
        [],
        ["Per-format pinned encoder availability, defaults and preservation checks"]
    ),
];

/// Return the seed inventory. No parser, encoder or edit capability is enabled.
///
/// Defaults left as `None` must be resolved from the pinned reference before
/// implementing compatibility parsing; native defaults are separate evidence.
pub fn inventory() -> CompatibilityInventory {
    CompatibilityInventory {
        schema_version: 1,
        reference_version: "7-Zip 26.04",
        reference_commit: REFERENCE_COMMIT,
        coverage: "seed roadmap inventory; not exhaustive; pinned executable probes pending",
        reference_executable_verified: false,
        rows: ROWS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn identifiers_and_acceptance_requirements_are_complete() {
        let mut ids = HashSet::new();
        for row in inventory().rows {
            assert!(ids.insert(row.id), "duplicate inventory id: {}", row.id);
            assert!(!row.formats.is_empty(), "{} has no scope", row.id);
            assert!(!row.grammar.is_empty(), "{} has no grammar", row.id);
            assert!(
                !row.planned_tests.is_empty(),
                "{} has no acceptance gate",
                row.id
            );
            assert!(!row.source.is_empty(), "{} has no source", row.id);
        }
    }

    #[test]
    fn frontend_support_is_limited_to_explicit_read_subset() {
        for row in inventory().rows {
            if row.id == "frontend.read-subset" {
                assert_eq!(row.compatibility_status, ImplementationStatus::Implemented);
            } else {
                assert!(matches!(
                    row.compatibility_status,
                    ImplementationStatus::Planned | ImplementationStatus::Unavailable
                ));
            }
        }
        assert!(!inventory().reference_executable_verified);
    }

    #[test]
    fn inventory_covers_every_roadmap_area_and_format_family() {
        for area in [
            OptionArea::Commands,
            OptionArea::Compression,
            OptionArea::Selection,
            OptionArea::Metadata,
            OptionArea::Operation,
            OptionArea::ExtendedAuthoring,
            OptionArea::Formats,
        ] {
            assert!(inventory().rows.iter().any(|row| row.area == area));
        }
        for format in [
            "zip",
            "7z",
            "tar",
            "gzip",
            "bzip2",
            "xz",
            "lzma",
            "cab",
            "brotli",
            "zlib",
            "deflate",
            "windows-codecs",
            "wim",
            "esd",
            "iso",
            "udf",
            "msi",
            "appx",
            "msix",
            "bundles",
        ] {
            assert!(
                inventory()
                    .rows
                    .iter()
                    .any(|row| row.area == OptionArea::Formats && row.formats.contains(&format)),
                "missing format: {format}"
            );
        }
    }

    #[test]
    fn native_short_option_collisions_are_explicit() {
        for (id, native, compatibility) in [
            ("compression.format", "-f", "-t"),
            ("compression.method", "-c", "-m"),
            ("operation.password", "-p", "-p"),
        ] {
            let row = inventory().rows.iter().find(|row| row.id == id).unwrap();
            assert!(row.native_spellings.contains(&native));
            assert!(row.compatibility_spellings.contains(&compatibility));
            assert_eq!(row.compatibility_status, ImplementationStatus::Planned);
        }
    }
}
