#![cfg(feature = "native")]

use std::io::{Cursor, Write};

use arcadestr_core::{
    archive_inspection::{
        inspect_zip, ArchiveWarning, ContainerTypeNameConflict, EntryKind, InspectionError,
        MAX_COMPRESSION_RATIO, RATIO_ENFORCEMENT_OUTPUT_BYTES,
    },
    distribution::PathError,
};
use zip::{result::ZipResult, write::SimpleFileOptions, CompressionMethod, System, ZipWriter};

const LOCAL_HEADER: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
const CENTRAL_HEADER: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
const EFS_FLAG: u16 = 1 << 11;
const UNIX_SYMLINK_MODE: u32 = 0o120_777;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeaderKind {
    Local,
    Central,
}

fn options(method: CompressionMethod) -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(method)
        .system(System::Unix)
}

fn generated_zip(build: impl FnOnce(&mut ZipWriter<Cursor<Vec<u8>>>) -> ZipResult<()>) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    build(&mut writer).expect("generate ZIP entries in memory");
    writer.finish().expect("finish generated ZIP").into_inner()
}

fn matching_headers(bytes: &[u8], name: &[u8]) -> Vec<(usize, HeaderKind)> {
    let mut matches = Vec::new();
    for offset in 0..=bytes.len().saturating_sub(4) {
        let signature = &bytes[offset..offset + 4];
        let (kind, name_length_offset, name_offset) = if signature == LOCAL_HEADER {
            (HeaderKind::Local, 26, 30)
        } else if signature == CENTRAL_HEADER {
            (HeaderKind::Central, 28, 46)
        } else {
            continue;
        };
        let Some(length_bytes) =
            bytes.get(offset + name_length_offset..offset + name_length_offset + 2)
        else {
            continue;
        };
        let name_length = usize::from(u16::from_le_bytes([length_bytes[0], length_bytes[1]]));
        let name_start = offset + name_offset;
        let Some(candidate) = bytes.get(name_start..name_start + name_length) else {
            continue;
        };
        if candidate == name {
            matches.push((offset, kind));
        }
    }
    matches
}

fn patch_central_unix_mode(bytes: &mut [u8], name: &str, mode: u32) {
    let central = matching_headers(bytes, name.as_bytes())
        .into_iter()
        .filter(|(_, kind)| *kind == HeaderKind::Central)
        .collect::<Vec<_>>();
    assert_eq!(
        central.len(),
        1,
        "one central header must identify the entry"
    );
    let attributes = mode << 16;
    bytes[central[0].0 + 38..central[0].0 + 42].copy_from_slice(&attributes.to_le_bytes());
}

/// ZIP stores the method in both headers; patching both avoids contradictory metadata.
fn patch_compression_method(bytes: &mut [u8], name: &str, method: u16) {
    let headers = matching_headers(bytes, name.as_bytes());
    assert_eq!(
        headers.iter().map(|(_, kind)| *kind).collect::<Vec<_>>(),
        vec![HeaderKind::Local, HeaderKind::Central],
        "one local and one central header must identify the entry"
    );
    for (offset, kind) in headers {
        let method_offset = match kind {
            HeaderKind::Local => 8,
            HeaderKind::Central => 10,
        };
        bytes[offset + method_offset..offset + method_offset + 2]
            .copy_from_slice(&method.to_le_bytes());
    }
}

fn patch_name_and_clear_efs(bytes: &mut [u8], placeholder: &str, raw_name: &[u8]) {
    assert_eq!(placeholder.len(), raw_name.len());
    let headers = matching_headers(bytes, placeholder.as_bytes());
    assert_eq!(
        headers.len(),
        2,
        "local and central names must both be patched"
    );
    for (offset, kind) in headers {
        let (flags_offset, name_offset) = match kind {
            HeaderKind::Local => (6, 30),
            HeaderKind::Central => (8, 46),
        };
        let flags = u16::from_le_bytes([
            bytes[offset + flags_offset],
            bytes[offset + flags_offset + 1],
        ]) & !EFS_FLAG;
        bytes[offset + flags_offset..offset + flags_offset + 2]
            .copy_from_slice(&flags.to_le_bytes());
        bytes[offset + name_offset..offset + name_offset + raw_name.len()]
            .copy_from_slice(raw_name);
    }
}

