//! Advisory executable candidate detection over inspected archive entries.

use std::{collections::BTreeSet, fmt};

use crate::distribution::{PlatformTarget, RelativePath};

use super::{ArchiveEntry, ArchiveIndex, EntryKind};

/// A publisher-facing executable suggestion, never an authoritative selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutableCandidate {
    /// A validated archive path addressable by a launch profile.
    pub path: RelativePath,
    /// The strength of the signal behind this suggestion.
    pub confidence: CandidateConfidence,
    /// A human-readable explanation of why the path was suggested.
    pub reason: CandidateReason,
}

/// Coarse confidence shown to publishers without implying automatic selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateConfidence {
    High,
    Medium,
    Low,
}

/// The primary signal that made an archive entry a candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateReason {
    GodotPackageBesideBinary,
    UnityDataDirectoryBesideBinary,
    UnrealWin64Layout,
    WindowsExecutableExtension { archive_root: bool },
    UnixExecutableMode { archive_root: bool },
}

impl fmt::Display for CandidateReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GodotPackageBesideBinary => {
                formatter.write_str("Godot binary with a matching .pck beside it")
            }
            Self::UnityDataDirectoryBesideBinary => {
                formatter.write_str("Unity executable with a matching _Data directory beside it")
            }
            Self::UnrealWin64Layout => {
                formatter.write_str("Windows executable in an Unreal Binaries/Win64 directory")
            }
            Self::WindowsExecutableExtension { archive_root: true } => {
                formatter.write_str("Windows .exe file at the archive root")
            }
            Self::WindowsExecutableExtension {
                archive_root: false,
            } => formatter.write_str("Windows .exe file in the archive"),
            Self::UnixExecutableMode { archive_root: true } => {
                formatter.write_str("Unix executable permission bits at the archive root")
            }
            Self::UnixExecutableMode {
                archive_root: false,
            } => formatter.write_str("Unix executable permission bits in the archive"),
        }
    }
}

/// Ranks plausible executables without selecting one on the publisher's behalf.
#[must_use]
pub fn detect_executable_candidates(
    index: &ArchiveIndex,
    target: &PlatformTarget,
) -> Vec<ExecutableCandidate> {
    let file_paths = index
        .entries
        .iter()
        .filter(|entry| matches!(entry.kind, EntryKind::File))
        .map(|entry| entry.raw_path.to_lowercase())
        .collect::<BTreeSet<_>>();
    let directories = index
        .working_directories()
        .into_iter()
        .map(|path| path.to_lowercase())
        .collect::<BTreeSet<_>>();

    let mut ranked = index
        .entries
        .iter()
        .filter_map(|entry| rank_entry(entry, target, &file_paths, &directories))
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.depth.cmp(&right.depth))
            .then_with(|| {
                left.candidate
                    .path
                    .as_str()
                    .to_lowercase()
                    .cmp(&right.candidate.path.as_str().to_lowercase())
            })
            .then_with(|| {
                left.candidate
                    .path
                    .as_str()
                    .cmp(right.candidate.path.as_str())
            })
    });
    ranked.into_iter().map(|ranked| ranked.candidate).collect()
}

struct RankedCandidate {
    candidate: ExecutableCandidate,
    score: u16,
    depth: usize,
}

fn rank_entry(
    entry: &ArchiveEntry,
    target: &PlatformTarget,
    file_paths: &BTreeSet<String>,
    directories: &BTreeSet<String>,
) -> Option<RankedCandidate> {
    if !matches!(entry.kind, EntryKind::File) || is_obvious_non_candidate(&entry.raw_path) {
        return None;
    }

    let path = RelativePath::parse(&entry.raw_path).ok()?;
    let depth = entry.raw_path.matches('/').count();
    let archive_root = depth == 0;
    let lowercase = entry.raw_path.to_lowercase();
    let generic_signal = match target.os.as_str() {
        "windows" if lowercase.ends_with(".exe") => {
            Some(CandidateReason::WindowsExecutableExtension { archive_root })
        }
        "linux" if entry.unix_mode.is_some_and(|mode| mode & 0o111 != 0) => {
            Some(CandidateReason::UnixExecutableMode { archive_root })
        }
        _ => None,
    }?;

    let (reason, score, confidence) = if has_matching_godot_package(&lowercase, file_paths) {
        (
            CandidateReason::GodotPackageBesideBinary,
            1_000,
            CandidateConfidence::High,
        )
    } else if has_matching_unity_directory(&lowercase, directories) {
        (
            CandidateReason::UnityDataDirectoryBesideBinary,
            950,
            CandidateConfidence::High,
        )
    } else if target.os == "windows" && lowercase.contains("/binaries/win64/") {
        (
            CandidateReason::UnrealWin64Layout,
            900,
            CandidateConfidence::High,
        )
    } else {
        let confidence = if archive_root {
            CandidateConfidence::Medium
        } else {
            CandidateConfidence::Low
        };
        (
            generic_signal,
            if archive_root { 550 } else { 500 },
            confidence,
        )
    };

    Some(RankedCandidate {
        candidate: ExecutableCandidate {
            path,
            confidence,
            reason,
        },
        score,
        depth,
    })
}

