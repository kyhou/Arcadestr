//! Archive index types and path-decoding policy.

use std::collections::{BTreeMap, BTreeSet};

use crate::distribution::{PackageFormat, PathError, RelativePath};

use super::limits::EntryCounter;
use super::InspectionError;

/// An inspected archive's validated entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveIndex {
    /// The package format declared by the publisher.
    pub format: PackageFormat,
    /// Every losslessly decoded entry, including entries with invalid paths.
    pub entries: Vec<ArchiveEntry>,
}

impl ArchiveIndex {
    /// Derives possible working directories without storing a second index.
    ///
    /// The result contains the installation root (`.`), explicit directories,
    /// and every parent of a regular file.
    #[must_use]
    pub fn working_directories(&self) -> Vec<String> {
        let mut directories = BTreeSet::from([String::from(".")]);

        for entry in &self.entries {
            match &entry.kind {
                EntryKind::Directory => insert_valid_directory(&mut directories, &entry.raw_path),
                EntryKind::File => insert_valid_parents(&mut directories, &entry.raw_path),
                EntryKind::Symlink | EntryKind::HardLink | EntryKind::Other => {}
            }
        }

        directories.into_iter().collect()
    }
}

/// One losslessly decoded archive entry, including invalid paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// The decoded, unnormalized name after directory slash handling.
    pub raw_path: String,
    /// The kind independently reported by the container.
    pub kind: EntryKind,
    /// Container-declared size metadata, never used for resource accounting.
    pub declared_size: u64,
    /// Unix permission bits when supplied by the container.
    pub unix_mode: Option<u32>,
}

/// An entry kind reported independently from its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symbolic link, which is blocked by v1 policy.
    Symlink,
    /// A hard link, which is blocked by v1 policy.
    HardLink,
    /// Another container-specific kind, which is blocked by v1 policy.
    Other,
}

/// Blocking archive hazards collected during inspection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArchiveHazards {
    /// Decoded names rejected by the A2 portable path rules.
    pub invalid_paths: Vec<(String, PathError)>,
    /// Symbolic-link entry names.
    pub symlinks: Vec<String>,
    /// Hard-link entry names.
    pub hard_links: Vec<String>,
    /// Groups of decoded paths that compare equal without case.
    pub case_collisions: Vec<Vec<String>>,
    /// Unsupported container entry kinds.
    pub other_entry_kinds: Vec<String>,
    /// Entry names that could not be decoded losslessly.
    pub undecodable_paths: Vec<UndecodablePath>,
    /// Disagreements between container kinds and trailing-slash names.
    pub container_type_name_conflicts: Vec<ContainerTypeNameConflict>,
    /// A disagreement between ZIP footer metadata and the parsed entry index.
    pub zip_declared_entry_count_mismatch: Option<ZipDeclaredEntryCountMismatch>,
}

impl ArchiveHazards {
    /// Reports whether inspection found any blocking hazard.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.invalid_paths.is_empty()
            && self.symlinks.is_empty()
            && self.hard_links.is_empty()
            && self.case_collisions.is_empty()
            && self.other_entry_kinds.is_empty()
            && self.undecodable_paths.is_empty()
            && self.container_type_name_conflicts.is_empty()
            && self.zip_declared_entry_count_mismatch.is_none()
    }
}

/// A path that the container decoder could not represent losslessly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndecodablePath {
    /// A display-only lossy rendering.
    pub lossy: String,
    /// The original path bytes.
    pub bytes: Vec<u8>,
}

/// A disagreement between a container entry kind and trailing slash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerTypeNameConflict {
    /// The original decoded name, before directory slash handling.
    pub entry: String,
    /// The independently determined container kind.
    pub kind: EntryKind,
}

/// A mismatch between preflight-declared and parser-observed ZIP entry counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZipDeclaredEntryCountMismatch {
    /// Logical entry count declared by the EOCD or ZIP64 EOCD.
    pub declared: u64,
    /// Entry count produced by the ZIP central-directory parser.
    pub parsed: u64,
}

/// Non-blocking archive warnings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArchiveWarnings {
    /// Structured warnings emitted during inspection.
    pub entries: Vec<ArchiveWarning>,
}

/// A non-blocking archive condition publishers should review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchiveWarning {
    /// An EFS-unset ZIP name has divergent UTF-8 and CP437 readings.
    ZipEfsUnsetUtf8Cp437Divergence {
        /// Raw ZIP filename bytes.
        bytes: Vec<u8>,
        /// The valid UTF-8 interpretation.
        utf8: String,
        /// The CP437 interpretation used by ZIP policy.
        cp437: String,
    },
}

/// The complete policy result of archive inspection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveInspection {
    /// Validated archive entries.
    pub index: ArchiveIndex,
    /// Blocking hazards.
    pub hazards: ArchiveHazards,
    /// Non-blocking warnings.
    pub warnings: ArchiveWarnings,
}