#[test]
fn generated_hazard_zip_indexes_exact_invalid_paths_collision_and_escaping_symlink() {
    // Arrange: the symlink body is an escaping target; only its metadata is patched.
    let long_path = "a".repeat(201);
    let mut archive = generated_zip(|writer| {
        for name in ["CON.txt", "dir\\escape", "Game.exe", "game.exe"] {
            writer.start_file(name, options(CompressionMethod::Stored))?;
        }
        writer.start_file(&long_path, options(CompressionMethod::Stored))?;
        writer.start_file("escape-link", options(CompressionMethod::Stored))?;
        writer.write_all(b"../../outside")?;
        Ok(())
    });
    patch_central_unix_mode(&mut archive, "escape-link", UNIX_SYMLINK_MODE);

    // Act.
    let inspection =
        inspect_zip(Cursor::new(archive)).expect("hazards are reportable, not parser failures");

    // Assert: every decodable invalid entry remains indexed with its original path.
    assert_eq!(
        inspection
            .index
            .entries
            .iter()
            .map(|entry| (entry.raw_path.as_str(), &entry.kind))
            .collect::<Vec<_>>(),
        vec![
            ("CON.txt", &EntryKind::File),
            ("dir\\escape", &EntryKind::File),
            ("Game.exe", &EntryKind::File),
            ("game.exe", &EntryKind::File),
            (long_path.as_str(), &EntryKind::File),
            ("escape-link", &EntryKind::Symlink),
        ]
    );
    assert_eq!(
        inspection.hazards.invalid_paths,
        vec![
            ("CON.txt".into(), PathError::Rule8ReservedDeviceName),
            ("dir\\escape".into(), PathError::Rule3Backslash),
            (long_path, PathError::Rule9LengthExceeded),
        ]
    );
    assert_eq!(
        inspection.hazards.case_collisions,
        vec![vec![String::from("Game.exe"), String::from("game.exe"),]]
    );
    assert_eq!(inspection.hazards.symlinks, vec!["escape-link"]);
}

