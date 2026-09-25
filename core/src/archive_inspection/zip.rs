//! Bounded, streaming ZIP and ZIP64 inspection.

use std::{
    cell::Cell,
    io::{self, Read, Seek, SeekFrom},
    rc::Rc,
};

use zip::{read::HasZipMetadata, result::ZipError, CompressionMethod, ZipArchive};

use super::{
    ArchiveInspection, ArchiveInspectionBuilder, ArchiveWarning, EntryKind, InspectionError,
    RatioAccounting, ZipPreflightError, MAX_ENTRIES,
};
use crate::distribution::PackageFormat;

const SCRATCH_BYTES: usize = 32 * 1024;
const UNIX_FILE_TYPE_MASK: u32 = 0o170_000;
const UNIX_FIFO: u32 = 0o010_000;
const UNIX_CHARACTER_DEVICE: u32 = 0o020_000;
const UNIX_DIRECTORY: u32 = 0o040_000;
const UNIX_BLOCK_DEVICE: u32 = 0o060_000;
const UNIX_REGULAR_FILE: u32 = 0o100_000;
const UNIX_SYMLINK: u32 = 0o120_000;
const UNIX_SOCKET: u32 = 0o140_000;
const DOS_DIRECTORY_ATTRIBUTE: u32 = 0x10;
const EFS_FLAG: u16 = 1 << 11;
const EOCD_SIGNATURE: [u8; 4] = 0x0605_4b50_u32.to_le_bytes();
const ZIP64_EOCD_SIGNATURE: [u8; 4] = 0x0606_4b50_u32.to_le_bytes();
const ZIP64_LOCATOR_SIGNATURE: [u8; 4] = 0x0706_4b50_u32.to_le_bytes();
const EOCD_FIXED_BYTES: usize = 22;
const MAX_EOCD_COMMENT_BYTES: usize = u16::MAX as usize;
const EOCD_TAIL_BYTES: usize = EOCD_FIXED_BYTES + MAX_EOCD_COMMENT_BYTES;
const ZIP64_LOCATOR_BYTES: u64 = 20;
const ZIP64_EOCD_REQUIRED_BYTES: usize = 56;
const ZIP64_EOCD_MIN_RECORD_SIZE: u64 = 44;

/// Maximum ZIP central-directory metadata accepted before parser allocation.
///
/// This inspection-local resource policy covers central-directory headers,
/// names, comments, and extra fields. It is not a protocol size limit.
pub const MAX_ZIP_CENTRAL_DIRECTORY_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ZipPreflight {
    declared_entries: u64,
    declared_central_directory_size: u64,
}

fn preflight_zip(reader: &mut (impl Read + Seek)) -> Result<ZipPreflight, InspectionError> {
    let file_len = reader.seek(SeekFrom::End(0)).map_err(map_preflight_io)?;
    let tail_len_u64 = file_len.min(EOCD_TAIL_BYTES as u64);
    let tail_len = usize::try_from(tail_len_u64)
        .map_err(|_| malformed(ZipPreflightError::ArithmeticOverflow))?;
    let tail_start = file_len
        .checked_sub(tail_len_u64)
        .ok_or_else(|| malformed(ZipPreflightError::ArithmeticOverflow))?;

    let mut tail = [0_u8; EOCD_TAIL_BYTES];
    reader
        .seek(SeekFrom::Start(tail_start))
        .map_err(map_preflight_io)?;
    reader
        .read_exact(&mut tail[..tail_len])
        .map_err(map_preflight_io)?;

    let eocd_in_tail = find_eocd(&tail[..tail_len])
        .ok_or_else(|| malformed(ZipPreflightError::EndOfCentralDirectoryNotFound))?;
    let eocd_offset = tail_start
        .checked_add(
            u64::try_from(eocd_in_tail)
                .map_err(|_| malformed(ZipPreflightError::ArithmeticOverflow))?,
        )
        .ok_or_else(|| malformed(ZipPreflightError::ArithmeticOverflow))?;
    let eocd_end = eocd_in_tail
        .checked_add(EOCD_FIXED_BYTES)
        .ok_or_else(|| malformed(ZipPreflightError::ArithmeticOverflow))?;
    let eocd = tail
        .get(eocd_in_tail..eocd_end)
        .ok_or_else(|| malformed(ZipPreflightError::EndOfCentralDirectoryNotFound))?;

    let disk_number = read_u16(eocd, 4)
        .ok_or_else(|| malformed(ZipPreflightError::EndOfCentralDirectoryNotFound))?;
    let central_directory_disk = read_u16(eocd, 6)
        .ok_or_else(|| malformed(ZipPreflightError::EndOfCentralDirectoryNotFound))?;
    let entries_on_disk = read_u16(eocd, 8)
        .ok_or_else(|| malformed(ZipPreflightError::EndOfCentralDirectoryNotFound))?;
    let entries = read_u16(eocd, 10)
        .ok_or_else(|| malformed(ZipPreflightError::EndOfCentralDirectoryNotFound))?;
    let central_directory_size = read_u32(eocd, 12)
        .ok_or_else(|| malformed(ZipPreflightError::EndOfCentralDirectoryNotFound))?;
    let central_directory_offset = read_u32(eocd, 16)
        .ok_or_else(|| malformed(ZipPreflightError::EndOfCentralDirectoryNotFound))?;

    if disk_number != 0 || central_directory_disk != 0 || entries_on_disk != entries {
        return Err(malformed(ZipPreflightError::MultiDiskUnsupported));
    }

    let uses_zip64 = entries == u16::MAX
        || central_directory_size == u32::MAX
        || central_directory_offset == u32::MAX;
    let preflight = if uses_zip64 {
        preflight_zip64(reader, eocd_offset)?
    } else {
        ZipPreflight {
            declared_entries: u64::from(entries),
            declared_central_directory_size: u64::from(central_directory_size),
        }
    };

    enforce_declared_preflight(preflight)
}

