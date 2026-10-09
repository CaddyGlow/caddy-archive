//! Validated encoding options, kept separate from operation resource limits.
use crate::{Error, Result};
use serde::Serialize;

/// Validated native DEFLATE tuning for gzip, zlib and raw DEFLATE streams.
/// Levels describe the published backend's effort setting, not byte-identical
/// output or equivalence with upstream 7-Zip levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct DeflateOptions {
    level: u8,
}

impl Default for DeflateOptions {
    fn default() -> Self {
        Self { level: 6 }
    }
}

impl DeflateOptions {
    /// Set backend effort from 0 (stored blocks) to 9 (maximum effort).
    ///
    /// # Errors
    /// Rejects levels above 9 before any output is opened or written.
    pub fn with_level(mut self, level: u8) -> Result<Self> {
        if level > 9 {
            return Err(Error::Unsupported("DEFLATE level must be in 0..=9".into()));
        }
        self.level = level;
        Ok(self)
    }

    /// Effective level sent to the codec backend.
    pub const fn level(self) -> u8 {
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_range_is_checked_before_encoding() {
        for level in 0..=9 {
            assert_eq!(
                DeflateOptions::default().with_level(level).unwrap().level(),
                level
            );
        }
        assert!(DeflateOptions::default().with_level(10).is_err());
    }
}
