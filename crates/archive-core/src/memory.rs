//! Portable interpretation of 7-Zip-style codec memory budgets.
use crate::Error;
use std::str::FromStr;

/// The operation whose automatic memory allowance is being selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOperation {
    Compress,
    Decompress,
}

/// A codec workspace allowance using 7-Zip's `memuse` syntax and defaults.
///
/// This is a component budget, not a process-wide allocator or RSS limit.
/// Callers supply physical RAM because the portable core does not query the OS.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MemoryUsage {
    #[default]
    Auto,
    Bytes(u64),
    Percent(u64),
}

impl MemoryUsage {
    /// Resolve against total usable physical RAM, or 7-Zip's fallback when
    /// detection is unavailable. Automatic allowances are 80% for compression
    /// and `RAM / 32 * 17` for decompression. A 32-bit automatic allowance uses
    /// at most 1.75 GiB as its RAM base. Explicit percentages use the full base.
    pub fn budget(self, physical_ram: Option<u64>, operation: MemoryOperation) -> u64 {
        self.budget_for_width(physical_ram, operation, usize::BITS)
    }

    fn budget_for_width(
        self,
        physical_ram: Option<u64>,
        operation: MemoryOperation,
        pointer_bits: u32,
    ) -> u64 {
        let fallback = u64::from(pointer_bits / 8) << 28;
        match self {
            Self::Bytes(bytes) => bytes,
            Self::Percent(percent) => percent_bytes(physical_ram.unwrap_or(fallback), percent),
            Self::Auto => {
                let Some(mut ram) = physical_ram else {
                    return fallback;
                };
                if pointer_bits == 32 {
                    ram = ram.min(7 << 28);
                }
                match operation {
                    MemoryOperation::Compress => percent_bytes(ram, 80),
                    MemoryOperation::Decompress => ram / 32 * 17,
                }
            }
        }
    }
}

fn percent_bytes(ram: u64, percent: u64) -> u64 {
    // Match 7-Zip's Calc_From_Val_Percents, including its overflow fallback
    // and integer rounding when the remainder cannot be multiplied safely.
    let whole = percent / 100;
    let remainder = percent % 100;
    let ceiling = i64::MAX as u64;
    if whole != 0 && ram > ceiling / whole {
        return ceiling;
    }
    let base = ram * whole;
    let fraction = if remainder == 0 {
        0
    } else if ram <= ceiling / remainder {
        ram * remainder / 100
    } else {
        ram / 100 * remainder
    };
    base.checked_add(fraction).unwrap_or(ceiling)
}

impl FromStr for MemoryUsage {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || {
            Error::Malformed(
                "memory usage must be auto, pN, N%, or N with an optional b/k/m/g/t suffix".into(),
            )
        };
        if value.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        let (percent_prefix, digits) = match value.as_bytes().first() {
            Some(b'p' | b'P') => (true, &value[1..]),
            _ => (false, value),
        };
        let count = digits.bytes().take_while(u8::is_ascii_digit).count();
        if count == 0 {
            return Err(invalid());
        }
        let number = digits[..count].parse::<u64>().map_err(|_| invalid())?;
        let suffix = &digits[count..];
        if percent_prefix {
            return if suffix.is_empty() {
                Ok(Self::Percent(number))
            } else {
                Err(invalid())
            };
        }
        if suffix == "%" {
            return Ok(Self::Percent(number));
        }
        let shift = match suffix.as_bytes() {
            [] | [b'b' | b'B'] => 0,
            [b'k' | b'K'] => 10,
            [b'm' | b'M'] => 20,
            [b'g' | b'G'] => 30,
            [b't' | b'T'] => 40,
            _ => return Err(invalid()),
        };
        number
            .checked_mul(1u64 << shift)
            .map(Self::Bytes)
            .ok_or_else(invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_budgets_match_7zip_defaults_and_detection_fallback() {
        let ram = 16u64 << 30;
        assert_eq!(
            MemoryUsage::Auto.budget_for_width(Some(ram), MemoryOperation::Compress, 64),
            ram * 80 / 100
        );
        assert_eq!(
            MemoryUsage::Auto.budget_for_width(Some(ram + 31), MemoryOperation::Decompress, 64),
            ram / 32 * 17
        );
        for operation in [MemoryOperation::Compress, MemoryOperation::Decompress] {
            assert_eq!(
                MemoryUsage::Auto.budget_for_width(None, operation, 64),
                2 << 30
            );
            assert_eq!(
                MemoryUsage::Auto.budget_for_width(None, operation, 32),
                1 << 30
            );
        }
        assert_eq!(
            MemoryUsage::Auto.budget_for_width(Some(ram), MemoryOperation::Decompress, 32),
            (7u64 << 28) / 32 * 17
        );
        assert_eq!(
            MemoryUsage::Percent(100).budget_for_width(Some(ram), MemoryOperation::Decompress, 32),
            ram
        );
    }

    #[test]
    fn explicit_sizes_and_percentages_match_7zip_command_syntax() {
        for (text, expected) in [
            ("auto", MemoryUsage::Auto),
            ("p80", MemoryUsage::Percent(80)),
            ("P125", MemoryUsage::Percent(125)),
            ("53%", MemoryUsage::Percent(53)),
            ("0", MemoryUsage::Bytes(0)),
            ("32768", MemoryUsage::Bytes(32768)),
            ("12b", MemoryUsage::Bytes(12)),
            ("32K", MemoryUsage::Bytes(32 << 10)),
            ("256m", MemoryUsage::Bytes(256 << 20)),
            ("2g", MemoryUsage::Bytes(2 << 30)),
            ("1t", MemoryUsage::Bytes(1 << 40)),
        ] {
            assert_eq!(text.parse::<MemoryUsage>().unwrap(), expected);
        }
        for text in [
            "",
            "p",
            "-1",
            "+1",
            "1.5g",
            "2gb",
            "p50%",
            "50%x",
            "18446744073709551616",
            "16777216t",
        ] {
            assert!(text.parse::<MemoryUsage>().is_err(), "{text}");
        }
        assert_eq!(
            MemoryUsage::Percent(125).budget(Some(1000), MemoryOperation::Compress),
            1250
        );
        assert_eq!(
            MemoryUsage::Percent(u64::MAX).budget(Some(u64::MAX), MemoryOperation::Compress),
            i64::MAX as u64
        );
        let ram = i64::MAX as u64;
        assert_eq!(
            MemoryUsage::Percent(125).budget(Some(ram), MemoryOperation::Compress),
            ram + ram / 100 * 25
        );
    }
}