fn find_eocd(tail: &[u8]) -> Option<usize> {
    let last_start = tail.len().checked_sub(EOCD_FIXED_BYTES)?;
    for offset in (0..=last_start).rev() {
        let signature_end = offset.checked_add(EOCD_SIGNATURE.len())?;
        if tail.get(offset..signature_end) != Some(EOCD_SIGNATURE.as_slice()) {
            continue;
        }
        let comment_length_offset = offset.checked_add(20)?;
        let comment_length = usize::from(read_u16(tail, comment_length_offset)?);
        let end = offset
            .checked_add(EOCD_FIXED_BYTES)?
            .checked_add(comment_length)?;
        if end == tail.len() {
            return Some(offset);
        }
    }
    None
}

fn preflight_zip64(
    reader: &mut (impl Read + Seek),
    classic_eocd_offset: u64,
) -> Result<ZipPreflight, InspectionError> {
    let locator_offset = classic_eocd_offset
        .checked_sub(ZIP64_LOCATOR_BYTES)
        .ok_or_else(|| malformed(ZipPreflightError::Zip64LocatorMissing))?;
    let mut locator = [0_u8; ZIP64_LOCATOR_BYTES as usize];
    read_preflight_record(
        reader,
        locator_offset,
        &mut locator,
        ZipPreflightError::Zip64LocatorMissing,
    )?;
    if locator.get(..4) != Some(ZIP64_LOCATOR_SIGNATURE.as_slice()) {
        return Err(malformed(ZipPreflightError::Zip64LocatorMissing));
    }

    let central_directory_disk =
        read_u32(&locator, 4).ok_or_else(|| malformed(ZipPreflightError::Zip64RecordsMalformed))?;
    let zip64_eocd_offset =
        read_u64(&locator, 8).ok_or_else(|| malformed(ZipPreflightError::Zip64RecordsMalformed))?;
    let disk_count = read_u32(&locator, 16)
        .ok_or_else(|| malformed(ZipPreflightError::Zip64RecordsMalformed))?;
    if central_directory_disk != 0 || disk_count != 1 {
        return Err(malformed(ZipPreflightError::MultiDiskUnsupported));
    }

    let required_end = zip64_eocd_offset
        .checked_add(ZIP64_EOCD_REQUIRED_BYTES as u64)
        .ok_or_else(|| malformed(ZipPreflightError::ArithmeticOverflow))?;
    if required_end > locator_offset {
        return Err(malformed(ZipPreflightError::Zip64RecordsMalformed));
    }

    let mut eocd64 = [0_u8; ZIP64_EOCD_REQUIRED_BYTES];
    read_preflight_record(
        reader,
        zip64_eocd_offset,
        &mut eocd64,
        ZipPreflightError::Zip64RecordsMalformed,
    )?;
    if eocd64.get(..4) != Some(ZIP64_EOCD_SIGNATURE.as_slice()) {
        return Err(malformed(ZipPreflightError::Zip64RecordsMalformed));
    }

    let record_size =
        read_u64(&eocd64, 4).ok_or_else(|| malformed(ZipPreflightError::Zip64RecordsMalformed))?;
    if record_size < ZIP64_EOCD_MIN_RECORD_SIZE {
        return Err(malformed(ZipPreflightError::Zip64RecordsMalformed));
    }
    let record_end = zip64_eocd_offset
        .checked_add(12)
        .and_then(|offset| offset.checked_add(record_size))
        .ok_or_else(|| malformed(ZipPreflightError::ArithmeticOverflow))?;
    if record_end != locator_offset {
        return Err(malformed(ZipPreflightError::Zip64RecordsMalformed));
    }

    let disk_number =
        read_u32(&eocd64, 16).ok_or_else(|| malformed(ZipPreflightError::Zip64RecordsMalformed))?;
    let central_directory_disk =
        read_u32(&eocd64, 20).ok_or_else(|| malformed(ZipPreflightError::Zip64RecordsMalformed))?;
    let entries_on_disk =
        read_u64(&eocd64, 24).ok_or_else(|| malformed(ZipPreflightError::Zip64RecordsMalformed))?;
    let entries =
        read_u64(&eocd64, 32).ok_or_else(|| malformed(ZipPreflightError::Zip64RecordsMalformed))?;
    if disk_number != 0 || central_directory_disk != 0 || entries_on_disk != entries {
        return Err(malformed(ZipPreflightError::MultiDiskUnsupported));
    }

    let central_directory_size =
        read_u64(&eocd64, 40).ok_or_else(|| malformed(ZipPreflightError::Zip64RecordsMalformed))?;
    Ok(ZipPreflight {
        declared_entries: entries,
        declared_central_directory_size: central_directory_size,
    })
}

