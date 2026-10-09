//! Bounded selection over stored archive-name bytes.
//!
//! This native extension is deliberately smaller than 7-Zip's selector grammar.
//! `/` is the only path separator. Wildcards operate on bytes: `*` matches zero
//! or more bytes within one component and `?` matches one byte within a component.
//! There is no Unicode normalization, filesystem lookup, extraction-path
//! validation, recursive-glob syntax, escaping, or automatic listfile expansion.
//! Leading `-` and `@` are ordinary name bytes.

use thiserror::Error;

/// How stored bytes are compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum CasePolicy {
    /// Every byte must compare exactly, including UTF-8/non-ASCII bytes.
    #[default]
    Sensitive,
    /// Fold ASCII A–Z only; non-ASCII bytes remain exact.
    AsciiInsensitive,
}

/// Pattern interpretation, never inferred from its spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PatternMode {
    /// Treat every byte literally, including wildcard/listfile markers.
    Literal,
    /// Support component-local `*` and `?`; reject other glob syntax.
    #[default]
    Wildcard,
}

/// Which portion of the stored name is matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum MatchScope {
    /// Match from the first byte of the complete stored path.
    #[default]
    FullPath,
    /// Match its final component, or ancestor components when descendants apply.
    Basename,
}

/// Resource ceilings for constructing and applying a selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionLimits {
    /// Maximum number of include plus exclude patterns.
    pub max_patterns: usize,
    /// Maximum bytes in one pattern.
    pub max_pattern_bytes: usize,
    /// Maximum combined bytes across all patterns.
    pub max_total_pattern_bytes: usize,
    /// Maximum stored-name bytes considered by one match call.
    pub max_name_bytes: usize,
    /// Maximum byte comparisons/DP cells across one match call.
    pub max_match_steps: usize,
}

impl Default for SelectionLimits {
    fn default() -> Self {
        Self {
            max_patterns: 256,
            max_pattern_bytes: 4096,
            max_total_pattern_bytes: 64 * 1024,
            max_name_bytes: 16 * 1024,
            max_match_steps: 4 * 1024 * 1024,
        }
    }
}

/// Unsupported syntax, invalid input or resource exhaustion.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SelectionError {
    /// A pattern must contain at least one byte.
    #[error("selection pattern is empty")]
    EmptyPattern,
    /// NUL is unsupported in names and patterns.
    #[error("selection names and patterns must not contain NUL")]
    NulByte,
    /// Advanced syntax must be rejected rather than silently given another meaning.
    #[error("unsupported wildcard syntax: {0}")]
    UnsupportedSyntax(&'static str),
    /// An explicit count/byte/work ceiling was exhausted.
    #[error("selection resource limit exceeded: {0}")]
    LimitExceeded(&'static str),
    /// Initial listfile profile accepts strict UTF-8 only.
    #[error("listfile must be UTF-8 (an optional UTF-8 BOM is supported)")]
    InvalidUtf8,
    /// Encodings outside the supported listfile profile are rejected explicitly.
    #[error("UTF-16/UTF-32 listfiles are unavailable; use UTF-8")]
    UnsupportedListfileEncoding,
}

/// One stored-name pattern and its explicit matching policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamePattern {
    bytes: Vec<u8>,
    mode: PatternMode,
    scope: MatchScope,
    include_descendants: bool,
}

impl NamePattern {
    /// Construct a pattern. No shell/listfile parsing or normalization is performed.
    ///
    /// Descendants match only after a `/` boundary: `dir` can select `dir/file`,
    /// but cannot select `directory/file`. For basename patterns, descendants
    /// additionally permit matching any ancestor component.
    ///
    /// # Errors
    /// Rejects empty/NUL patterns and unsupported wildcard grammar. Size limits
    /// are enforced by [`NameSelection::new`].
    pub fn new(
        bytes: impl Into<Vec<u8>>,
        mode: PatternMode,
        scope: MatchScope,
        include_descendants: bool,
    ) -> Result<Self, SelectionError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(SelectionError::EmptyPattern);
        }
        if bytes.contains(&0) {
            return Err(SelectionError::NulByte);
        }
        if mode == PatternMode::Wildcard {
            if bytes.windows(2).any(|pair| pair == b"**") {
                return Err(SelectionError::UnsupportedSyntax("recursive **"));
            }
            if bytes.iter().any(|byte| b"[]{}\\".contains(byte)) {
                return Err(SelectionError::UnsupportedSyntax(
                    "brackets, braces and escapes",
                ));
            }
        }
        if scope == MatchScope::Basename && bytes.contains(&b'/') {
            return Err(SelectionError::UnsupportedSyntax(
                "basename pattern contains /",
            ));
        }
        Ok(Self {
            bytes,
            mode,
            scope,
            include_descendants,
        })
    }

    /// Return the original raw pattern bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Reusable immutable include/exclude selector for one archive-name namespace.
