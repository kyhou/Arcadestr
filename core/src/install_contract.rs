//! Errors carried by the desktop installation command.

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum InstallCommandError {
    #[error("package decompression ratio exceeds safety limits")]
    CompressionRatioExceeded,
    #[error("{message}")]
    Other { message: String },
}

impl From<String> for InstallCommandError {
    fn from(message: String) -> Self {
        Self::Other { message }
    }
}

impl From<&str> for InstallCommandError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

impl InstallCommandError {
    #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
    pub fn from_extraction(error: &crate::package_extraction::ExtractionError) -> Self {
        use crate::archive_inspection::{DetectInspectError, InspectionError};
        use crate::package_extraction::ExtractionError;

        match error {
            ExtractionError::Policy(InspectionError::CompressionRatioExceeded { .. })
            | ExtractionError::Inspection(DetectInspectError::Inspection(
                InspectionError::CompressionRatioExceeded { .. },
            )) => Self::CompressionRatioExceeded,
            _ => error.to_string().into(),
        }
    }
}

#[cfg(all(test, feature = "native", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::archive_inspection::{DetectInspectError, InspectionError};
    use crate::package_extraction::ExtractionError;

    #[test]
    fn ratio_rejection_survives_both_extraction_passes_and_serialization() {
        for error in [
            ExtractionError::Policy(InspectionError::CompressionRatioExceeded {
                measured_input: 1,
                measured_output: 1001,
                maximum_ratio: 1000,
            }),
            ExtractionError::Inspection(DetectInspectError::Inspection(
                InspectionError::CompressionRatioExceeded {
                    measured_input: 1,
                    measured_output: 1001,
                    maximum_ratio: 1000,
                },
            )),
        ] {
            let value = serde_json::to_value(InstallCommandError::from_extraction(&error))
                .expect("serialize install error");
            assert_eq!(
                value,
                serde_json::json!({"code": "compression_ratio_exceeded"})
            );
            assert_eq!(
                serde_json::from_value::<InstallCommandError>(value).expect("decode install error"),
                InstallCommandError::CompressionRatioExceeded,
            );
        }
    }

    #[test]
    fn other_resource_limits_and_error_text_are_not_ratio_rejections() {
        for error in [
            ExtractionError::Policy(InspectionError::EntryCountOverflow),
            ExtractionError::Inspection(DetectInspectError::Inspection(InspectionError::ZipIo {
                message: "CompressionRatioExceeded".into(),
            })),
        ] {
            assert!(matches!(
                InstallCommandError::from_extraction(&error),
                InstallCommandError::Other { .. }
            ));
        }
    }
}