fn enforce_declared_preflight(preflight: ZipPreflight) -> Result<ZipPreflight, InspectionError> {
    if preflight.declared_entries > MAX_ENTRIES {
        return Err(InspectionError::EntryLimitExceeded { limit: MAX_ENTRIES });
    }
    if preflight.declared_central_directory_size > MAX_ZIP_CENTRAL_DIRECTORY_BYTES {
        return Err(InspectionError::ZipCentralDirectoryLimitExceeded {
            declared_size: preflight.declared_central_directory_size,
            limit: MAX_ZIP_CENTRAL_DIRECTORY_BYTES,
        });
    }
    Ok(preflight)
}

fn read_preflight_record(
    reader: &mut (impl Read + Seek),
    offset: u64,
    buffer: &mut [u8],
    malformed_reason: ZipPreflightError,
) -> Result<(), InspectionError> {
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(map_preflight_io)?;
    reader.read_exact(buffer).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            malformed(malformed_reason)
        } else {
            map_preflight_io(error)
        }
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let value: [u8; 2] = bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_le_bytes(value))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let value: [u8; 4] = bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_le_bytes(value))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let value: [u8; 8] = bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?;
    Some(u64::from_le_bytes(value))
}

fn malformed(reason: ZipPreflightError) -> InspectionError {
    InspectionError::ZipPreflightMalformed { reason }
}

fn map_preflight_io(error: io::Error) -> InspectionError {
    InspectionError::ZipIo {
        message: error.to_string(),
    }
}

/// Inspects ZIP or ZIP64 data without extraction or filesystem access.
///
/// Entry payloads are streamed through a fixed scratch buffer. Declared sizes
/// are retained only as display metadata; measured decoder input and output
/// enforce the cumulative compression-ratio policy.
///
/// # Errors
///
/// Returns [`InspectionError`] for malformed ZIP data, I/O failures,
/// unsupported compression methods, resource-limit violations, or decoder
/// completion failures.
pub fn inspect_zip<R: Read + Seek>(mut reader: R) -> Result<ArchiveInspection, InspectionError> {
    let preflight = preflight_zip(&mut reader)?;
    let (reader, input_counter) = CountingReader::new(reader);
    let mut archive =
        ZipArchive::new(reader).map_err(|error| map_zip_error(error, &input_counter))?;
    let parsed_entries = enforce_entry_count(archive.len())?;

    let mut builder = ArchiveInspectionBuilder::new(PackageFormat::Zip);
    if preflight.declared_entries != parsed_entries {
        builder.record_zip_entry_count_mismatch(preflight.declared_entries, parsed_entries);
    }
    let mut ratio = RatioAccounting::default();
    for index in 0..archive.len() {
        inspect_entry(
            &mut archive,
            index,
            &input_counter,
            &mut builder,
            &mut ratio,
        )?;
    }
    Ok(builder.finish())
}

