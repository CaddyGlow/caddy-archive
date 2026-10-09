//! Bounded 7z timestamp and payload/header encryption reconstruction.
//!
//! Untouched packed bytes and stored file properties are retained. Encryption
//! transforms wrap/remove AES at the compressed stream boundary, preserving
//! compression and filter properties. Selected strict subsets of a solid folder
//! are rejected rather than silently changing other entries. Callers provide a
//! stable source, empty provisional seekable output, credentials and secure
//! randomness; filesystem publication belongs to the caller.
use crate::{Limits, RandomSource, Result};
use serde::Serialize;
use std::io::{Read, Seek, Write};

/// Requested encryption state for selected payload streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EncryptionMode {
    /// Encrypt plaintext streams, or rekey existing AES streams, using new credentials.
    Encrypt,
    /// Remove AES using the old credentials.
    Decrypt,
}

/// Operations resolve against original decoded archive names; `None` targets all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum EditOperation {
    /// Store Unix seconds as checked 100 ns FILETIME while preserving other times.
    SetModified {
        /// Exact decoded name, or every entry.
        name: Option<String>,
        /// Seconds since 1970-01-01 UTC; unsupported FILETIME ranges are rejected.
        modified_unix_seconds: u64,
    },
    /// Transform complete packed folders without recompressing their payloads.
    SetEncryption {
        /// Exact decoded name, or every entry. Solid subsets are unavailable.
        name: Option<String>,
        /// Desired payload encryption state.
        mode: EncryptionMode,
    },
}

/// Credentials and archive-wide header policy. This type deliberately does not
/// implement Debug or Serialize: plans and reports must never expose secrets.
#[derive(Default)]
pub struct EditOptions<'a> {
    /// Credentials for old encrypted headers and transformed source payloads.
    pub old_password: Option<&'a [u8]>,
    /// Credentials used for newly encrypted payloads and headers.
    pub new_password: Option<&'a [u8]>,
    /// Caller-provided cryptographically secure randomness for fresh salt/IVs.
    pub randomness: Option<&'a mut dyn RandomSource>,
    /// None preserves header encryption; Some enables/disables it explicitly.
    pub encrypt_headers: Option<bool>,
}

/// Structural reuse and verified transform counts; no passwords are included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EditReport {
    /// Total preserved logical entries, including empty files/directories.
    pub retained_entries: u64,
    /// Entries whose modification time was explicitly changed.
    pub timestamp_entries: u64,
    /// Entries in payload folders subjected to an encryption transformation.
    pub encryption_entries: u64,
    /// Globally selected empty/directory entries have no payload to encrypt.
    pub entries_without_payload: u64,
    /// Unchanged packed stream bytes retained exactly.
    pub packed_bytes_copied: u64,
    /// Source packed bytes decrypted/encrypted without recompression.
    pub packed_bytes_transformed: u64,
    /// All source payload folders were decoded/checked before output.
    /// False means retained payloads were copied without decoded verification.
    pub payloads_verified: bool,
    /// Final archive-wide header encryption state.
    pub headers_encrypted: bool,
}

/// Reconstruct an archive after fully validating operations and credentials.
///
/// Transformed source folders are decoded once to verify sizes and available
/// CRCs before the first output write. They are then transformed as bounded
/// compressed streams. Retained folders need no decode/password unless their
/// encrypted headers must be opened. Outputs stay provisional until success.
/// Empty files have no payload encryption; selecting one explicitly for a
/// payload change is unsupported, while global operations report them separately.
///
/// # Errors
/// Rejects unsupported graphs, solid subsets, missing/wrong credentials,
/// ambiguous duplicate operations, unsupported preservation and resource limits.
/// I/O failure after writing begins can leave provisional output incomplete.
pub fn edit<R: Read + Seek, W: Write + Seek>(
    source: &mut R,
    output: &mut W,
    operations: &[EditOperation],
    options: EditOptions<'_>,
    limits: Limits,
) -> Result<EditReport> {
    crate::sevenz_backend::edit_archive(source, output, operations, options, limits)
}
