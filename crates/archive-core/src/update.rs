//! Portable update decisions and `-u` action-set parsing.
//!
//! This module plans policy only: it never opens files or publishes archives.
//! Timestamp normalization, name selection and preservation checks belong to the
//! caller. See the pinned 7-Zip `UpdateAction.cpp`, `UpdatePair.cpp` and
//! `ArchiveCommandLine.cpp` sources in the repository's options/update plan.

use crate::{Error, Result};
use serde::Serialize;
use std::cmp::Ordering;

/// The seven archive/source pairing states, in upstream `pqrxyzw` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateState {
    /// An archive item outside the selection (`p`).
    NotSelected,
    /// A selected archive item absent from the source (`q`).
    OnlyInArchive,
    /// A source item absent from the archive (`r`).
    OnlyOnDisk,
    /// The archive modification time is newer (`x`).
    ArchiveNewer,
    /// The source modification time is newer (`y`).
    DiskNewer,
    /// Equal comparison time (or unknown archive time) and equal known size (`z`).
    Same,
    /// Equal/unknown time and differing/unknown size (`w`).
    Unknown,
}

impl UpdateState {
    /// Upstream state letter used by an update action switch.
    pub const fn letter(self) -> char {
        match self {
            Self::NotSelected => 'p',
            Self::OnlyInArchive => 'q',
            Self::OnlyOnDisk => 'r',
            Self::ArchiveNewer => 'x',
            Self::DiskNewer => 'y',
            Self::Same => 'z',
            Self::Unknown => 'w',
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// Action for a paired archive/source item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateAction {
    /// Omit this item (`0`).
    Omit,
    /// Retain archive payload and properties (`1`).
    Retain,
    /// Read source payload and properties (`2`).
    UseDisk,
    /// Emit an anti-item deletion marker (`3`), requiring backend support.
    AntiItem,
}

/// Default upstream update policies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// Add all selected source items, including equal or older items.
    Add,
    /// Replace newer or indeterminate items and add missing items.
    Update,
    /// Update existing items without adding new source items.
    Freshen,
    /// Update and remove selected archive items missing from the source.
    Synchronize,
    /// Remove selected archive items.
    Delete,
}

/// Immutable actions indexed by [`UpdateState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct UpdateActionSet {
    actions: [UpdateAction; 7],
}

impl UpdateActionSet {
    /// Resolve the upstream defaults for a policy.
    pub const fn for_mode(mode: UpdateMode) -> Self {
        use UpdateAction::{Omit as O, Retain as R, UseDisk as D};
        Self {
            actions: match mode {
                UpdateMode::Add => [R, R, D, D, D, D, D],
                UpdateMode::Update => [R, R, D, R, D, R, D],
                UpdateMode::Freshen => [R, R, O, R, D, R, D],
                UpdateMode::Synchronize => [R, O, D, R, D, R, D],
                UpdateMode::Delete => [R, O, O, O, O, O, O],
            },
        }
    }

    /// The action for one classified item.
    pub const fn action(self, state: UpdateState) -> UpdateAction {
        self.actions[state.index()]
    }

    /// Reject anti-item policies before execution by a backend lacking them.
    pub fn validate_anti_items(self, supported: bool) -> Result<()> {
        if !supported && self.actions.contains(&UpdateAction::AntiItem) {
            return Err(Error::Unsupported("update anti-items".into()));
        }
        Ok(())
    }
}

/// A single additional output selected by an update switch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UpdateOutput {
    /// Caller-interpreted output name. This is not a validated filesystem path.
    pub name: String,
    /// Independent actions for this output.
    pub actions: UpdateActionSet,
}

/// Resolved repeated `-u` switches, without performing any publication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UpdateSwitches {
    /// Base-archive actions, or `None` after `-u-`.
    pub base: Option<UpdateActionSet>,
    /// Additional output action sets in argument order.
    pub additional: Vec<UpdateOutput>,
}