fn inspect_entry<R: Read + Seek>(
    archive: &mut ZipArchive<CountingReader<R>>,
    index: usize,
    input_counter: &ReadCount,
    builder: &mut ArchiveInspectionBuilder,
    ratio: &mut RatioAccounting,
) -> Result<(), InspectionError> {
    let metadata = read_central_entry(archive, index, input_counter)?;
    record_entry(builder, &metadata)?;

    if !matches!(
        metadata.method,
        CompressionMethod::Stored | CompressionMethod::Deflated
    ) {
        return Err(InspectionError::UnsupportedCompressionMethod {
            entry: metadata.display_name,
            method: compression_method_id(metadata.method),
        });
    }

    let mut file = archive
        .by_index(index)
        .map_err(|error| map_zip_error(error, input_counter))?;
    let entry_input_start = input_counter.bytes();
    scan_payload(
        &mut file,
        input_counter,
        entry_input_start,
        matches!(metadata.kind, EntryKind::File),
        ratio,
    )
}

fn read_central_entry<R: Read + Seek>(
    archive: &mut ZipArchive<CountingReader<R>>,
    index: usize,
    input_counter: &ReadCount,
) -> Result<CentralEntry, InspectionError> {
    // Raw access accepts every method, allowing policy to report the numeric
    // method before the crate attempts to construct a decoder.
    let file = archive
        .by_index_raw(index)
        .map_err(|error| map_zip_error(error, input_counter))?;
    let zip_metadata = file.get_metadata();
    let raw_name = file.name_raw().to_vec();
    let unix_mode = file.unix_mode();
    let kind = entry_kind(zip_metadata.external_attributes, unix_mode);
    let decoded_name = decode_zip_name(&raw_name, zip_metadata.flags & EFS_FLAG != 0);
    let display_name = decoded_name.as_ref().map_or_else(
        || String::from_utf8_lossy(&raw_name).into_owned(),
        Clone::clone,
    );

    Ok(CentralEntry {
        raw_name,
        decoded_name,
        display_name,
        kind,
        declared_size: file.size(),
        unix_mode,
        method: file.compression(),
        efs: zip_metadata.flags & EFS_FLAG != 0,
    })
}

fn record_entry(
    builder: &mut ArchiveInspectionBuilder,
    metadata: &CentralEntry,
) -> Result<(), InspectionError> {
    let Some(decoded_name) = &metadata.decoded_name else {
        return builder.record_undecodable_path(
            String::from_utf8_lossy(&metadata.raw_name).into_owned(),
            metadata.raw_name.clone(),
        );
    };

    builder.record_decoded_entry(
        decoded_name,
        metadata.kind.clone(),
        metadata.declared_size,
        metadata.unix_mode,
    )?;
    if !metadata.efs {
        if let Ok(utf8) = std::str::from_utf8(&metadata.raw_name) {
            if utf8 != decoded_name {
                builder.record_warning(ArchiveWarning::ZipEfsUnsetUtf8Cp437Divergence {
                    bytes: metadata.raw_name.clone(),
                    utf8: utf8.to_owned(),
                    cp437: decoded_name.clone(),
                });
            }
        }
    }
    Ok(())
}

fn scan_payload(
    reader: &mut impl Read,
    input_counter: &ReadCount,
    input_start: u64,
    counts_for_ratio: bool,
    ratio: &mut RatioAccounting,
) -> Result<(), InspectionError> {
    let mut scratch = [0_u8; SCRATCH_BYTES];
    let mut previous_input = input_start;
    loop {
        let output = reader
            .read(&mut scratch)
            .map_err(|error| map_io_error(error, input_counter))?;
        let current_input = input_counter.bytes();
        let input = current_input
            .checked_sub(previous_input)
            .ok_or(InspectionError::MeasuredInputCounterRegressed)?;

        if counts_for_ratio {
            let output =
                u64::try_from(output).map_err(|_| InspectionError::MeasuredOutputOverflow)?;
            ratio.record(input, output)?;
        }
        previous_input = current_input;
        if output == 0 {
            return Ok(());
        }
    }
}

fn enforce_entry_count(count: usize) -> Result<u64, InspectionError> {
    let count = u64::try_from(count).map_err(|_| InspectionError::EntryCountOverflow)?;
    if count > MAX_ENTRIES {
        return Err(InspectionError::EntryLimitExceeded { limit: MAX_ENTRIES });
    }
    Ok(count)
}

