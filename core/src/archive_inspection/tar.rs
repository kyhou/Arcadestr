//! Bounded, streaming tar.zst and tar.gz inspection.

use std::{
    cell::Cell,
    fmt,
    io::{self, BufRead, BufReader, Read},
    rc::Rc,
};

use flate2::bufread::MultiGzDecoder;
use tar::{Archive, EntryType};
use zstd::stream::read::Decoder as ZstdDecoder;

use super::{
    ArchiveInspection, ArchiveInspectionBuilder, EntryCounter, EntryKind, InspectionError,
    RatioAccounting, MAX_TAR_METADATA_BYTES,
};
use crate::distribution::PackageFormat;

const SCRATCH_BYTES: usize = 32 * 1024;

const TAR_BLOCK_BYTES: u64 = 512;
// 128 MiB bounds decoder history while accommodating ordinary high-ratio frames.
pub(crate) const ZSTD_WINDOW_LOG_MAX: u32 = 27;

/// Inspects a gzip-compressed tar stream without extraction or filesystem access.
///
/// The complete gzip stream, including concatenated members, is consumed and
/// validated. Regular-file payloads are read through a fixed scratch buffer.
///
/// # Errors
///
/// Returns [`InspectionError`] for malformed tar data, decoder failures,
/// resource-limit violations, or source I/O failures.
pub fn inspect_tar_gz<R: Read>(reader: R) -> Result<ArchiveInspection, InspectionError> {
    let (reader, input_count) = ConsumedBufRead::new(BufReader::new(reader));
    let decoder = MultiGzDecoder::new(reader);
    inspect_tar_decoder(decoder, input_count, PackageFormat::TarGz)
}

/// Inspects a Zstandard-compressed tar stream without filesystem access.
///
/// Zstandard frames are decoded sequentially with a bounded 128 MiB maximum
/// back-reference window. Regular-file payloads use a fixed scratch buffer.
///
/// # Errors
///
/// Returns [`InspectionError`] for malformed tar data, decoder failures,
/// resource-limit violations, or source I/O failures.
pub fn inspect_tar_zstd<R: Read>(reader: R) -> Result<ArchiveInspection, InspectionError> {
    let (reader, input_count) = ConsumedBufRead::new(BufReader::new(reader));
    let mut decoder = ZstdDecoder::with_buffer(reader).map_err(tar_decoder_error)?;
    decoder
        .window_log_max(ZSTD_WINDOW_LOG_MAX)
        .map_err(tar_decoder_error)?;
    inspect_tar_decoder(decoder, input_count, PackageFormat::TarZstd)
}

fn inspect_tar_decoder<R: Read>(
    decoder: R,
    input_count: ConsumedCount,
    format: PackageFormat,
) -> Result<ArchiveInspection, InspectionError> {
    let output_accounting = TarOutputAccounting::default();
    let mut archive = Archive::new(DecoderReader::new(decoder, output_accounting.clone()));
    let mut builder = ArchiveInspectionBuilder::new(format);
    let mut ratio = RatioAccounting::default();
    let mut accounted_input = 0;

    {
        let entries = archive.entries().map_err(map_tar_io)?;
        for entry in entries {
            let mut entry = entry.map_err(map_tar_io)?;
            account_consumed_input(&input_count, &mut accounted_input, &mut ratio, 0)?;
            inspect_entry(
                &mut entry,
                &input_count,
                &mut accounted_input,
                &output_accounting,
                &mut builder,
                &mut ratio,
            )?;
        }
    }

    account_consumed_input(&input_count, &mut accounted_input, &mut ratio, 0)?;
    let mut decoder = archive.into_inner();
    drain_decoder(&mut decoder, &input_count, &mut accounted_input, &mut ratio)?;
    Ok(builder.finish())
}

/// Failure while policy-checking or emitting one regular tar payload.
#[derive(Debug)]
pub(crate) enum TarExtractionStreamError {
    Policy(InspectionError),
    Output(io::Error),
}

