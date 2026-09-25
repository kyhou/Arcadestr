#![cfg(feature = "native")]

use std::io::{self, Cursor, Read, Write};

use arcadestr_core::{
    archive_inspection::{
        inspect_tar_gz, inspect_tar_zstd, EntryKind, InspectionError, UndecodablePath,
        MAX_COMPRESSION_RATIO, MAX_ENTRIES, RATIO_ENFORCEMENT_OUTPUT_BYTES,
    },
    distribution::PathError,
};
use flate2::{write::GzEncoder, Compression};
use tar::{Builder, EntryType, Header};

const TAR_BLOCK_BYTES: usize = 512;
const TAR_NAME_BYTES: usize = 100;
const TAR_CHECKSUM_START: usize = 148;
const TAR_CHECKSUM_END: usize = 156;
const STREAM_CHUNK_BYTES: usize = 32 * 1024;

fn header(entry_type: EntryType, size: u64) -> Header {
    let mut header = Header::new_gnu();
    header.set_entry_type(entry_type);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_size(size);
    header.set_cksum();
    header
}

fn append_empty<W: Write>(
    builder: &mut Builder<W>,
    path: &str,
    entry_type: EntryType,
) -> io::Result<()> {
    let mut header = header(entry_type, 0);
    builder.append_data(&mut header, path, io::empty())
}

fn generated_tar(build: impl FnOnce(&mut Builder<Vec<u8>>) -> io::Result<()>) -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());
    build(&mut builder).expect("generate tar entries in memory");
    builder.finish().expect("finish generated tar");
    builder.into_inner().expect("recover generated tar bytes")
}

fn gzip_bytes(bytes: &[u8], compression: Compression) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), compression);
    encoder.write_all(bytes).expect("stream bytes into gzip");
    encoder.finish().expect("finish gzip stream")
}

fn zstd_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3)
        .expect("create in-memory Zstandard encoder");
    encoder
        .write_all(bytes)
        .expect("stream bytes into Zstandard");
    encoder.finish().expect("finish Zstandard stream")
}

fn generated_tar_gz(
    compression: Compression,
    build: impl FnOnce(&mut Builder<GzEncoder<Vec<u8>>>) -> io::Result<()>,
) -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), compression);
    let mut builder = Builder::new(encoder);
    build(&mut builder).expect("stream generated tar into gzip");
    builder.finish().expect("finish tar stream");
    builder
        .into_inner()
        .expect("recover gzip encoder")
        .finish()
        .expect("finish generated gzip")
}

fn patch_header_name(raw_tar: &mut [u8], placeholder: &[u8], replacement: &[u8]) {
    assert_eq!(placeholder.len(), replacement.len());
    assert!(placeholder.len() <= TAR_NAME_BYTES);
    let matches = raw_tar
        .chunks_exact(TAR_BLOCK_BYTES)
        .enumerate()
        .filter_map(|(index, block)| block.starts_with(placeholder).then_some(index))
        .collect::<Vec<_>>();
    assert_eq!(
        matches.len(),
        1,
        "one raw tar header must match the placeholder"
    );
    let header_start = matches[0] * TAR_BLOCK_BYTES;
    let header_end = header_start + TAR_BLOCK_BYTES;
    let header = &mut raw_tar[header_start..header_end];

    header[..replacement.len()].copy_from_slice(replacement);
    header[TAR_CHECKSUM_START..TAR_CHECKSUM_END].fill(b' ');
    let checksum = header.iter().map(|byte| u32::from(*byte)).sum::<u32>();
    let encoded = format!("{checksum:06o}\0 ");
    assert_eq!(encoded.len(), TAR_CHECKSUM_END - TAR_CHECKSUM_START);
    header[TAR_CHECKSUM_START..TAR_CHECKSUM_END].copy_from_slice(encoded.as_bytes());
}