fn has_matching_godot_package(path: &str, file_paths: &BTreeSet<String>) -> bool {
    let Some(stem) = path.strip_suffix(".exe") else {
        return file_paths.contains(&format!("{path}.pck"));
    };
    file_paths.contains(&format!("{stem}.pck"))
}

fn has_matching_unity_directory(path: &str, directories: &BTreeSet<String>) -> bool {
    let stem = path.strip_suffix(".exe").unwrap_or(path);
    directories.contains(&format!("{stem}_data"))
}

fn is_obvious_non_candidate(path: &str) -> bool {
    let lowercase = path.to_lowercase();
    let basename = lowercase.rsplit('/').next().unwrap_or(lowercase.as_str());
    let excluded_name = basename.starts_with("unitycrashhandler")
        || basename.ends_with("handler.exe")
        || basename.starts_with("uninstall")
        || basename.starts_with("unins");
    let under_data_payload = lowercase.split('/').any(|segment| {
        segment == "data"
            || segment == "assets"
            || segment == "content"
            || segment.ends_with("_data")
    });

    excluded_name || under_data_payload
}

#[cfg(test)]
mod tests {
    use crate::distribution::PackageFormat;

    use super::*;

    #[test]
    fn godot_fixture_ranks_matching_binary_first() {
        let index = index(vec![
            file("Game.exe", None),
            file("Game.pck", None),
            file("tools/config.exe", None),
        ]);

        let candidates = detect_executable_candidates(&index, &windows());

        assert_eq!(candidates[0].path.as_str(), "Game.exe");
        assert_eq!(candidates[0].confidence, CandidateConfidence::High);
        assert_eq!(
            candidates[0].reason,
            CandidateReason::GodotPackageBesideBinary
        );
        assert!(!candidates[0].reason.to_string().is_empty());
    }

    #[test]
    fn unity_fixture_excludes_crash_handler_and_ranks_game_first() {
        let index = index(vec![
            file("Build.exe", None),
            file("Build_Data/globalgamemanagers", None),
            file("UnityCrashHandler64.exe", None),
            file("Build_Data/plugin.exe", None),
        ]);

        let candidates = detect_executable_candidates(&index, &windows());

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path.as_str(), "Build.exe");
        assert_eq!(
            candidates[0].reason,
            CandidateReason::UnityDataDirectoryBesideBinary
        );
    }

    #[test]
    fn single_unix_binary_is_the_top_candidate() {
        let index = index(vec![
            file("game", Some(0o100755)),
            file("tools/config", Some(0o100755)),
            file("README", Some(0o100644)),
        ]);

        let candidates = detect_executable_candidates(&index, &linux());

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].path.as_str(), "game");
        assert_eq!(
            candidates[0].reason,
            CandidateReason::UnixExecutableMode { archive_root: true }
        );
    }

    #[test]
    fn unreal_layout_outranks_generic_windows_executable() {
        let index = index(vec![
            file("Launcher.exe", None),
            file("Game/Binaries/Win64/Game.exe", None),
        ]);

        let candidates = detect_executable_candidates(&index, &windows());

        assert_eq!(candidates[0].path.as_str(), "Game/Binaries/Win64/Game.exe");
        assert_eq!(candidates[0].reason, CandidateReason::UnrealWin64Layout);
    }

    #[test]
    fn hazardous_executable_path_is_never_suggested() {
        let index = index(vec![file("bad\\Game.exe", None), file("safe.exe", None)]);

        let candidates = detect_executable_candidates(&index, &windows());

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path.as_str(), "safe.exe");
    }

    #[test]
    fn empty_candidate_list_is_valid_when_no_plausible_signal_exists() {
        let index = index(vec![file("game", Some(0o100644)), file("README", None)]);

        assert!(detect_executable_candidates(&index, &linux()).is_empty());
    }

    fn index(entries: Vec<ArchiveEntry>) -> ArchiveIndex {
        ArchiveIndex {
            format: PackageFormat::Zip,
            entries,
        }
    }

    fn file(raw_path: &str, unix_mode: Option<u32>) -> ArchiveEntry {
        ArchiveEntry {
            raw_path: raw_path.to_owned(),
            kind: EntryKind::File,
            declared_size: 0,
            unix_mode,
        }
    }

    fn windows() -> PlatformTarget {
        PlatformTarget {
            os: "windows".to_owned(),
            arch: "x86_64".to_owned(),
        }
    }

    fn linux() -> PlatformTarget {
        PlatformTarget {
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
        }
    }
}