/// Shared tar decoder, entry-count, metadata, padding, and ratio accounting for
/// the writing extraction pass.
pub(crate) struct TarExtractionAccounting {
    input_count: ConsumedCount,
    output_accounting: TarOutputAccounting,
    ratio: RatioAccounting,
    accounted_input: u64,
    entries: EntryCounter,
}

impl TarExtractionAccounting {
    fn new(input_count: ConsumedCount, output_accounting: TarOutputAccounting) -> Self {
        Self {
            input_count,
            output_accounting,
            ratio: RatioAccounting::default(),
            accounted_input: 0,
            entries: EntryCounter::default(),
        }
    }

    /// Accounts one parsed entry and compressed metadata before it is handled.
    pub(crate) fn begin_entry(&mut self) -> Result<(), InspectionError> {
        self.entries.record_entry()?;
        account_consumed_input(
            &self.input_count,
            &mut self.accounted_input,
            &mut self.ratio,
            0,
        )
    }

    /// Streams one regular payload, checking the ratio before each output
    /// callback so rejected bytes never reach the filesystem.
    pub(crate) fn stream_regular_payload<R: Read>(
        &mut self,
        entry: &mut tar::Entry<'_, R>,
        mut emit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> Result<(), TarExtractionStreamError> {
        let _payload_mode = self.output_accounting.enter_payload_mode();
        let mut measured_payload = 0_u64;
        let mut scratch = [0_u8; SCRATCH_BYTES];
        loop {
            let output = entry
                .read(&mut scratch)
                .map_err(map_tar_io)
                .map_err(TarExtractionStreamError::Policy)?;
            let output_u64 = u64::try_from(output)
                .map_err(|_| InspectionError::MeasuredOutputOverflow)
                .map_err(TarExtractionStreamError::Policy)?;
            measured_payload = measured_payload
                .checked_add(output_u64)
                .ok_or(InspectionError::MeasuredOutputOverflow)
                .map_err(TarExtractionStreamError::Policy)?;
            account_consumed_input(
                &self.input_count,
                &mut self.accounted_input,
                &mut self.ratio,
                output_u64,
            )
            .map_err(TarExtractionStreamError::Policy)?;
            if output == 0 {
                self.output_accounting
                    .schedule_regular_padding(measured_payload)
                    .map_err(TarExtractionStreamError::Policy)?;
                return Ok(());
            }
            emit(&scratch[..output]).map_err(TarExtractionStreamError::Output)?;
        }
    }

    /// Accounts trailing metadata and drains the decoder to validate its end.
    pub(crate) fn finish(mut self, reader: &mut impl Read) -> Result<(), InspectionError> {
        account_consumed_input(
            &self.input_count,
            &mut self.accounted_input,
            &mut self.ratio,
            0,
        )?;
        drain_decoder(
            reader,
            &self.input_count,
            &mut self.accounted_input,
            &mut self.ratio,
        )
    }
}

/// Builds a gzip tar reader with the exact accounting layers used by inspection.
pub(crate) fn extraction_tar_gz_reader<R: Read>(reader: R) -> (impl Read, TarExtractionAccounting) {
    let (reader, input_count) = ConsumedBufRead::new(BufReader::new(reader));
    let decoder = MultiGzDecoder::new(reader);
    let output_accounting = TarOutputAccounting::default();
    let accounting = TarExtractionAccounting::new(input_count, output_accounting.clone());
    (DecoderReader::new(decoder, output_accounting), accounting)
}

/// Builds a bounded-window Zstandard tar reader with inspection-identical accounting.
pub(crate) fn extraction_tar_zstd_reader<R: Read>(
    reader: R,
) -> Result<(impl Read, TarExtractionAccounting), InspectionError> {
    let (reader, input_count) = ConsumedBufRead::new(BufReader::new(reader));
    let mut decoder = ZstdDecoder::with_buffer(reader).map_err(tar_decoder_error)?;
    decoder
        .window_log_max(ZSTD_WINDOW_LOG_MAX)
        .map_err(tar_decoder_error)?;
    let output_accounting = TarOutputAccounting::default();
    let accounting = TarExtractionAccounting::new(input_count, output_accounting.clone());
    Ok((DecoderReader::new(decoder, output_accounting), accounting))
}

pub(crate) fn map_extraction_io(error: io::Error) -> InspectionError {
    map_tar_io(error)
}

fn inspect_entry<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    input_count: &ConsumedCount,
    accounted_input: &mut u64,
    output_accounting: &TarOutputAccounting,
    builder: &mut ArchiveInspectionBuilder,
    ratio: &mut RatioAccounting,
) -> Result<(), InspectionError> {
    let raw_typeflag = entry.header().entry_type().as_byte();
    let kind = entry_kind(raw_typeflag);
    let declared_size = entry.size();
    let unix_mode = entry.header().mode().map_err(map_tar_io).map(Some)?;
    let path_bytes = entry.path_bytes();

    match std::str::from_utf8(path_bytes.as_ref()) {
        Ok(path) => builder.record_decoded_entry_with_other_kind(
            path,
            kind.clone(),
            declared_size,
            unix_mode,
            matches!(kind, EntryKind::Other).then(|| other_kind_diagnostic(path, raw_typeflag)),
        )?,
        Err(_) => builder.record_undecodable_path(
            String::from_utf8_lossy(path_bytes.as_ref()).into_owned(),
            path_bytes.into_owned(),
        )?,
    }

    if matches!(kind, EntryKind::File) {
        scan_regular_payload(
            entry,
            input_count,
            accounted_input,
            output_accounting,
            ratio,
        )?;
    }
    Ok(())
}