/// Shared decoder output for one losslessly decoded entry name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedEntryName {
    /// The decoded name after the sole permitted slash removal.
    pub raw_path: String,
    /// The preserved A2 error when the decoded name is invalid.
    pub path_error: Option<PathError>,
    /// A type/name conflict, when the independently determined kind disagrees.
    pub conflict: Option<ContainerTypeNameConflict>,
}

/// Applies the shared, non-normalizing entry-name policy.
///
/// Only a container-confirmed directory loses exactly one trailing slash.
/// Every resulting name is passed exactly once to `RelativePath::parse`.
#[must_use]
pub fn decode_entry_name(decoded_name: &str, kind: &EntryKind) -> DecodedEntryName {
    let conflict =
        (!matches!(kind, EntryKind::Directory) && decoded_name.ends_with('/')).then(|| {
            ContainerTypeNameConflict {
                entry: decoded_name.to_owned(),
                kind: kind.clone(),
            }
        });
    let raw_path = if matches!(kind, EntryKind::Directory) {
        decoded_name.strip_suffix('/').unwrap_or(decoded_name)
    } else {
        decoded_name
    }
    .to_owned();
    let path_error = RelativePath::parse(&raw_path).err();

    DecodedEntryName {
        raw_path,
        path_error,
        conflict,
    }
}

/// Incrementally builds an inspection result without performing parser I/O.
#[derive(Debug)]
pub struct ArchiveInspectionBuilder {
    format: PackageFormat,
    entries: Vec<ArchiveEntry>,
    hazards: ArchiveHazards,
    warnings: ArchiveWarnings,
    entry_counter: EntryCounter,
}

impl ArchiveInspectionBuilder {
    /// Creates a builder for the publisher-declared package format.
    #[must_use]
    pub fn new(format: PackageFormat) -> Self {
        Self {
            format,
            entries: Vec::new(),
            hazards: ArchiveHazards::default(),
            warnings: ArchiveWarnings::default(),
            entry_counter: EntryCounter::default(),
        }
    }

    /// Records one losslessly decoded container entry.
    ///
    /// # Errors
    ///
    /// Returns [`InspectionError::EntryLimitExceeded`] for entry 200,001.
    pub fn record_decoded_entry(
        &mut self,
        decoded_name: &str,
        kind: EntryKind,
        declared_size: u64,
        unix_mode: Option<u32>,
    ) -> Result<(), InspectionError> {
        self.record_decoded_entry_with_other_kind(
            decoded_name,
            kind,
            declared_size,
            unix_mode,
            None,
        )
    }

    pub(crate) fn record_decoded_entry_with_other_kind(
        &mut self,
        decoded_name: &str,
        kind: EntryKind,
        declared_size: u64,
        unix_mode: Option<u32>,
        other_kind_diagnostic: Option<String>,
    ) -> Result<(), InspectionError> {
        self.entry_counter.record_entry()?;
        self.record_kind_hazard(decoded_name, &kind, other_kind_diagnostic);

        let decoded = decode_entry_name(decoded_name, &kind);
        if let Some(conflict) = decoded.conflict {
            self.hazards.container_type_name_conflicts.push(conflict);
        }

        if let Some(error) = decoded.path_error {
            self.hazards
                .invalid_paths
                .push((decoded.raw_path.clone(), error));
        }
        self.entries.push(ArchiveEntry {
            raw_path: decoded.raw_path,
            kind,
            declared_size,
            unix_mode,
        });
        Ok(())
    }

    /// Records one undecodable entry, excluding it from the index and collisions.
    ///
    /// # Errors
    ///
    /// Returns [`InspectionError::EntryLimitExceeded`] for entry 200,001.
    pub fn record_undecodable_path(
        &mut self,
        lossy: String,
        bytes: Vec<u8>,
    ) -> Result<(), InspectionError> {
        self.entry_counter.record_entry()?;
        self.hazards
            .undecodable_paths
            .push(UndecodablePath { lossy, bytes });
        Ok(())
    }

    /// Records a non-blocking warning.
    pub fn record_warning(&mut self, warning: ArchiveWarning) {
        self.warnings.entries.push(warning);
    }

    pub(crate) fn record_zip_entry_count_mismatch(&mut self, declared: u64, parsed: u64) {
        self.hazards.zip_declared_entry_count_mismatch =
            Some(ZipDeclaredEntryCountMismatch { declared, parsed });
    }

    /// Completes inspection and computes case-insensitive collision groups.
    #[must_use]
    pub fn finish(mut self) -> ArchiveInspection {
        self.hazards.case_collisions = case_collisions(&self.entries);
        ArchiveInspection {
            index: ArchiveIndex {
                format: self.format,
                entries: self.entries,
            },
            hazards: self.hazards,
            warnings: self.warnings,
        }
    }