///
/// An empty include set selects all names. Any exclude wins over an include.
/// Separate calls use separate work budgets; callers selecting entire archives
/// must also bound entry count/aggregate operation work.
#[derive(Debug, Clone)]
pub struct NameSelection {
    includes: Vec<NamePattern>,
    excludes: Vec<NamePattern>,
    case_policy: CasePolicy,
    limits: SelectionLimits,
}

impl NameSelection {
    /// Validate pattern count/bytes and construct a selector.
    ///
    /// # Errors
    /// Returns [`SelectionError::LimitExceeded`] before retaining patterns when
    /// the combined pattern count or byte budgets are exceeded.
    pub fn new(
        includes: &[NamePattern],
        excludes: &[NamePattern],
        case_policy: CasePolicy,
        limits: SelectionLimits,
    ) -> Result<Self, SelectionError> {
        let count = includes
            .len()
            .checked_add(excludes.len())
            .ok_or(SelectionError::LimitExceeded("pattern count"))?;
        if count > limits.max_patterns {
            return Err(SelectionError::LimitExceeded("pattern count"));
        }
        let mut bytes = 0usize;
        for pattern in includes.iter().chain(excludes) {
            if pattern.bytes.len() > limits.max_pattern_bytes {
                return Err(SelectionError::LimitExceeded("pattern bytes"));
            }
            bytes = bytes
                .checked_add(pattern.bytes.len())
                .ok_or(SelectionError::LimitExceeded("total pattern bytes"))?;
            if bytes > limits.max_total_pattern_bytes {
                return Err(SelectionError::LimitExceeded("total pattern bytes"));
            }
        }
        Ok(Self {
            includes: includes.to_vec(),
            excludes: excludes.to_vec(),
            case_policy,
            limits,
        })
    }

    /// Select all stored names, subject to the default name-size limit.
    pub fn all() -> Self {
        Self {
            includes: Vec::new(),
            excludes: Vec::new(),
            case_policy: CasePolicy::Sensitive,
            limits: SelectionLimits::default(),
        }
    }