fn scan_regular_payload(
    reader: &mut impl Read,
    input_count: &ConsumedCount,
    accounted_input: &mut u64,
    output_accounting: &TarOutputAccounting,
    ratio: &mut RatioAccounting,
) -> Result<(), InspectionError> {
    let _payload_mode = output_accounting.enter_payload_mode();
    let mut measured_payload = 0_u64;
    let mut scratch = [0_u8; SCRATCH_BYTES];
    loop {
        let output = reader.read(&mut scratch).map_err(map_tar_io)?;
        let output = u64::try_from(output).map_err(|_| InspectionError::MeasuredOutputOverflow)?;
        measured_payload = measured_payload
            .checked_add(output)
            .ok_or(InspectionError::MeasuredOutputOverflow)?;
        account_consumed_input(input_count, accounted_input, ratio, output)?;
        if output == 0 {
            output_accounting.schedule_regular_padding(measured_payload)?;
            return Ok(());
        }
    }
}

fn drain_decoder(
    reader: &mut impl Read,
    input_count: &ConsumedCount,
    accounted_input: &mut u64,
    ratio: &mut RatioAccounting,
) -> Result<(), InspectionError> {
    let mut scratch = [0_u8; SCRATCH_BYTES];
    loop {
        let read = reader.read(&mut scratch).map_err(map_tar_io)?;
        account_consumed_input(input_count, accounted_input, ratio, 0)?;
        if read == 0 {
            return Ok(());
        }
    }
}

fn account_consumed_input(
    count: &ConsumedCount,
    accounted_input: &mut u64,
    ratio: &mut RatioAccounting,
    output: u64,
) -> Result<(), InspectionError> {
    if count.overflowed() {
        return Err(InspectionError::MeasuredInputOverflow);
    }
    let current = count.bytes();
    let input = current
        .checked_sub(*accounted_input)
        .ok_or(InspectionError::MeasuredInputCounterRegressed)?;
    ratio
        .record(input, output)
        .map_err(|error| map_tar_io(io::Error::other(error)))?;
    *accounted_input = current;
    Ok(())
}