    fn record_kind_hazard(
        &mut self,
        decoded_name: &str,
        kind: &EntryKind,
        other_kind_diagnostic: Option<String>,
    ) {
        match kind {
            EntryKind::Symlink => self.hazards.symlinks.push(decoded_name.to_owned()),
            EntryKind::HardLink => self.hazards.hard_links.push(decoded_name.to_owned()),
            EntryKind::Other => self
                .hazards
                .other_entry_kinds
                .push(other_kind_diagnostic.unwrap_or_else(|| decoded_name.to_owned())),
            EntryKind::File | EntryKind::Directory => {}
        }
    }
}

fn insert_valid_directory(directories: &mut BTreeSet<String>, path: &str) {
    if RelativePath::parse(path).is_ok() {
        directories.insert(path.to_owned());
    }
}

fn insert_valid_parents(directories: &mut BTreeSet<String>, path: &str) {
    let mut end = 0;
    while let Some(relative) = path[end..].find('/') {
        end += relative;
        insert_valid_directory(directories, &path[..end]);
        end += 1;
    }
}

fn case_collisions(entries: &[ArchiveEntry]) -> Vec<Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in entries {
        groups
            .entry(entry.raw_path.to_lowercase())
            .or_default()
            .push(entry.raw_path.clone());
    }

    groups
        .into_values()
        .filter_map(|mut paths| {
            if paths.len() < 2 {
                return None;
            }
            paths.sort();
            Some(paths)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_decoder_strips_exactly_one_slash() {
        let decoded = decode_entry_name("bin//", &EntryKind::Directory);

        assert_eq!(decoded.raw_path, "bin/");
        assert_eq!(decoded.path_error, Some(PathError::Rule6EmptyOrDotSegment));
        assert_eq!(decoded.conflict, None);
    }

    #[test]
    fn directory_root_and_empty_names_remain_invalid() {
        for name in ["/", ""] {
            let decoded = decode_entry_name(name, &EntryKind::Directory);
            assert_eq!(decoded.raw_path, "");
            assert_eq!(decoded.path_error, Some(PathError::Rule6EmptyOrDotSegment));
        }
    }

    #[test]
    fn non_directory_slash_is_unchanged_and_conflicting() {
        let decoded = decode_entry_name("bin/", &EntryKind::File);

        assert_eq!(decoded.raw_path, "bin/");
        assert_eq!(decoded.path_error, Some(PathError::Rule6EmptyOrDotSegment));
        assert_eq!(
            decoded.conflict,
            Some(ContainerTypeNameConflict {
                entry: String::from("bin/"),
                kind: EntryKind::File,
            })
        );
    }

    #[test]
    fn decoded_invalid_entries_are_indexed_collide_and_undecodable_entries_are_excluded() {
        let mut builder = ArchiveInspectionBuilder::new(PackageFormat::Zip);
        builder
            .record_decoded_entry("Bad//Name", EntryKind::File, 4, None)
            .expect("first decoded entry should fit");
        builder
            .record_decoded_entry("bad//name", EntryKind::File, 8, Some(0o755))
            .expect("second decoded entry should fit");
        builder
            .record_undecodable_path(String::from("BAD//NAME"), vec![0xff])
            .expect("undecodable entry should fit");

        let inspection = builder.finish();
        assert_eq!(
            inspection
                .index
                .entries
                .iter()
                .map(|entry| entry.raw_path.as_str())
                .collect::<Vec<_>>(),
            vec!["Bad//Name", "bad//name"]
        );
        assert_eq!(
            inspection.hazards.invalid_paths,
            vec![
                (String::from("Bad//Name"), PathError::Rule6EmptyOrDotSegment),
                (String::from("bad//name"), PathError::Rule6EmptyOrDotSegment),
            ]
        );
        assert_eq!(
            inspection.hazards.case_collisions,
            vec![vec![String::from("Bad//Name"), String::from("bad//name")]]
        );
        assert_eq!(inspection.hazards.undecodable_paths.len(), 1);
    }

    #[test]
    fn declared_size_is_display_metadata_and_does_not_affect_limits() {
        let mut builder = ArchiveInspectionBuilder::new(PackageFormat::TarZstd);
        builder
            .record_decoded_entry("game", EntryKind::File, u64::MAX, None)
            .expect("declared size must not affect entry accounting");

        let inspection = builder.finish();
        assert_eq!(inspection.index.entries.len(), 1);
        assert_eq!(inspection.index.entries[0].declared_size, u64::MAX);
    }

    #[test]
    fn working_directories_are_derived_without_discarding_hostile_entries() {
        let mut builder = ArchiveInspectionBuilder::new(PackageFormat::TarGz);
        builder
            .record_decoded_entry("empty/", EntryKind::Directory, 0, None)
            .expect("directory should fit");
        builder
            .record_decoded_entry("bin/tools/game", EntryKind::File, 1, None)
            .expect("file should fit");
        builder
            .record_decoded_entry("../hostile/", EntryKind::Directory, 0, None)
            .expect("invalid decoded directory is still indexed");

        let inspection = builder.finish();
        assert_eq!(inspection.index.entries.len(), 3);
        assert_eq!(
            inspection.index.working_directories(),
            vec![".", "bin", "bin/tools", "empty"]
        );
    }
}
