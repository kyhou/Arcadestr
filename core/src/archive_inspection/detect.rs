//! Bounded content-based archive format detection and dispatch.

use std::io::{Read, Seek, SeekFrom};

use crate::distribution::PackageFormat;

use super::{inspect_tar_gz, inspect_tar_zstd, inspect_zip, ArchiveInspection, InspectionError};

/// Failures from bounded archive signature detection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveFormatDetectionError {
    UnknownFormat,
    AmbiguousFormat,
    TruncatedSignature,
}

/// Detects ZIP, gzip, or Zstandard from a bounded leading signature.
pub fn detect_archive_format<R: Read>(
    reader: &mut R,
) -> Result<PackageFormat, ArchiveFormatDetectionError> {
    let mut signature = [0_u8; 4];
    let mut read = 0;
    while read < signature.len() {
        match reader.read(&mut signature[read..]) {
            Ok(0) => break,
            Ok(count) => read += count,
            Err(_) => return Err(ArchiveFormatDetectionError::UnknownFormat),
        }
    }
    if read < 2 {
        return Err(ArchiveFormatDetectionError::TruncatedSignature);
    }
    if signature[..2] == [0x1f, 0x8b] {
        return Ok(PackageFormat::TarGz);
    }
    if read < 4 {
        return Err(ArchiveFormatDetectionError::TruncatedSignature);
    }
    match signature {
        [0x50, 0x4b, 0x03, 0x04] | [0x50, 0x4b, 0x05, 0x06] | [0x50, 0x4b, 0x07, 0x08] => {
            Ok(PackageFormat::Zip)
        }
        [0x28, 0xb5, 0x2f, 0xfd] => Ok(PackageFormat::TarZstd),
        _ => Err(ArchiveFormatDetectionError::UnknownFormat),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_signatures_require_exact_pairs() {
        for signature in [
            [0x50, 0x4b, 0x03, 0x04],
            [0x50, 0x4b, 0x05, 0x06],
            [0x50, 0x4b, 0x07, 0x08],
        ] {
            assert_eq!(
                detect_archive_format(&mut signature.as_slice()),
                Ok(PackageFormat::Zip)
            );
        }
        for signature in [
            [0x50, 0x4b, 0x03, 0x06],
            [0x50, 0x4b, 0x05, 0x08],
            [0x50, 0x4b, 0x07, 0x04],
        ] {
            assert_eq!(
                detect_archive_format(&mut signature.as_slice()),
                Err(ArchiveFormatDetectionError::UnknownFormat)
            );
        }
    }

    #[test]
    fn a3_5_2_detects_gzip_and_zstd_signatures() {
        // Arrange
        let gzip = [0x1f, 0x8b, 0x08, 0x00];
        let zstd = [0x28, 0xb5, 0x2f, 0xfd];

        // Act
        let gzip_result = detect_archive_format(&mut gzip.as_slice());
        let zstd_result = detect_archive_format(&mut zstd.as_slice());

        // Assert
        assert_eq!(gzip_result, Ok(PackageFormat::TarGz));
        assert_eq!(zstd_result, Ok(PackageFormat::TarZstd));
    }

    #[test]
    fn a3_5_2_rejects_truncated_signatures() {
        // Arrange
        let empty = [];
        let one_byte = [0x50];
        let partial_zip = [0x50, 0x4b, 0x03];

        // Act
        let results = [
            detect_archive_format(&mut empty.as_slice()),
            detect_archive_format(&mut one_byte.as_slice()),
            detect_archive_format(&mut partial_zip.as_slice()),
        ];

        // Assert
        assert!(results
            .into_iter()
            .all(|result| { result == Err(ArchiveFormatDetectionError::TruncatedSignature) }));
    }
}

/// Detects and inspects one seekable archive without trusting its extension.
pub fn inspect_detected_archive<R: Read + Seek>(
    mut reader: R,
) -> Result<(PackageFormat, ArchiveInspection), DetectInspectError> {
    let format = detect_archive_format(&mut reader).map_err(DetectInspectError::Detection)?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|_| DetectInspectError::Read)?;
    let inspection = match format {
        PackageFormat::Zip => inspect_zip(reader),
        PackageFormat::TarGz => inspect_tar_gz(reader),
        PackageFormat::TarZstd => inspect_tar_zstd(reader),
    }
    .map_err(DetectInspectError::Inspection)?;
    Ok((format, inspection))
}

/// Combined detection/inspection failure used by native adapters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetectInspectError {
    Detection(ArchiveFormatDetectionError),
    Inspection(InspectionError),
    Read,
}