pub(crate) fn entry_kind(typeflag: u8) -> EntryKind {
    match EntryType::new(typeflag) {
        EntryType::Regular => EntryKind::File,
        EntryType::Directory => EntryKind::Directory,
        EntryType::Symlink => EntryKind::Symlink,
        EntryType::Link => EntryKind::HardLink,
        // The amendment permits only regular files and directories. GNU sparse,
        // contiguous, devices, FIFOs, extension records, and unknown flags block.
        _ => EntryKind::Other,
    }
}

fn other_kind_diagnostic(path: &str, typeflag: u8) -> String {
    let display = if typeflag.is_ascii_graphic() {
        format!("'{}'", char::from(typeflag))
    } else {
        format!("0x{typeflag:02x}")
    };
    format!("{path}: unsupported tar typeflag {display}")
}

fn tar_decoder_error(error: io::Error) -> InspectionError {
    InspectionError::TarDecoder {
        message: error.to_string(),
    }
}

fn map_tar_io(error: io::Error) -> InspectionError {
    if let Some(error) = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<InspectionError>())
    {
        return error.clone();
    }
    if let Some(error) = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<DecoderFailure>())
    {
        return InspectionError::TarDecoder {
            message: error.to_string(),
        };
    }
    InspectionError::TarParser {
        message: error.to_string(),
    }
}

#[derive(Debug)]
struct DecoderFailure(io::Error);

impl fmt::Display for DecoderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for DecoderFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TarOutputMode {
    #[default]
    Metadata,
    RegularPayload,
}

#[derive(Debug, Default)]
struct TarOutputState {
    mode: Cell<TarOutputMode>,
    metadata_bytes: Cell<u64>,
    regular_payload_bytes: Cell<u64>,
    payload_padding_bytes: Cell<u64>,
    pending_regular_padding: Cell<u64>,
}

#[derive(Clone, Debug, Default)]
struct TarOutputAccounting {
    state: Rc<TarOutputState>,
}

impl TarOutputAccounting {
    fn record_bytes(&self, bytes: u64) -> Result<(), InspectionError> {
        match self.state.mode.get() {
            TarOutputMode::Metadata => self.record_metadata_mode_bytes(bytes),
            TarOutputMode::RegularPayload => {
                let measured_payload = self
                    .state
                    .regular_payload_bytes
                    .get()
                    .checked_add(bytes)
                    .ok_or(InspectionError::MeasuredOutputOverflow)?;
                self.state.regular_payload_bytes.set(measured_payload);
                Ok(())
            }
        }
    }

    fn record_metadata_mode_bytes(&self, bytes: u64) -> Result<(), InspectionError> {
        let pending = self.state.pending_regular_padding.get();
        let padding = pending.min(bytes);
        let metadata = bytes - padding;
        let measured_padding = self
            .state
            .payload_padding_bytes
            .get()
            .checked_add(padding)
            .ok_or(InspectionError::MeasuredOutputOverflow)?;
        let measured_metadata = self
            .state
            .metadata_bytes
            .get()
            .checked_add(metadata)
            .ok_or(InspectionError::MeasuredOutputOverflow)?;
        if measured_metadata > MAX_TAR_METADATA_BYTES {
            return Err(InspectionError::TarMetadataLimitExceeded {
                measured_metadata,
                limit: MAX_TAR_METADATA_BYTES,
            });
        }

        self.state.pending_regular_padding.set(pending - padding);
        self.state.payload_padding_bytes.set(measured_padding);
        self.state.metadata_bytes.set(measured_metadata);
        Ok(())
    }

    fn schedule_regular_padding(&self, measured_payload: u64) -> Result<(), InspectionError> {
        let remainder = measured_payload % TAR_BLOCK_BYTES;
        let padding = if remainder == 0 {
            0
        } else {
            TAR_BLOCK_BYTES - remainder
        };
        let pending = self
            .state
            .pending_regular_padding
            .get()
            .checked_add(padding)
            .ok_or(InspectionError::MeasuredOutputOverflow)?;
        self.state.pending_regular_padding.set(pending);
        Ok(())
    }