#[test]
fn nested_zip_decodes_one_directory_slash_and_derives_explicit_and_implied_working_directories() {
    // Arrange.
    let archive = generated_zip(|writer| {
        writer.add_directory("game/", options(CompressionMethod::Stored))?;
        writer.add_directory("game/assets/", options(CompressionMethod::Stored))?;
        writer.add_directory("game/data//", options(CompressionMethod::Stored))?;
        writer.start_file("game/bin/game.exe", options(CompressionMethod::Stored))?;
        writer.write_all(b"play")?;
        writer.start_file("game/docs/readme.txt", options(CompressionMethod::Stored))?;
        writer.write_all(b"manual")?;
        Ok(())
    });

    // Act.
    let inspection = inspect_zip(Cursor::new(archive)).expect("nested ZIP must inspect");

    // Assert: container directories lose exactly one slash and sizes remain u64 metadata.
    assert_eq!(
        inspection
            .index
            .entries
            .iter()
            .map(|entry| (
                entry.raw_path.as_str(),
                entry.kind.clone(),
                entry.declared_size
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

#[test]
fn stored_zip_indexes_successfully() {
    // Arrange.
    let archive = generated_zip(|writer| {
        writer.start_file("stored.txt", options(CompressionMethod::Stored))?;
        writer.write_all(b"stored payload")?;
        Ok(())
    });

    // Act.
    let inspection = inspect_zip(Cursor::new(archive)).expect("Store is supported");

    // Assert.
    assert_eq!(inspection.index.entries[0].raw_path, "stored.txt");
    assert_eq!(inspection.index.entries[0].declared_size, 14_u64);
}

#[test]
fn deflated_zip_indexes_successfully() {
    // Arrange.
    let archive = generated_zip(|writer| {
        writer.start_file("deflated.txt", options(CompressionMethod::Deflated))?;
        writer.write_all(b"deflated payload")?;
        Ok(())
    });

    // Act.
    let inspection = inspect_zip(Cursor::new(archive)).expect("Deflate is supported");

    // Assert.
    assert_eq!(inspection.index.entries[0].raw_path, "deflated.txt");
    assert_eq!(inspection.index.entries[0].declared_size, 16_u64);
}

#[test]
fn runtime_deflate_ratio_bomb_returns_exact_measured_ratio_error_after_64_mib() {
    // Arrange: stream zeroes into the writer; never allocate the 64 MiB payload as a whole.
    let chunk = [0_u8; 32 * 1024];
    let chunks =
        RATIO_ENFORCEMENT_OUTPUT_BYTES / u64::try_from(chunk.len()).expect("chunk length") + 1;
    let archive = generated_zip(|writer| {
        writer.start_file("ratio.bin", options(CompressionMethod::Deflated))?;
        for _ in 0..chunks {
            writer.write_all(&chunk)?;
        }
        Ok(())
    });

    // Act.
    let error = inspect_zip(Cursor::new(archive)).expect_err("measured output must exceed 200:1");

    // Assert: enforcement evidence is measured output/input, never the declared size field.
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
    assert!(measured_output > measured_input * MAX_COMPRESSION_RATIO);
}

#[test]
fn efs_unset_valid_utf8_name_warns_and_indexes_only_cp437_decoding() {
    // Arrange: C3 A9 is UTF-8 "é" but CP437 "├⌐".
    let raw_name = [0xc3, 0xa9, b'.', b't', b'x', b't'];
    let mut archive = generated_zip(|writer| {
        writer.start_file("xx.txt", options(CompressionMethod::Stored))?;
        Ok(())
    });
    patch_name_and_clear_efs(&mut archive, "xx.txt", &raw_name);

    // Act.
    let inspection = inspect_zip(Cursor::new(archive)).expect("EFS-unset CP437 name must inspect");

    // Assert.
    assert_eq!(inspection.index.entries.len(), 1);
    assert_eq!(inspection.index.entries[0].raw_path, "├⌐.txt");
    assert!(inspection.hazards.invalid_paths.is_empty());
    assert_eq!(
        inspection.warnings.entries,
        vec![ArchiveWarning::ZipEfsUnsetUtf8Cp437Divergence {
            bytes: raw_name.to_vec(),
            utf8: "é.txt".into(),
            cp437: "├⌐.txt".into(),
        }]
    );
}

#[test]
fn patched_unsupported_method_returns_exact_entry_and_method() {
    // Arrange: method 12 is patched narrowly in the local and central headers.
    let mut archive = generated_zip(|writer| {
        writer.start_file("unsupported.bin", options(CompressionMethod::Stored))?;
        writer.write_all(b"payload")?;
        Ok(())
    });
    patch_compression_method(&mut archive, "unsupported.bin", 12);

    // Act.
    let error = inspect_zip(Cursor::new(archive)).expect_err("method 12 is outside ZIP v1 policy");

    // Assert.
    assert_eq!(
        error,
        InspectionError::UnsupportedCompressionMethod {
            entry: "unsupported.bin".into(),
            method: 12,
        }
    );
}

#[test]
fn file_metadata_with_trailing_slash_is_conflict_and_invalid_not_directory() {
    // Arrange.
    let archive = generated_zip(|writer| {
        writer.start_file("metadata/", options(CompressionMethod::Stored))?;
        writer.write_all(b"file payload")?;
        Ok(())
    });

    // Act.
    let inspection = inspect_zip(Cursor::new(archive)).expect("type/name conflict is reportable");

    // Assert.
    assert_eq!(inspection.index.entries[0].raw_path, "metadata/");
    assert_eq!(inspection.index.entries[0].kind, EntryKind::File);
    assert_eq!(
        inspection.hazards.invalid_paths,
        vec![("metadata/".into(), PathError::Rule6EmptyOrDotSegment)]
    );
    assert_eq!(
        inspection.hazards.container_type_name_conflicts,
        vec![ContainerTypeNameConflict {
            entry: "metadata/".into(),
            kind: EntryKind::File,
        }]
    );
}