    /// Determine whether stored archive-name bytes are selected.
    ///
    /// This result grants no permission to publish a filesystem destination.
    /// Wildcard DP has bounded polynomial work and two name-sized state arrays.
    /// It does not recurse or search an exponential set of wildcard expansions.
    ///
    /// # Errors
    /// Rejects NUL, excessive name bytes or the accumulated matching-work limit.
    pub fn matches(&self, name: &[u8]) -> Result<bool, SelectionError> {
        if name.len() > self.limits.max_name_bytes {
            return Err(SelectionError::LimitExceeded("name bytes"));
        }
        if name.contains(&0) {
            return Err(SelectionError::NulByte);
        }
        let mut budget = self.limits.max_match_steps;
        for pattern in &self.excludes {
            if self.match_pattern(pattern, name, &mut budget)? {
                return Ok(false);
            }
        }
        if self.includes.is_empty() {
            return Ok(true);
        }
        for pattern in &self.includes {
            if self.match_pattern(pattern, name, &mut budget)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn match_pattern(
        &self,
        pattern: &NamePattern,
        name: &[u8],
        budget: &mut usize,
    ) -> Result<bool, SelectionError> {
        match pattern.scope {
            MatchScope::FullPath => self.match_bytes(pattern, name, budget),
            MatchScope::Basename => {
                if pattern.include_descendants {
                    for component in name.split(|byte| *byte == b'/') {
                        if self.match_bytes(pattern, component, budget)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                } else {
                    let component = name.rsplit(|byte| *byte == b'/').next().unwrap_or(name);
                    self.match_bytes(pattern, component, budget)
                }
            }
        }
    }

    fn match_bytes(
        &self,
        pattern: &NamePattern,
        name: &[u8],
        budget: &mut usize,
    ) -> Result<bool, SelectionError> {
        if pattern.mode == PatternMode::Literal {
            spend(budget, pattern.bytes.len())?;
            if name.len() < pattern.bytes.len() {
                return Ok(false);
            }
            let matches = pattern
                .bytes
                .iter()
                .zip(name)
                .all(|(&a, &b)| self.equal(a, b));
            return Ok(matches
                && (name.len() == pattern.bytes.len()
                    || (pattern.include_descendants && name[pattern.bytes.len()] == b'/')));
        }
        let width = name
            .len()
            .checked_add(1)
            .ok_or(SelectionError::LimitExceeded("matching work"))?;
        let work = width
            .checked_mul(pattern.bytes.len())
            .ok_or(SelectionError::LimitExceeded("matching work"))?;
        spend(budget, work)?;
        let mut previous = vec![false; width];
        let mut current = vec![false; width];
        previous[0] = true;
        for &token in &pattern.bytes {
            current.fill(false);
            if token == b'*' {
                current[0] = previous[0];
                for index in 1..width {
                    current[index] =
                        previous[index] || (name[index - 1] != b'/' && current[index - 1]);
                }
            } else {
                for index in 1..width {
                    let byte = name[index - 1];
                    current[index] = previous[index - 1]
                        && (if token == b'?' {
                            byte != b'/'
                        } else {
                            self.equal(token, byte)
                        });
                }
            }
            std::mem::swap(&mut previous, &mut current);
        }
        Ok(previous[name.len()]
            || (pattern.include_descendants
                && name
                    .iter()
                    .enumerate()
                    .any(|(index, &byte)| byte == b'/' && previous[index])))
    }

    fn equal(&self, a: u8, b: u8) -> bool {
        match self.case_policy {
            CasePolicy::Sensitive => a == b,
            CasePolicy::AsciiInsensitive => a.eq_ignore_ascii_case(&b),
        }
    }
}

fn spend(budget: &mut usize, work: usize) -> Result<(), SelectionError> {
    *budget = budget
        .checked_sub(work)
        .ok_or(SelectionError::LimitExceeded("matching work"))?;
    Ok(())
}

/// Decode the initial strict UTF-8 listfile profile into patterns.
///
/// An optional UTF-8 BOM and LF/CRLF line endings are accepted. Empty lines are
/// skipped; spaces, quotes, leading `-` and `@` are literal bytes, with no shell
/// quoting/comments/listfile expansion. UTF-16/UTF-32 BOMs and NUL are rejected.
/// Pattern count and byte budgets are checked before each pattern allocation.
///
/// # Errors
/// Returns encoding/syntax errors or [`SelectionError::LimitExceeded`] for
/// excessive listfile/pattern input. Applying [`NameSelection::new`] also bounds
/// the combined include/exclude sets.
pub fn patterns_from_utf8_listfile(
    bytes: &[u8],
    mode: PatternMode,
    scope: MatchScope,
    include_descendants: bool,
    limits: SelectionLimits,
) -> Result<Vec<NamePattern>, SelectionError> {
    if bytes.starts_with(&[0xff, 0xfe])
        || bytes.starts_with(&[0xfe, 0xff])
        || bytes.starts_with(&[0, 0, 0xfe, 0xff])
    {
        return Err(SelectionError::UnsupportedListfileEncoding);
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    // Permit bounded line terminators in addition to the retained pattern bytes.
    let max_input = limits
        .max_patterns
        .checked_mul(2)
        .and_then(|terminators| limits.max_total_pattern_bytes.checked_add(terminators))
        .ok_or(SelectionError::LimitExceeded("listfile bytes"))?;
    if bytes.len() > max_input {
        return Err(SelectionError::LimitExceeded("listfile bytes"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| SelectionError::InvalidUtf8)?;
    let mut patterns = Vec::new();
    let mut total = 0usize;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        if patterns.len() >= limits.max_patterns {
            return Err(SelectionError::LimitExceeded("pattern count"));
        }
        if line.len() > limits.max_pattern_bytes {
            return Err(SelectionError::LimitExceeded("pattern bytes"));
        }
        total = total
            .checked_add(line.len())
            .ok_or(SelectionError::LimitExceeded("total pattern bytes"))?;
        if total > limits.max_total_pattern_bytes {
            return Err(SelectionError::LimitExceeded("total pattern bytes"));
        }
        patterns.push(NamePattern::new(
            line.as_bytes(),
            mode,
            scope,
            include_descendants,
        )?);
    }
    Ok(patterns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(bytes: &[u8], mode: PatternMode, descendants: bool) -> NamePattern {
        NamePattern::new(bytes, mode, MatchScope::FullPath, descendants).unwrap()
    }

    fn selection(includes: &[NamePattern], excludes: &[NamePattern]) -> NameSelection {
        NameSelection::new(
            includes,
            excludes,
            CasePolicy::Sensitive,
            SelectionLimits::default(),
        )
        .unwrap()
    }

    #[test]
    fn exclusions_win_and_directory_matches_require_boundaries() {
        let selector = selection(
            &[pattern(b"dir", PatternMode::Literal, true)],
            &[pattern(b"dir/private", PatternMode::Literal, true)],
        );
        for name in [b"dir".as_slice(), b"dir/a", b"dir/private-other/a"] {
            assert!(selector.matches(name).unwrap());
        }
        for name in [b"directory/a".as_slice(), b"dir/private/a", b"other/dir/a"] {
            assert!(!selector.matches(name).unwrap());
        }
    }

    #[test]
    fn component_wildcards_do_not_cross_separators() {
        let selector = selection(&[pattern(b"a/*.?", PatternMode::Wildcard, false)], &[]);
        assert!(selector.matches(b"a/file.x").unwrap());
        assert!(!selector.matches(b"a/nested/file.x").unwrap());
        assert!(!selector.matches(b"a/file.xy").unwrap());
        let descendants = selection(&[pattern(b"a/*", PatternMode::Wildcard, true)], &[]);
        assert!(descendants.matches(b"a/nested/file.x").unwrap());
    }

    #[test]
    fn raw_bytes_and_leading_markers_are_not_expanded() {
        for bytes in [b"-name".as_slice(), b"@list", b"a*?[]", &[0xff, b'x']] {
            let selector = selection(&[pattern(bytes, PatternMode::Literal, false)], &[]);
            assert!(selector.matches(bytes).unwrap());
        }
        let selector = selection(&[pattern(b"?", PatternMode::Wildcard, false)], &[]);
        assert!(selector.matches(&[0xff]).unwrap());
        assert!(!selector.matches("é".as_bytes()).unwrap());
    }

    #[test]
    fn ascii_case_folding_keeps_non_ascii_bytes_exact() {
        let patterns = [pattern("Ä/File".as_bytes(), PatternMode::Literal, false)];
        let selector = NameSelection::new(
            &patterns,
            &[],
            CasePolicy::AsciiInsensitive,
            SelectionLimits::default(),
        )
        .unwrap();
        assert!(selector.matches("Ä/fILE".as_bytes()).unwrap());
        assert!(!selector.matches("ä/file".as_bytes()).unwrap());
        assert!(
            !selection(&patterns, &[])
                .matches("Ä/file".as_bytes())
                .unwrap()
        );
    }

    #[test]
    fn basename_descendants_are_explicit() {
        for descendants in [false, true] {
            let patterns = [NamePattern::new(
                b"cache",
                PatternMode::Literal,
                MatchScope::Basename,
                descendants,
            )
            .unwrap()];
            let selector = selection(&patterns, &[]);
            assert!(selector.matches(b"root/cache").unwrap());
            assert_eq!(selector.matches(b"root/cache/file").unwrap(), descendants);
            assert!(!selector.matches(b"root/cache-other/file").unwrap());
        }
    }

    #[test]
    fn unsupported_grammar_fails_before_matching() {
        for bytes in [b"**".as_slice(), b"[ab]", b"{a,b}", b"a\\*"] {
            assert!(matches!(
                NamePattern::new(bytes, PatternMode::Wildcard, MatchScope::FullPath, false,),
                Err(SelectionError::UnsupportedSyntax(_))
            ));
        }
        assert_eq!(
            NamePattern::new(b"", PatternMode::Literal, MatchScope::FullPath, false),
            Err(SelectionError::EmptyPattern)
        );
        assert_eq!(
            NamePattern::new(b"a\0", PatternMode::Literal, MatchScope::FullPath, false),
            Err(SelectionError::NulByte)
        );
    }

    #[test]
    fn pattern_name_and_accumulated_work_budgets_fail_closed() {
        let patterns = [pattern(b"a*", PatternMode::Wildcard, false)];
        let limits = SelectionLimits {
            max_pattern_bytes: 1,
            ..SelectionLimits::default()
        };
        assert!(matches!(
            NameSelection::new(&patterns, &[], CasePolicy::Sensitive, limits),
            Err(SelectionError::LimitExceeded("pattern bytes"))
        ));
        let limits = SelectionLimits {
            max_name_bytes: 3,
            ..SelectionLimits::default()
        };
        let selector = NameSelection::new(&[], &[], CasePolicy::Sensitive, limits).unwrap();
        assert_eq!(
            selector.matches(b"abcd"),
            Err(SelectionError::LimitExceeded("name bytes"))
        );
        // Neither pattern matches, so work must be charged across both attempts.
        let patterns = [
            pattern(b"a?", PatternMode::Wildcard, false),
            pattern(b"b?", PatternMode::Wildcard, false),
        ];
        let limits = SelectionLimits {
            max_match_steps: 6,
            ..SelectionLimits::default()
        };
        let selector = NameSelection::new(&patterns, &[], CasePolicy::Sensitive, limits).unwrap();
        assert_eq!(
            selector.matches(b"zz"),
            Err(SelectionError::LimitExceeded("matching work"))
        );
    }

    #[test]
    fn adversarial_glob_is_bounded_without_recursive_backtracking() {
        let bytes = b"*a*a*a*a*a*a*a*a*a*a*a*b";
        let selector = selection(&[pattern(bytes, PatternMode::Wildcard, false)], &[]);
        assert!(!selector.matches(&vec![b'a'; 4096]).unwrap());
        let limits = SelectionLimits {
            max_match_steps: 32,
            ..SelectionLimits::default()
        };
        let selector = NameSelection::new(
            &[pattern(bytes, PatternMode::Wildcard, false)],
            &[],
            CasePolicy::Sensitive,
            limits,
        )
        .unwrap();
        assert_eq!(
            selector.matches(&vec![b'a'; 4096]),
            Err(SelectionError::LimitExceeded("matching work"))
        );
    }

    #[test]
    fn listfiles_have_a_strict_bounded_utf8_profile() {
        let patterns = patterns_from_utf8_listfile(
            b"\xef\xbb\xbf-name\r\n@list\n quoted \n\n",
            PatternMode::Literal,
            MatchScope::FullPath,
            false,
            SelectionLimits::default(),
        )
        .unwrap();
        assert_eq!(
            patterns.iter().map(NamePattern::bytes).collect::<Vec<_>>(),
            [b"-name".as_slice(), b"@list", b" quoted "]
        );
        assert_eq!(
            patterns_from_utf8_listfile(
                &[0xff, 0xfe, 0, 0],
                PatternMode::Literal,
                MatchScope::FullPath,
                false,
                SelectionLimits::default()
            ),
            Err(SelectionError::UnsupportedListfileEncoding)
        );
        assert_eq!(
            patterns_from_utf8_listfile(
                &[0xff],
                PatternMode::Literal,
                MatchScope::FullPath,
                false,
                SelectionLimits::default()
            ),
            Err(SelectionError::InvalidUtf8)
        );
        let limits = SelectionLimits {
            max_patterns: 1,
            ..SelectionLimits::default()
        };
        assert_eq!(
            patterns_from_utf8_listfile(
                b"a\nb",
                PatternMode::Literal,
                MatchScope::FullPath,
                false,
                limits
            ),
            Err(SelectionError::LimitExceeded("pattern count"))
        );
    }
}