    fn enter_payload_mode(&self) -> PayloadModeGuard {
        let previous = self.state.mode.replace(TarOutputMode::RegularPayload);
        PayloadModeGuard {
            accounting: self.clone(),
            previous,
        }
    }

    #[cfg(test)]
    fn metadata_bytes(&self) -> u64 {
        self.state.metadata_bytes.get()
    }

    #[cfg(test)]
    fn regular_payload_bytes(&self) -> u64 {
        self.state.regular_payload_bytes.get()
    }

    #[cfg(test)]
    fn payload_padding_bytes(&self) -> u64 {
        self.state.payload_padding_bytes.get()
    }

    #[cfg(test)]
    fn pending_regular_padding(&self) -> u64 {
        self.state.pending_regular_padding.get()
    }

    #[cfg(test)]
    fn mode(&self) -> TarOutputMode {
        self.state.mode.get()
    }
}

#[derive(Debug)]
struct PayloadModeGuard {
    accounting: TarOutputAccounting,
    previous: TarOutputMode,
}

impl Drop for PayloadModeGuard {
    fn drop(&mut self) {
        self.accounting.state.mode.set(self.previous);
    }
}

struct DecoderReader<R> {
    inner: R,
    output_accounting: TarOutputAccounting,
}

impl<R> DecoderReader<R> {
    fn new(inner: R, output_accounting: TarOutputAccounting) -> Self {
        Self {
            inner,
            output_accounting,
        }
    }
}

impl<R: Read> Read for DecoderReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer).map_err(|error| {
            if error
                .get_ref()
                .is_some_and(|source| source.is::<InspectionError>())
            {
                error
            } else {
                io::Error::new(error.kind(), DecoderFailure(error))
            }
        })?;
        let bytes = u64::try_from(read)
            .map_err(|_| io::Error::other(InspectionError::MeasuredOutputOverflow))?;
        self.output_accounting
            .record_bytes(bytes)
            .map_err(io::Error::other)?;
        Ok(read)
    }
}

#[derive(Clone, Debug)]
struct ConsumedCount {
    bytes: Rc<Cell<u64>>,
    overflowed: Rc<Cell<bool>>,
}

impl ConsumedCount {
    fn bytes(&self) -> u64 {
        self.bytes.get()
    }

    fn overflowed(&self) -> bool {
        self.overflowed.get()
    }
}

#[derive(Debug)]
struct ConsumedBufRead<R> {
    inner: R,
    count: ConsumedCount,
}