fn nested_tar() -> Vec<u8> {
    let mut raw_tar = generated_tar(|builder| {
        append_empty(builder, "game/", EntryType::Directory)?;
        append_empty(builder, "game/assets/", EntryType::Directory)?;
        append_empty(builder, "game/data-x", EntryType::Directory)?;

        let mut executable = header(EntryType::Regular, 4);
        builder.append_data(&mut executable, "game/bin/game.exe", Cursor::new(b"play"))?;
        let mut readme = header(EntryType::Regular, 6);
        builder.append_data(&mut readme, "game/docs/readme.txt", Cursor::new(b"manual"))?;
        Ok(())
    });
    // `Path::components` collapses repeated separators, so patch this hostile raw name narrowly.
    patch_header_name(&mut raw_tar, b"game/data-x", b"game/data//");
    raw_tar
}

#[derive(Debug)]
struct ErrorAfterBytes {
    bytes: Cursor<Vec<u8>>,
}

impl Read for ErrorAfterBytes {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.bytes.read(buffer)?;
        if read == 0 {
            Err(io::Error::other("sentinel compressed-source failure"))
        } else {
            Ok(read)
        }
    }
}

#[test]
fn generated_hazard_tar_indexes_invalid_path_and_reports_all_blocking_kinds() {
    // Arrange: the symbolic link has an escaping target even though every symlink blocks.
    let archive = generated_tar_gz(Compression::default(), |builder| {
        append_empty(builder, "CON.txt", EntryType::Regular)?;
        append_empty(builder, "Game.exe", EntryType::Regular)?;
        append_empty(builder, "game.exe", EntryType::Regular)?;

        let mut symlink = header(EntryType::Symlink, 0);
        symlink.set_link_name("../../outside")?;
        builder.append_data(&mut symlink, "escape-link", io::empty())?;

        let mut hardlink = header(EntryType::Link, 0);
        hardlink.set_link_name("Game.exe")?;
        builder.append_data(&mut hardlink, "game-hardlink", io::empty())?;
        append_empty(builder, "named-pipe", EntryType::Fifo)?;
        Ok(())
    });

    // Act.
    let inspection =
        inspect_tar_gz(Cursor::new(archive)).expect("tar hazards must remain reportable");

    // Assert.
    assert_eq!(
        inspection
            .index
            .entries
            .iter()
            .map(|entry| (entry.raw_path.as_str(), entry.kind.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("CON.txt", EntryKind::File),
            ("Game.exe", EntryKind::File),
            ("game.exe", EntryKind::File),
            ("escape-link", EntryKind::Symlink),
            ("game-hardlink", EntryKind::HardLink),
            ("named-pipe", EntryKind::Other),
        ]
    );
    assert_eq!(
        inspection.hazards.invalid_paths,
        vec![("CON.txt".into(), PathError::Rule8ReservedDeviceName)]
    );
    assert_eq!(inspection.hazards.symlinks, vec!["escape-link"]);
    assert_eq!(inspection.hazards.hard_links, vec!["game-hardlink"]);
    assert_eq!(
        inspection.hazards.other_entry_kinds,
        vec!["named-pipe: unsupported tar typeflag '6'"]
    );
    assert_eq!(
        inspection.hazards.case_collisions,
        vec![vec![String::from("Game.exe"), String::from("game.exe")]]
    );
}

#[test]
fn undecodable_tar_path_is_diagnostic_only_and_cannot_collide() {
    // Arrange: patch only the first raw name and then restore its tar checksum.
    let placeholder = b"bad-x.txt";
    let raw_name = b"bad-\xff.txt";
    let mut raw_tar = generated_tar(|builder| {
        append_empty(builder, "bad-x.txt", EntryType::Regular)?;
        append_empty(builder, "bad-�.txt", EntryType::Regular)?;
        Ok(())
    });
    patch_header_name(&mut raw_tar, placeholder, raw_name);
    let archive = gzip_bytes(&raw_tar, Compression::default());

    // Act.
    let inspection = inspect_tar_gz(Cursor::new(archive)).expect("invalid UTF-8 is a path hazard");

    // Assert: lossy text is display-only and therefore cannot collide with the valid entry.
    assert_eq!(
        inspection.hazards.undecodable_paths,
        vec![UndecodablePath {
            lossy: "bad-�.txt".into(),
            bytes: raw_name.to_vec(),
        }]
    );
    assert_eq!(inspection.index.entries.len(), 1);
    assert_eq!(inspection.index.entries[0].raw_path, "bad-�.txt");
    assert!(inspection.hazards.case_collisions.is_empty());
}

#[test]
fn nested_tree_indexes_identically_through_tar_gz_and_tar_zst() {
    // Arrange.
    let raw_tar = nested_tar();
    let archives = [
        ("tar.gz", gzip_bytes(&raw_tar, Compression::default())),
        ("tar.zst", zstd_bytes(&raw_tar)),
    ];

    // Act.
    let inspections = archives
        .into_iter()
        .map(|(format, bytes)| {
            let inspection = match format {
                "tar.gz" => inspect_tar_gz(Cursor::new(bytes)),
                "tar.zst" => inspect_tar_zstd(Cursor::new(bytes)),
                _ => unreachable!("fixed test formats"),
            };
            inspection.expect("nested tar must inspect through both decoders")
        })
        .collect::<Vec<_>>();

    // Assert: directories lose one slash only; explicit and implied directories are retained.
    for inspection in inspections {
        assert_eq!(
            inspection
                .index
                .entries
                .iter()
                .map(|entry| (
                    entry.raw_path.as_str(),
                    entry.kind.clone(),
                    entry.declared_size,
                ))
                .collect::<Vec<_>>(),
            vec![
                ("game", EntryKind::Directory, 0_u64),
                ("game/assets", EntryKind::Directory, 0_u64),
                ("game/data/", EntryKind::Directory, 0_u64),
                ("game/bin/game.exe", EntryKind::File, 4_u64),
                ("game/docs/readme.txt", EntryKind::File, 6_u64),
            ]
        );
        assert_eq!(
            inspection.hazards.invalid_paths,
            vec![("game/data/".into(), PathError::Rule6EmptyOrDotSegment)]
        );
        assert_eq!(
            inspection.index.working_directories(),
            vec![".", "game", "game/assets", "game/bin", "game/docs"]
        );
    }
}

#[test]
fn tar_backslash_name_is_not_rewritten_before_a2_rule3_validation() {
    // Arrange.
    let archive = generated_tar_gz(Compression::default(), |builder| {
        append_empty(builder, "bin\\game.exe", EntryType::Regular)
    });

    // Act.
    let inspection = inspect_tar_gz(Cursor::new(archive)).expect("backslash is a path hazard");

    // Assert.
    assert_eq!(inspection.index.entries[0].raw_path, "bin\\game.exe");
    assert_eq!(
        inspection.hazards.invalid_paths,
        vec![("bin\\game.exe".into(), PathError::Rule3Backslash)]
    );
}

#[test]
fn streaming_tar_entry_cap_accepts_200000_and_rejects_200001_exactly() {
    // Arrange: short names prevent GNU/PAX extension records from changing logical entry counts.
    let accepted = generated_tar_gz(Compression::best(), |builder| {
        for index in 0..MAX_ENTRIES {
            append_empty(builder, &format!("e{index:06}"), EntryType::Regular)?;
        }
        Ok(())
    });

    // Act (positive boundary).
    let inspection = inspect_tar_gz(Cursor::new(accepted)).expect("the inclusive cap must pass");

    // Assert (positive boundary).
    assert_eq!(inspection.index.entries.len(), 200_000);
    drop(inspection);

    // Arrange (negative boundary): compress headers as generated instead of retaining raw tar.
    let rejected = generated_tar_gz(Compression::best(), |builder| {
        for index in 0..=MAX_ENTRIES {
            append_empty(builder, &format!("e{index:06}"), EntryType::Regular)?;
        }
        Ok(())
    });

    // Act (negative boundary).
    let error = inspect_tar_gz(Cursor::new(rejected)).expect_err("entry 200,001 must abort");

    // Assert (negative boundary).
    assert_eq!(
        error,
        InspectionError::EntryLimitExceeded { limit: 200_000 }
    );
}

#[test]
fn gzip_ratio_bomb_aborts_mid_stream_with_exact_measured_ratio_error() {
    // Arrange: write real regular-file output in chunks without materializing the payload.
    let payload_size = RATIO_ENFORCEMENT_OUTPUT_BYTES
        + u64::try_from(STREAM_CHUNK_BYTES).expect("chunk length") * 2;
    let archive = generated_tar_gz(Compression::best(), |builder| {
        let mut payload = io::repeat(0).take(payload_size);
        let mut payload_header = header(EntryType::Regular, payload_size);
        builder.append_data(&mut payload_header, "ratio.bin", &mut payload)
    });

    // Act.
    let error = inspect_tar_gz(Cursor::new(archive))
        .expect_err("measured output over 64 MiB must exceed 200:1");

    // Assert.
    let (measured_input, measured_output, maximum_ratio) = match error {
        InspectionError::CompressionRatioExceeded {
            measured_input,
            measured_output,
            maximum_ratio,
        } => (measured_input, measured_output, maximum_ratio),
        other => panic!("expected exact CompressionRatioExceeded variant, got {other:?}"),
    };
    assert_eq!(maximum_ratio, MAX_COMPRESSION_RATIO);
    assert!(measured_output > RATIO_ENFORCEMENT_OUTPUT_BYTES);
    assert!(measured_output < payload_size);
    assert!(measured_input > 0);
    assert!(measured_output > measured_input * maximum_ratio);
}

#[test]
fn valid_large_declared_size_is_indexed_metadata_not_an_absolute_cap() {
    // Arrange: no-compression gzip keeps the measured ratio near 1:1 for a valid >64 MiB file.
    let payload_size = RATIO_ENFORCEMENT_OUTPUT_BYTES + 1;
    let archive = generated_tar_gz(Compression::none(), |builder| {
        let mut payload = io::repeat(0).take(payload_size);
        let mut payload_header = header(EntryType::Regular, payload_size);
        builder.append_data(&mut payload_header, "large.bin", &mut payload)
    });

    // Act.
    let inspection = inspect_tar_gz(Cursor::new(archive))
        .expect("a valid large file has no declared-size ceiling");

    // Assert.
    assert_eq!(inspection.index.entries.len(), 1);
    assert_eq!(inspection.index.entries[0].raw_path, "large.bin");
    assert_eq!(inspection.index.entries[0].kind, EntryKind::File);
    assert_eq!(inspection.index.entries[0].declared_size, payload_size);
}

#[test]
fn compressed_source_failure_is_exact_tar_decoder_error() {
    // Arrange.
    let archive = generated_tar_gz(Compression::default(), |builder| {
        append_empty(builder, "ok.txt", EntryType::Regular)
    });
    let source = ErrorAfterBytes {
        bytes: Cursor::new(archive),
    };

    // Act.
    let error = inspect_tar_gz(source).expect_err("source failure must remain a decoder error");

    // Assert.
    assert_eq!(
        error,
        InspectionError::TarDecoder {
            message: "sentinel compressed-source failure".into(),
        }
    );
}

#[test]
fn checksum_mismatch_is_exact_tar_parser_error() {
    // Arrange: corrupt a valid header without repairing its checksum.
    let mut raw_tar =
        generated_tar(|builder| append_empty(builder, "parser.txt", EntryType::Regular));
    raw_tar[0] ^= 1;
    let archive = gzip_bytes(&raw_tar, Compression::default());

    // Act.
    let error = inspect_tar_gz(Cursor::new(archive)).expect_err("bad tar checksum must not decode");

    // Assert.
    assert_eq!(
        error,
        InspectionError::TarParser {
            message: "archive header checksum mismatch".into(),
        }
    );
}