pub(crate) fn decode_zip_name(raw_name: &[u8], efs: bool) -> Option<String> {
    if efs {
        std::str::from_utf8(raw_name).ok().map(str::to_owned)
    } else {
        Some(raw_name.iter().copied().map(cp437_char).collect())
    }
}

pub(crate) fn entry_kind(external_attributes: u32, unix_mode: Option<u32>) -> EntryKind {
    match unix_mode.map(|mode| mode & UNIX_FILE_TYPE_MASK) {
        Some(UNIX_DIRECTORY) => EntryKind::Directory,
        Some(UNIX_SYMLINK) => EntryKind::Symlink,
        Some(UNIX_REGULAR_FILE) => EntryKind::File,
        Some(UNIX_FIFO | UNIX_CHARACTER_DEVICE | UNIX_BLOCK_DEVICE | UNIX_SOCKET) => {
            EntryKind::Other
        }
        Some(0) | None if external_attributes & DOS_DIRECTORY_ATTRIBUTE != 0 => {
            EntryKind::Directory
        }
        Some(0) | None => EntryKind::File,
        Some(_) => EntryKind::Other,
    }
}

#[expect(
    deprecated,
    reason = "zip 8.6 exposes no non-deprecated numeric accessor for unknown methods"
)]
fn compression_method_id(method: CompressionMethod) -> u16 {
    method.to_u16()
}

fn map_zip_error(error: ZipError, counter: &ReadCount) -> InspectionError {
    if counter.overflowed() {
        return InspectionError::MeasuredInputOverflow;
    }
    match error {
        ZipError::Io(error) => InspectionError::ZipIo {
            message: error.to_string(),
        },
        other => InspectionError::ZipParser {
            message: other.to_string(),
        },
    }
}

fn map_io_error(error: io::Error, counter: &ReadCount) -> InspectionError {
    if counter.overflowed() {
        InspectionError::MeasuredInputOverflow
    } else {
        InspectionError::ZipIo {
            message: error.to_string(),
        }
    }
}

#[derive(Debug)]
struct CentralEntry {
    raw_name: Vec<u8>,
    decoded_name: Option<String>,
    display_name: String,
    kind: EntryKind,
    declared_size: u64,
    unix_mode: Option<u32>,
    method: CompressionMethod,
    efs: bool,
}

#[derive(Clone, Debug)]
struct ReadCount {
    bytes: Rc<Cell<u64>>,
    overflowed: Rc<Cell<bool>>,
}

impl ReadCount {
    fn bytes(&self) -> u64 {
        self.bytes.get()
    }

    fn overflowed(&self) -> bool {
        self.overflowed.get()
    }
}

#[derive(Debug)]
pub(crate) struct CountingReader<R> {
    inner: R,
    count: ReadCount,
}

impl<R> CountingReader<R> {
    fn new(inner: R) -> (Self, ReadCount) {
        let count = ReadCount {
            bytes: Rc::new(Cell::new(0)),
            overflowed: Rc::new(Cell::new(false)),
        };
        (
            Self {
                inner,
                count: count.clone(),
            },
            count,
        )
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        let read_u64 = u64::try_from(read).map_err(|_| {
            self.count.overflowed.set(true);
            io::Error::other("ZIP input byte count exceeded u64")
        })?;
        let bytes = self
            .count
            .bytes
            .get()
            .checked_add(read_u64)
            .ok_or_else(|| {
                self.count.overflowed.set(true);
                io::Error::other("ZIP input byte count exceeded u64")
            })?;
        self.count.bytes.set(bytes);
        Ok(read)
    }
}

impl<R: Seek> Seek for CountingReader<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

/// Shared measured-input and ratio policy for the writing ZIP pass.
pub(crate) struct ZipExtractionAccounting {
    input_count: ReadCount,
    previous_input: u64,
    ratio: RatioAccounting,
}

impl ZipExtractionAccounting {
    /// Starts measurement after ZIP has prepared the selected entry decoder.
    pub(crate) fn begin_regular_entry(&mut self) -> Result<(), InspectionError> {
        if self.input_count.overflowed() {
            return Err(InspectionError::MeasuredInputOverflow);
        }
        self.previous_input = self.input_count.bytes();
        Ok(())
    }

    /// Reads and accounts one decoded buffer before the caller writes it.
    pub(crate) fn read_regular_output(
        &mut self,
        reader: &mut impl Read,
        buffer: &mut [u8],
    ) -> Result<usize, InspectionError> {
        let output = reader
            .read(buffer)
            .map_err(|error| map_io_error(error, &self.input_count))?;
        self.record_output(output)?;
        Ok(output)
    }