impl UpdateSwitches {
    /// Parse switch suffixes (without `-u`) using command defaults for each set.
    ///
    /// Repeated unsuffixed sets replace the base set. `-` disables the base
    /// output permanently; later unsuffixed sets do not re-enable it. `!name`
    /// adds an independent output. State letters are ASCII case insensitive.
    ///
    /// # Errors
    /// Rejects malformed pairs, actions outside 0..=3, forbidden `p2/q2/r1`,
    /// invalid suffixes and empty additional output names.
    pub fn parse(mode: UpdateMode, switches: &[&str]) -> Result<Self> {
        let defaults = UpdateActionSet::for_mode(mode);
        let mut result = Self {
            base: Some(defaults),
            additional: Vec::new(),
        };
        for suffix in switches {
            if *suffix == "-" {
                result.base = None;
                continue;
            }
            let mut actions = defaults;
            let mut rest = *suffix;
            while !rest.is_empty() && !rest.starts_with('!') {
                let bytes = rest.as_bytes();
                let state = match bytes[0].to_ascii_lowercase() {
                    b'p' => UpdateState::NotSelected,
                    b'q' => UpdateState::OnlyInArchive,
                    b'r' => UpdateState::OnlyOnDisk,
                    b'x' => UpdateState::ArchiveNewer,
                    b'y' => UpdateState::DiskNewer,
                    b'z' => UpdateState::Same,
                    b'w' => UpdateState::Unknown,
                    _ => return Err(invalid_switch()),
                };
                let action = match bytes.get(1) {
                    Some(b'0') => UpdateAction::Omit,
                    Some(b'1') => UpdateAction::Retain,
                    Some(b'2') => UpdateAction::UseDisk,
                    Some(b'3') => UpdateAction::AntiItem,
                    _ => return Err(invalid_switch()),
                };
                if matches!(
                    (state, action),
                    (
                        UpdateState::NotSelected | UpdateState::OnlyInArchive,
                        UpdateAction::UseDisk
                    ) | (UpdateState::OnlyOnDisk, UpdateAction::Retain)
                ) {
                    return Err(invalid_switch());
                }
                actions.actions[state.index()] = action;
                rest = &rest[2..];
            }
            if let Some(name) = rest.strip_prefix('!') {
                if name.is_empty() || name.contains('\0') {
                    return Err(invalid_switch());
                }
                result.additional.push(UpdateOutput {
                    name: name.to_owned(),
                    actions,
                });
            } else if result.base.is_some() {
                result.base = Some(actions);
            }
        }
        Ok(result)
    }
}

fn invalid_switch() -> Error {
    // Switches can include user-provided names. Keep them out of diagnostics.
    Error::Malformed("incorrect update switch command".into())
}

/// Metadata required to classify an existing item; unknown size stays unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpdateItem {
    /// Known logical size, without decoding the payload.
    pub size: Option<u64>,
}