impl<R> ConsumedBufRead<R> {
    fn new(inner: R) -> (Self, ConsumedCount) {
        let count = ConsumedCount {
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

impl<R: BufRead> Read for ConsumedBufRead<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let read = available.len().min(buffer.len());
        buffer[..read].copy_from_slice(&available[..read]);
        self.consume(read);
        Ok(read)
    }
}

impl<R: BufRead> BufRead for ConsumedBufRead<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.count.overflowed() {
            return Err(io::Error::other(InspectionError::MeasuredInputOverflow));
        }
        self.inner.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        self.inner.consume(amount);
        let Ok(amount) = u64::try_from(amount) else {
            self.count.overflowed.set(true);
            return;
        };
        let Some(bytes) = self.count.bytes().checked_add(amount) else {
            self.count.overflowed.set(true);
            return;
        };
        self.count.bytes.set(bytes);
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, Cursor};

    use super::{
        entry_kind, ConsumedBufRead, EntryKind, InspectionError, RatioAccounting,
        TarOutputAccounting, TarOutputMode, MAX_TAR_METADATA_BYTES,
    };

    #[test]
    fn compressed_count_advances_on_consume_not_fill() {
        let (mut reader, count) = ConsumedBufRead::new(Cursor::new([1_u8, 2, 3]));
        assert_eq!(reader.fill_buf().expect("fixture fill succeeds").len(), 3);
        assert_eq!(count.bytes(), 0);
        reader.consume(2);
        assert_eq!(count.bytes(), 2);
    }

    #[test]
    fn only_standard_regular_typeflag_is_a_file() {
        assert_eq!(entry_kind(b'0'), EntryKind::File);
        assert_eq!(entry_kind(0), EntryKind::File);
        assert_eq!(entry_kind(b'7'), EntryKind::Other);
        assert_eq!(entry_kind(b'S'), EntryKind::Other);
    }

    #[test]
    fn metadata_limit_accepts_128_mib_and_rejects_next_byte() {
        let accounting = TarOutputAccounting::default();
        assert_eq!(accounting.mode(), TarOutputMode::Metadata);
        accounting
            .record_bytes(MAX_TAR_METADATA_BYTES)
            .expect("the exact metadata cap is accepted");

        assert_eq!(accounting.metadata_bytes(), MAX_TAR_METADATA_BYTES);
        assert_eq!(
            accounting.record_bytes(1),
            Err(InspectionError::TarMetadataLimitExceeded {
                measured_metadata: MAX_TAR_METADATA_BYTES + 1,
                limit: MAX_TAR_METADATA_BYTES,
            })
        );
    }

    #[test]
    fn metadata_mode_covers_headers_extensions_end_blocks_and_nonregular_bodies() {
        let accounting = TarOutputAccounting::default();
        let ordinary_header = 512;
        let gnu_long_name_record = 1_024;
        let pax_record = 1_536;
        let end_blocks = 1_024;
        let nonregular_body = 37;

        for bytes in [
            ordinary_header,
            gnu_long_name_record,
            pax_record,
            end_blocks,
            nonregular_body,
        ] {
            accounting
                .record_bytes(bytes)
                .expect("metadata class fits the cap");
        }

        assert_eq!(
            accounting.metadata_bytes(),
            ordinary_header + gnu_long_name_record + pax_record + end_blocks + nonregular_body
        );
    }

    #[test]
    fn padding_and_following_header_are_split_without_changing_ratio_output() {
        let accounting = TarOutputAccounting::default();
        let mut ratio = RatioAccounting::default();
        let measured_payload = 513;
        {
            let _payload_mode = accounting.enter_payload_mode();
            accounting
                .record_bytes(measured_payload)
                .expect("regular payload fits accounting");
            ratio
                .record(100, measured_payload)
                .expect("fixture ratio remains below enforcement threshold");
            accounting
                .schedule_regular_padding(measured_payload)
                .expect("padding schedule fits accounting");
        }

        accounting
            .record_bytes(511 + 512)
            .expect("one read can span pending padding and the next header");

        assert_eq!(accounting.regular_payload_bytes(), measured_payload);
        assert_eq!(accounting.payload_padding_bytes(), 511);
        assert_eq!(accounting.pending_regular_padding(), 0);
        assert_eq!(accounting.metadata_bytes(), 512);
        assert_eq!(ratio.measured_output(), measured_payload);
    }

    #[test]
    fn arbitrary_regular_payload_does_not_consume_metadata_budget() {
        let accounting = TarOutputAccounting::default();
        {
            let _payload_mode = accounting.enter_payload_mode();
            accounting
                .record_bytes(MAX_TAR_METADATA_BYTES + 1)
                .expect("regular payload is not metadata");
        }

        assert_eq!(accounting.metadata_bytes(), 0);
        assert_eq!(
            accounting.regular_payload_bytes(),
            MAX_TAR_METADATA_BYTES + 1
        );
    }

    #[test]
    fn payload_mode_restores_after_accounting_error() {
        fn fail_in_payload(accounting: &TarOutputAccounting) -> Result<(), InspectionError> {
            let _payload_mode = accounting.enter_payload_mode();
            accounting.record_bytes(u64::MAX)?;
            accounting.record_bytes(1)
        }

        let accounting = TarOutputAccounting::default();
        assert_eq!(
            fail_in_payload(&accounting),
            Err(InspectionError::MeasuredOutputOverflow)
        );
        assert_eq!(accounting.mode(), TarOutputMode::Metadata);
    }
}