    fn record_output(&mut self, output: usize) -> Result<(), InspectionError> {
        if self.input_count.overflowed() {
            return Err(InspectionError::MeasuredInputOverflow);
        }
        let current_input = self.input_count.bytes();
        let input = current_input
            .checked_sub(self.previous_input)
            .ok_or(InspectionError::MeasuredInputCounterRegressed)?;
        let output = u64::try_from(output).map_err(|_| InspectionError::MeasuredOutputOverflow)?;
        self.ratio.record(input, output)?;
        self.previous_input = current_input;
        Ok(())
    }
}

/// Builds the writing-pass ZIP reader with the same counter and entry cap used
/// by inspection. The returned archive's concrete reader remains crate-private.
pub(crate) fn extraction_archive<R: Read + Seek>(
    reader: R,
) -> Result<(ZipArchive<CountingReader<R>>, ZipExtractionAccounting), InspectionError> {
    let (reader, input_count) = CountingReader::new(reader);
    let archive = ZipArchive::new(reader).map_err(|error| map_zip_error(error, &input_count))?;
    enforce_entry_count(archive.len())?;
    let accounting = ZipExtractionAccounting {
        input_count,
        previous_input: 0,
        ratio: RatioAccounting::default(),
    };
    Ok((archive, accounting))
}

pub(crate) fn extraction_entry<'a, R: Read + Seek>(
    archive: &'a mut ZipArchive<CountingReader<R>>,
    index: usize,
    accounting: &ZipExtractionAccounting,
) -> Result<zip::read::ZipFile<'a, CountingReader<R>>, InspectionError> {
    archive
        .by_index(index)
        .map_err(|error| map_zip_error(error, &accounting.input_count))
}