/// Classify a selected archive/source pair without reading payloads.
///
/// `disk_time_vs_archive` is the source time compared to the archive time after
/// the caller applies archive precision, timezone and representable-range rules.
/// `None` represents unavailable comparison, not an invented timestamp. With
/// unknown time, equal known sizes are state `z`, matching the reference policy;
/// this does not establish equal contents.
///
/// # Errors
/// Rejects an empty pair or a disk item colliding with an unselected archive item.
pub fn classify(
    archive: Option<UpdateItem>,
    disk: Option<UpdateItem>,
    selected: bool,
    disk_time_vs_archive: Option<Ordering>,
) -> Result<UpdateState> {
    Ok(match (archive, disk) {
        (None, None) => return Err(Error::Malformed("empty update pair".into())),
        (Some(_), None) if !selected => UpdateState::NotSelected,
        (Some(_), Some(_)) if !selected => {
            return Err(Error::Malformed(
                "source collides with unselected archive item".into(),
            ));
        }
        (Some(_), None) => UpdateState::OnlyInArchive,
        (None, Some(_)) => UpdateState::OnlyOnDisk,
        (Some(archive), Some(disk)) => match disk_time_vs_archive {
            Some(Ordering::Less) => UpdateState::ArchiveNewer,
            Some(Ordering::Greater) => UpdateState::DiskNewer,
            Some(Ordering::Equal) | None => {
                if archive.size.is_some() && archive.size == disk.size {
                    UpdateState::Same
                } else {
                    UpdateState::Unknown
                }
            }
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEM: UpdateItem = UpdateItem { size: Some(42) };

    #[test]
    fn classify_all_states_and_unknown_metadata() {
        for (archive, disk, selected, ordering, expected) in [
            (Some(ITEM), None, false, None, UpdateState::NotSelected),
            (Some(ITEM), None, true, None, UpdateState::OnlyInArchive),
            (None, Some(ITEM), true, None, UpdateState::OnlyOnDisk),
            (
                Some(ITEM),
                Some(ITEM),
                true,
                Some(Ordering::Less),
                UpdateState::ArchiveNewer,
            ),
            (
                Some(ITEM),
                Some(ITEM),
                true,
                Some(Ordering::Greater),
                UpdateState::DiskNewer,
            ),
            (
                Some(ITEM),
                Some(ITEM),
                true,
                Some(Ordering::Equal),
                UpdateState::Same,
            ),
            (Some(ITEM), Some(ITEM), true, None, UpdateState::Same),
            (
                Some(UpdateItem { size: None }),
                Some(ITEM),
                true,
                None,
                UpdateState::Unknown,
            ),
            (
                Some(ITEM),
                Some(UpdateItem { size: Some(43) }),
                true,
                None,
                UpdateState::Unknown,
            ),
        ] {
            assert_eq!(
                classify(archive, disk, selected, ordering).unwrap(),
                expected
            );
        }
        assert!(classify(None, None, true, None).is_err());
        assert!(classify(Some(ITEM), Some(ITEM), false, None).is_err());
    }

    #[test]
    fn policies_preserve_excluded_items_and_distinguish_equal_time() {
        use UpdateAction::{Omit as O, Retain as R, UseDisk as D};
        for (mode, expected) in [
            (UpdateMode::Add, [R, R, D, D, D, D, D]),
            (UpdateMode::Update, [R, R, D, R, D, R, D]),
            (UpdateMode::Delete, [R, O, O, O, O, O, O]),
            (UpdateMode::Freshen, [R, R, O, R, D, R, D]),
            (UpdateMode::Synchronize, [R, O, D, R, D, R, D]),
        ] {
            assert_eq!(UpdateActionSet::for_mode(mode).actions, expected);
        }
    }

    #[test]
    fn repeated_sets_reset_defaults_and_additional_outputs_are_independent() {
        let result = UpdateSwitches::parse(
            UpdateMode::Update,
            &["x2", "z2", "q0!copy.zip", "!other.zip"],
        )
        .unwrap();
        let base = result.base.unwrap();
        assert_eq!(base.action(UpdateState::ArchiveNewer), UpdateAction::Retain);
        assert_eq!(base.action(UpdateState::Same), UpdateAction::UseDisk);
        assert_eq!(
            result.additional[0]
                .actions
                .action(UpdateState::OnlyInArchive),
            UpdateAction::Omit
        );
        assert_eq!(
            result.additional[1].actions,
            UpdateActionSet::for_mode(UpdateMode::Update)
        );
    }

    #[test]
    fn disabled_base_stays_disabled_and_names_accept_unicode() {
        let result =
            UpdateSwitches::parse(UpdateMode::Update, &["-", "x2", "Y1!résultat.zip"]).unwrap();
        assert_eq!(result.base, None);
        assert_eq!(result.additional[0].name, "résultat.zip");
        assert_eq!(
            result.additional[0].actions.action(UpdateState::DiskNewer),
            UpdateAction::Retain
        );
    }

    #[test]
    fn invalid_switches_fail_and_anti_items_require_backend_support() {
        for suffix in [
            "p2",
            "q2",
            "r1",
            "x",
            "z4",
            "z-",
            "!",
            "x2garbage",
            "é2",
            "x2-",
            "!a\0b",
        ] {
            assert!(
                UpdateSwitches::parse(UpdateMode::Update, &[suffix]).is_err(),
                "{suffix:?}"
            );
        }
        let anti = UpdateSwitches::parse(UpdateMode::Update, &["q3"])
            .unwrap()
            .base
            .unwrap();
        assert!(anti.validate_anti_items(false).is_err());
        anti.validate_anti_items(true).unwrap();
    }
}
