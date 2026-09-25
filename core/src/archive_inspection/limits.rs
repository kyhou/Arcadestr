//! Shared archive entry and measured decompression limits.

use super::InspectionError;

/// Maximum number of entries accepted by one archive.
pub const MAX_ENTRIES: u64 = 200_000;
/// Maximum cumulative measured output-to-input ratio.
pub const MAX_COMPRESSION_RATIO: u64 = 200;
/// Output bytes required before ratio enforcement begins.
pub const RATIO_ENFORCEMENT_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;

/// Bytes consumed by tar headers and GNU long-name/PAX extension records; alignment padding is attributed to payload because it follows file size; inspection-local policy, not protocol limit.
pub const MAX_TAR_METADATA_BYTES: u64 = 128 * 1024 * 1024;

/// Inclusive archive entry-limit accounting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryCounter {
    count: u64,
}

impl EntryCounter {
    /// Records an entry and returns the new inclusive count.
    ///
    /// # Errors
    ///
    /// Returns [`InspectionError::EntryLimitExceeded`] for entry 200,001.
    pub fn record_entry(&mut self) -> Result<u64, InspectionError> {
        let count = self
            .count
            .checked_add(1)
            .ok_or(InspectionError::EntryCountOverflow)?;
        if count > MAX_ENTRIES {
            return Err(InspectionError::EntryLimitExceeded { limit: MAX_ENTRIES });
        }
        self.count = count;
        Ok(count)
    }

    /// Returns the number of accepted entries.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }
}

/// Checked cumulative accounting for measured decompressor bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RatioAccounting {
    measured_input: u64,
    measured_output: u64,
}

impl RatioAccounting {
    /// Adds measured byte deltas and enforces the cumulative ratio policy.
    ///
    /// Declared entry sizes are intentionally absent from this API.
    ///
    /// # Errors
    ///
    /// Returns an overflow error if either counter cannot represent its sum.
    /// Once measured output exceeds 64 MiB, zero input and ratios above 200:1
    /// return dedicated errors.
    pub fn record(
        &mut self,
        measured_input: u64,
        measured_output: u64,
    ) -> Result<(), InspectionError> {
        let next_input = self
            .measured_input
            .checked_add(measured_input)
            .ok_or(InspectionError::MeasuredInputOverflow)?;
        let next_output = self
            .measured_output
            .checked_add(measured_output)
            .ok_or(InspectionError::MeasuredOutputOverflow)?;

        enforce_ratio(next_input, next_output)?;
        self.measured_input = next_input;
        self.measured_output = next_output;
        Ok(())
    }

    /// Returns cumulative measured compressed input bytes.
    #[must_use]
    pub fn measured_input(&self) -> u64 {
        self.measured_input
    }

    /// Returns cumulative measured regular-file output bytes.
    #[must_use]
    pub fn measured_output(&self) -> u64 {
        self.measured_output
    }
}

fn enforce_ratio(measured_input: u64, measured_output: u64) -> Result<(), InspectionError> {
    if measured_output <= RATIO_ENFORCEMENT_OUTPUT_BYTES {
        return Ok(());
    }
    if measured_input == 0 {
        return Err(InspectionError::ZeroMeasuredInput { measured_output });
    }

    let permitted_output = measured_input.checked_mul(MAX_COMPRESSION_RATIO);
    if permitted_output.is_some_and(|permitted| measured_output > permitted) {
        return Err(InspectionError::CompressionRatioExceeded {
            measured_input,
            measured_output,
            maximum_ratio: MAX_COMPRESSION_RATIO,
        });
    }
    // If multiplication overflowed, every possible u64 output is within 200:1.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_limit_accepts_200_000_and_rejects_200_001() {
        let mut counter = EntryCounter::default();
        for _ in 0..MAX_ENTRIES {
            counter
                .record_entry()
                .expect("entry should fit inclusive cap");
        }
        assert_eq!(counter.count(), MAX_ENTRIES);
        assert_eq!(
            counter.record_entry(),
            Err(InspectionError::EntryLimitExceeded { limit: MAX_ENTRIES })
        );
    }

    #[test]
    fn ratio_starts_only_after_64_mib_and_handles_zero_input() {
        let mut accounting = RatioAccounting::default();
        accounting
            .record(0, RATIO_ENFORCEMENT_OUTPUT_BYTES)
            .expect("threshold itself is not enforced");
        assert_eq!(
            accounting.record(0, 1),
            Err(InspectionError::ZeroMeasuredInput {
                measured_output: RATIO_ENFORCEMENT_OUTPUT_BYTES + 1,
            })
        );
    }

    #[test]
    fn ratio_accepts_200_to_1_and_rejects_the_next_output_byte() {
        let mut accounting = RatioAccounting::default();
        let input = 400 * 1024;
        let output = input * MAX_COMPRESSION_RATIO;
        accounting
            .record(input, output)
            .expect("exactly 200:1 should succeed");

        assert_eq!(
            accounting.record(0, 1),
            Err(InspectionError::CompressionRatioExceeded {
                measured_input: input,
                measured_output: output + 1,
                maximum_ratio: MAX_COMPRESSION_RATIO,
            })
        );
    }

    #[test]
    fn measured_counter_overflow_is_explicit_and_transactional() {
        let mut accounting = RatioAccounting::default();
        accounting
            .record(u64::MAX, 1)
            .expect("large input with small output is safe");

        assert_eq!(
            accounting.record(1, 0),
            Err(InspectionError::MeasuredInputOverflow)
        );
        assert_eq!(accounting.measured_input(), u64::MAX);
        assert_eq!(accounting.measured_output(), 1);
    }
}