// This table intentionally matches zip 8.6.0's private CP437 decoder.
#[expect(
    clippy::too_many_lines,
    reason = "CP437 requires an exhaustive byte mapping"
)]
fn cp437_char(input: u8) -> char {
    match input {
        0x00..=0x7f => char::from(input),
        0x80 => '\u{00c7}',
        0x81 => '\u{00fc}',
        0x82 => '\u{00e9}',
        0x83 => '\u{00e2}',
        0x84 => '\u{00e4}',
        0x85 => '\u{00e0}',
        0x86 => '\u{00e5}',
        0x87 => '\u{00e7}',
        0x88 => '\u{00ea}',
        0x89 => '\u{00eb}',
        0x8a => '\u{00e8}',
        0x8b => '\u{00ef}',
        0x8c => '\u{00ee}',
        0x8d => '\u{00ec}',
        0x8e => '\u{00c4}',
        0x8f => '\u{00c5}',
        0x90 => '\u{00c9}',
        0x91 => '\u{00e6}',
        0x92 => '\u{00c6}',
        0x93 => '\u{00f4}',
        0x94 => '\u{00f6}',
        0x95 => '\u{00f2}',
        0x96 => '\u{00fb}',
        0x97 => '\u{00f9}',
        0x98 => '\u{00ff}',
        0x99 => '\u{00d6}',
        0x9a => '\u{00dc}',
        0x9b => '\u{00a2}',
        0x9c => '\u{00a3}',
        0x9d => '\u{00a5}',
        0x9e => '\u{20a7}',
        0x9f => '\u{0192}',
        0xa0 => '\u{00e1}',
        0xa1 => '\u{00ed}',
        0xa2 => '\u{00f3}',
        0xa3 => '\u{00fa}',
        0xa4 => '\u{00f1}',
        0xa5 => '\u{00d1}',
        0xa6 => '\u{00aa}',
        0xa7 => '\u{00ba}',
        0xa8 => '\u{00bf}',
        0xa9 => '\u{2310}',
        0xaa => '\u{00ac}',
        0xab => '\u{00bd}',
        0xac => '\u{00bc}',
        0xad => '\u{00a1}',
        0xae => '\u{00ab}',
        0xaf => '\u{00bb}',
        0xb0 => '\u{2591}',
        0xb1 => '\u{2592}',
        0xb2 => '\u{2593}',
        0xb3 => '\u{2502}',
        0xb4 => '\u{2524}',
        0xb5 => '\u{2561}',
        0xb6 => '\u{2562}',
        0xb7 => '\u{2556}',
        0xb8 => '\u{2555}',
        0xb9 => '\u{2563}',
        0xba => '\u{2551}',
        0xbb => '\u{2557}',
        0xbc => '\u{255d}',
        0xbd => '\u{255c}',
        0xbe => '\u{255b}',
        0xbf => '\u{2510}',
        0xc0 => '\u{2514}',
        0xc1 => '\u{2534}',
        0xc2 => '\u{252c}',
        0xc3 => '\u{251c}',
        0xc4 => '\u{2500}',
        0xc5 => '\u{253c}',
        0xc6 => '\u{255e}',
        0xc7 => '\u{255f}',
        0xc8 => '\u{255a}',
        0xc9 => '\u{2554}',
        0xca => '\u{2569}',
        0xcb => '\u{2566}',
        0xcc => '\u{2560}',
        0xcd => '\u{2550}',
        0xce => '\u{256c}',
        0xcf => '\u{2567}',
        0xd0 => '\u{2568}',
        0xd1 => '\u{2564}',
        0xd2 => '\u{2565}',
        0xd3 => '\u{2559}',
        0xd4 => '\u{2558}',
        0xd5 => '\u{2552}',
        0xd6 => '\u{2553}',
        0xd7 => '\u{256b}',
        0xd8 => '\u{256a}',
        0xd9 => '\u{2518}',
        0xda => '\u{250c}',
        0xdb => '\u{2588}',
        0xdc => '\u{2584}',
        0xdd => '\u{258c}',
        0xde => '\u{2590}',
        0xdf => '\u{2580}',
        0xe0 => '\u{03b1}',
        0xe1 => '\u{00df}',
        0xe2 => '\u{0393}',
        0xe3 => '\u{03c0}',
        0xe4 => '\u{03a3}',
        0xe5 => '\u{03c3}',
        0xe6 => '\u{00b5}',
        0xe7 => '\u{03c4}',
        0xe8 => '\u{03a6}',
        0xe9 => '\u{0398}',
        0xea => '\u{03a9}',
        0xeb => '\u{03b4}',
        0xec => '\u{221e}',
        0xed => '\u{03c6}',
        0xee => '\u{03b5}',
        0xef => '\u{2229}',
        0xf0 => '\u{2261}',
        0xf1 => '\u{00b1}',
        0xf2 => '\u{2265}',
        0xf3 => '\u{2264}',
        0xf4 => '\u{2320}',
        0xf5 => '\u{2321}',
        0xf6 => '\u{00f7}',
        0xf7 => '\u{2248}',
        0xf8 => '\u{00b0}',
        0xf9 => '\u{2219}',
        0xfa => '\u{00b7}',
        0xfb => '\u{221a}',
        0xfc => '\u{207f}',
        0xfd => '\u{00b2}',
        0xfe => '\u{25a0}',
        0xff => '\u{00a0}',
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Seek, SeekFrom};

    use super::{
        entry_kind, preflight_zip, CountingReader, EntryKind, InspectionError, ZipPreflightError,
        DOS_DIRECTORY_ATTRIBUTE, EOCD_FIXED_BYTES, EOCD_SIGNATURE, MAX_ENTRIES,
        MAX_ZIP_CENTRAL_DIRECTORY_BYTES, UNIX_DIRECTORY, UNIX_REGULAR_FILE, UNIX_SYMLINK,
        ZIP64_EOCD_MIN_RECORD_SIZE, ZIP64_EOCD_SIGNATURE, ZIP64_LOCATOR_SIGNATURE,
    };

    fn classic_footer(entries: u16, central_directory_size: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(EOCD_FIXED_BYTES);
        bytes.extend_from_slice(&EOCD_SIGNATURE);
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&entries.to_le_bytes());
        bytes.extend_from_slice(&entries.to_le_bytes());
        bytes.extend_from_slice(&central_directory_size.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes
    }

    fn zip64_footer(entries: u64, central_directory_size: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&ZIP64_EOCD_SIGNATURE);
        bytes.extend_from_slice(&ZIP64_EOCD_MIN_RECORD_SIZE.to_le_bytes());
        bytes.extend_from_slice(&45_u16.to_le_bytes());
        bytes.extend_from_slice(&45_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&entries.to_le_bytes());
        bytes.extend_from_slice(&entries.to_le_bytes());
        bytes.extend_from_slice(&central_directory_size.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&ZIP64_LOCATOR_SIGNATURE);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&classic_footer(u16::MAX, u32::MAX));
        bytes
    }

    #[test]
    fn zip64_declared_entry_boundary_is_enforced_before_parser_construction() {
        let accepted = preflight_zip(&mut Cursor::new(zip64_footer(MAX_ENTRIES, 0)))
            .expect("entry boundary is accepted");
        assert_eq!(accepted.declared_entries, MAX_ENTRIES);

        assert_eq!(
            preflight_zip(&mut Cursor::new(zip64_footer(MAX_ENTRIES + 1, 0))),
            Err(InspectionError::EntryLimitExceeded { limit: MAX_ENTRIES })
        );
    }

    #[test]
    fn classic_central_directory_size_boundary_has_exact_typed_rejection() {
        let accepted = preflight_zip(&mut Cursor::new(classic_footer(
            0,
            MAX_ZIP_CENTRAL_DIRECTORY_BYTES as u32,
        )))
        .expect("central-directory boundary is accepted");
        assert_eq!(
            accepted.declared_central_directory_size,
            MAX_ZIP_CENTRAL_DIRECTORY_BYTES
        );

        assert_eq!(
            preflight_zip(&mut Cursor::new(classic_footer(
                0,
                MAX_ZIP_CENTRAL_DIRECTORY_BYTES as u32 + 1,
            ))),
            Err(InspectionError::ZipCentralDirectoryLimitExceeded {
                declared_size: MAX_ZIP_CENTRAL_DIRECTORY_BYTES + 1,
                limit: MAX_ZIP_CENTRAL_DIRECTORY_BYTES,
            })
        );
    }

    #[test]
    fn zip64_central_directory_size_boundary_is_enforced() {
        let accepted = preflight_zip(&mut Cursor::new(zip64_footer(
            MAX_ENTRIES,
            MAX_ZIP_CENTRAL_DIRECTORY_BYTES,
        )))
        .expect("ZIP64 boundaries are accepted");
        assert_eq!(accepted.declared_entries, MAX_ENTRIES);
        assert_eq!(
            accepted.declared_central_directory_size,
            MAX_ZIP_CENTRAL_DIRECTORY_BYTES
        );

        assert_eq!(
            preflight_zip(&mut Cursor::new(zip64_footer(
                MAX_ENTRIES,
                MAX_ZIP_CENTRAL_DIRECTORY_BYTES + 1,
            ))),
            Err(InspectionError::ZipCentralDirectoryLimitExceeded {
                declared_size: MAX_ZIP_CENTRAL_DIRECTORY_BYTES + 1,
                limit: MAX_ZIP_CENTRAL_DIRECTORY_BYTES,
            })
        );
    }

    #[test]
    fn missing_and_malformed_zip64_records_are_exact_failures() {
        assert_eq!(
            preflight_zip(&mut Cursor::new(classic_footer(u16::MAX, u32::MAX))),
            Err(InspectionError::ZipPreflightMalformed {
                reason: ZipPreflightError::Zip64LocatorMissing,
            })
        );

        let mut malformed_record = zip64_footer(MAX_ENTRIES, 0);
        malformed_record[0] = 0;
        assert_eq!(
            preflight_zip(&mut Cursor::new(malformed_record)),
            Err(InspectionError::ZipPreflightMalformed {
                reason: ZipPreflightError::Zip64RecordsMalformed,
            })
        );
    }

    #[test]
    fn counting_reader_counts_reads_but_not_seeks() {
        let (mut reader, count) = CountingReader::new(Cursor::new([1_u8, 2, 3, 4]));
        let mut bytes = [0_u8; 2];
        assert_eq!(reader.read(&mut bytes).expect("fixture read succeeds"), 2);
        assert_eq!(
            reader
                .seek(SeekFrom::Start(0))
                .expect("fixture seek succeeds"),
            0
        );
        assert_eq!(reader.read(&mut bytes).expect("fixture reread succeeds"), 2);
        assert_eq!(count.bytes(), 4);
    }

    #[test]
    fn kind_uses_type_bits_before_dos_directory_attribute() {
        assert_eq!(
            entry_kind(0, Some(UNIX_DIRECTORY | 0o755)),
            EntryKind::Directory
        );
        assert_eq!(
            entry_kind(0, Some(UNIX_SYMLINK | 0o777)),
            EntryKind::Symlink
        );
        assert_eq!(
            entry_kind(DOS_DIRECTORY_ATTRIBUTE, Some(UNIX_REGULAR_FILE | 0o644)),
            EntryKind::File
        );
        assert_eq!(
            entry_kind(DOS_DIRECTORY_ATTRIBUTE, None),
            EntryKind::Directory
        );
    }
}
