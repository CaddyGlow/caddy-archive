//! ms-package's published API uses the registry archive-core type identities.
//! Keep that boundary explicit instead of patching published dependencies.
pub(crate) fn limits(value: archive_core::Limits) -> package_core::Limits {
    package_core::Limits {
        max_entries: value.max_entries,
        max_metadata_bytes: value.max_metadata_bytes,
        max_entry_bytes: value.max_entry_bytes,
        max_total_bytes: value.max_total_bytes,
        max_dictionary_bytes: value.max_dictionary_bytes,
        max_input_bytes: value.max_input_bytes,
        max_active_workspace_bytes: value.max_active_workspace_bytes,
        max_pending_output_bytes: value.max_pending_output_bytes,
        max_workers: value.max_workers,
        max_password_iterations: value.max_password_iterations,
        max_nesting_depth: value.max_nesting_depth,
    }
}
pub(crate) fn metadata(value: package_core::EntryMetadata) -> archive_core::EntryMetadata {
    archive_core::EntryMetadata {
        modified: value.modified.map(|value| match value {
            package_core::StoredTimestamp::UnixSeconds(seconds) => {
                archive_core::StoredTimestamp::UnixSeconds(seconds)
            }
            package_core::StoredTimestamp::DosLocal {
                year,
                month,
                day,
                hour,
                minute,
                second,
            } => archive_core::StoredTimestamp::DosLocal {
                year,
                month,
                day,
                hour,
                minute,
                second,
            },
        }),
        unix_mode: value.unix_mode,
        user_id: value.user_id,
        group_id: value.group_id,
        link_target: value.link_target,
        format: value.format.map(|value| match value {
            package_core::EntryFormatMetadata::Zip {
                crc32,
                compression_method,
                aes_version,
                aes_strength,
            } => archive_core::EntryFormatMetadata::Zip {
                crc32,
                compression_method,
                aes_version,
                aes_strength,
            },
            package_core::EntryFormatMetadata::Tar { stored_type } => {
                archive_core::EntryFormatMetadata::Tar { stored_type }
            }
        }),
    }
}
