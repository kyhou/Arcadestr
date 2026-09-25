//! Native-only archive inspection types and shared safety policy.
//!
//! This module defines parser-independent indexing, path decoding, hazard
//! collection, measured resource accounting, and bounded format inspectors.
//! Inspectors stream from caller-provided readers and never extract entries.

mod candidates;
mod detect;
mod limits;
mod model;
mod tar;
mod zip;

use thiserror::Error;

#[doc(inline)]
pub use candidates::{
    detect_executable_candidates, CandidateConfidence, CandidateReason, ExecutableCandidate,
};
#[doc(inline)]
pub use detect::{
    detect_archive_format, inspect_detected_archive, ArchiveFormatDetectionError,
    DetectInspectError,
};
#[doc(inline)]
pub use limits::{
    EntryCounter, RatioAccounting, MAX_COMPRESSION_RATIO, MAX_ENTRIES, MAX_TAR_METADATA_BYTES,
    RATIO_ENFORCEMENT_OUTPUT_BYTES,
};
#[doc(inline)]
pub use model::{
    decode_entry_name, ArchiveEntry, ArchiveHazards, ArchiveIndex, ArchiveInspection,
    ArchiveInspectionBuilder, ArchiveWarning, ArchiveWarnings, ContainerTypeNameConflict,
    DecodedEntryName, EntryKind, UndecodablePath, ZipDeclaredEntryCountMismatch,
};
pub(crate) use tar::{
    entry_kind as tar_entry_kind, extraction_tar_gz_reader, extraction_tar_zstd_reader,
    map_extraction_io as map_tar_extraction_io, TarExtractionAccounting, TarExtractionStreamError,
};
#[doc(inline)]
pub use tar::{inspect_tar_gz, inspect_tar_zstd};
pub(crate) use zip::{
    decode_zip_name, entry_kind as zip_entry_kind, extraction_archive as extraction_zip_archive,
    extraction_entry as extraction_zip_entry, ZipExtractionAccounting,
};
#[doc(inline)]
pub use zip::{inspect_zip, MAX_ZIP_CENTRAL_DIRECTORY_BYTES};

/// A structural failure found by bounded ZIP footer preflight.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ZipPreflightError {
    /// No specification-conforming classic EOCD was present in the bounded tail.
    #[error("classic end-of-central-directory record not found")]
    EndOfCentralDirectoryNotFound,
    /// ZIP64 sentinel values were present without the required locator.
    #[error("ZIP64 end-of-central-directory locator is missing")]
    Zip64LocatorMissing,
    /// The ZIP64 locator or required EOCD fields were inconsistent.
    #[error("ZIP64 end-of-central-directory records are malformed")]
    Zip64RecordsMalformed,
    /// The footer describes a multi-disk ZIP, which is unsupported.
    #[error("multi-disk ZIP archives are unsupported")]
    MultiDiskUnsupported,
    /// Footer offset arithmetic exceeded `u64` or `usize`.
    #[error("ZIP footer offset arithmetic overflowed")]
    ArithmeticOverflow,
}

/// An archive inspection failure distinct from distribution resolution errors.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InspectionError {
    /// ZIP v1 encountered a compression method it does not support.
    #[error("unsupported compression method {method} for archive entry {entry}")]
    UnsupportedCompressionMethod {
        /// The decoded or display-safe entry name.
        entry: String,
        /// The ZIP compression method identifier.
        method: u16,
    },
    /// The archive attempted to add more than the inclusive entry cap.
    #[error("archive exceeds the {limit}-entry limit")]
    EntryLimitExceeded {
        /// The inclusive configured limit.
        limit: u64,
    },
    /// The entry count could not be represented as `u64`.
    #[error("archive entry count overflowed u64")]
    EntryCountOverflow,
    /// The declared ZIP central directory exceeds the inspection-local cap.
    #[error(
        "ZIP central directory exceeds the {limit}-byte limit ({declared_size} bytes declared)"
    )]
    ZipCentralDirectoryLimitExceeded {
        /// Central-directory bytes declared by EOCD or ZIP64 EOCD.
        declared_size: u64,
        /// Inspection-local central-directory metadata cap.
        limit: u64,
    },
    /// Bounded ZIP footer preflight found malformed structural metadata.
    #[error("ZIP preflight failed: {reason}")]
    ZipPreflightMalformed {
        /// Exact structural preflight failure.
        reason: ZipPreflightError,
    },
    /// Measured compressed-input accounting overflowed.
    #[error("measured compressed input overflowed u64")]
    MeasuredInputOverflow,
    /// Measured regular-file output accounting overflowed.
    #[error("measured regular-file output overflowed u64")]
    MeasuredOutputOverflow,
    /// Ratio enforcement began before any compressed input was measured.
    #[error("measured output reached {measured_output} bytes with zero measured input")]
    ZeroMeasuredInput {
        /// Cumulative measured regular-file output.
        measured_output: u64,
    },
    /// Measured cumulative decompression exceeded the allowed ratio.
    #[error(
        "measured decompression ratio exceeds {maximum_ratio}:1 ({measured_output} output bytes from {measured_input} input bytes)"
    )]
    CompressionRatioExceeded {
        /// Cumulative measured compressed input.
        measured_input: u64,
        /// Cumulative measured regular-file output.
        measured_output: u64,
        /// Maximum permitted output-to-input ratio.
        maximum_ratio: u64,
    },
    /// The shared ZIP input counter moved backwards unexpectedly.
    #[error("measured ZIP input counter regressed")]
    MeasuredInputCounterRegressed,
    /// Tar metadata decompression exceeded the inspection-local cap.
    #[error("tar metadata exceeds the {limit}-byte limit ({measured_metadata} bytes measured)")]
    TarMetadataLimitExceeded {
        /// Decompressed metadata bytes measured when the cap was exceeded.
        measured_metadata: u64,
        /// The inspection-local metadata limit.
        limit: u64,
    },
    /// Tar decompression or compressed-source reading failed.
    #[error("tar decoder error: {message}")]
    TarDecoder {
        /// Decoder-provided diagnostic text.
        message: String,
    },
    /// Tar container parsing failed.
    #[error("tar parser error: {message}")]
    TarParser {
        /// Parser-provided diagnostic text.
        message: String,
    },
    /// ZIP parsing or decoder setup failed.
    #[error("ZIP parser error: {message}")]
    ZipParser {
        /// Parser-provided diagnostic text.
        message: String,
    },
    /// ZIP source reading or decoder completion failed.
    #[error("ZIP I/O error: {message}")]
    ZipIo {
        /// I/O diagnostic text.
        message: String,
    },
}
