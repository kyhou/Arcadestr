use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const STAMP_RELATIVE_PATH: &str = "web/.arcadestr-frontend-build.txt";

const TRACKED_FILES: &[&str] = &[
    "Cargo.lock",
    "Cargo.toml",
    "app/Cargo.toml",
    "core/Cargo.toml",
];
const TRACKED_DIRECTORIES: &[&str] = &["app/src", "core/src", "web"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendBuildStamp {
    pub fingerprint: String,
    pub distribution_fingerprint: String,
    pub built_at_unix: u64,
    pub profile: String,
}

impl FrontendBuildStamp {
    #[allow(dead_code)]
    pub fn parse(value: &str) -> io::Result<Self> {
        let field = |name: &str| {
            value
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{name}=")))
                .map(str::to_owned)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("missing {name}"))
                })
        };
        let built_at_unix = field("built_at_unix")?.parse().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid built_at_unix: {error}"),
            )
        })?;
        Ok(Self {
            fingerprint: field("fingerprint")?,
            distribution_fingerprint: field("distribution_fingerprint")?,
            built_at_unix,
            profile: field("profile")?,
        })
    }

    #[allow(dead_code)]
    pub fn encode(&self) -> String {
        format!(
            "fingerprint={}\ndistribution_fingerprint={}\nbuilt_at_unix={}\nprofile={}\n",
            self.fingerprint, self.distribution_fingerprint, self.built_at_unix, self.profile
        )
    }
}

pub fn tracked_inputs(workspace_root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut inputs = Vec::new();
    for relative in TRACKED_FILES {
        let path = workspace_root.join(relative);
        if path.is_file() {
            inputs.push(path);
        }
    }
    for relative in TRACKED_DIRECTORIES {
        collect_files(
            workspace_root,
            &workspace_root.join(relative),
            &mut inputs,
            true,
        )?;
    }
    inputs.sort();
    Ok(inputs)
}

pub fn fingerprint(workspace_root: &Path) -> io::Result<String> {
    fingerprint_paths(workspace_root, tracked_inputs(workspace_root)?)
}

pub fn distribution_fingerprint(workspace_root: &Path) -> io::Result<String> {
    let directory = workspace_root.join("web/dist");
    let mut inputs = Vec::new();
    collect_files(workspace_root, &directory, &mut inputs, false)?;
    inputs.sort();
    if inputs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "web/dist is missing or empty",
        ));
    }
    fingerprint_paths(workspace_root, inputs)
}

fn fingerprint_paths(workspace_root: &Path, paths: Vec<PathBuf>) -> io::Result<String> {
    let mut hash = Sha256::new();
    for path in paths {
        let relative = path
            .strip_prefix(workspace_root)
            .map_err(io::Error::other)?;
        let relative = relative.to_string_lossy();
        let contents = fs::read(&path)?;
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative.as_bytes());
        hash.update((contents.len() as u64).to_le_bytes());
        hash.update(contents);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[allow(dead_code)]
pub fn stamp_is_fresh(
    stamp: Option<&FrontendBuildStamp>,
    fingerprint: &str,
    distribution_fingerprint: Option<&str>,
    profile: &str,
) -> bool {
    stamp.is_some_and(|stamp| {
        stamp.fingerprint == fingerprint
            && distribution_fingerprint == Some(stamp.distribution_fingerprint.as_str())
            && (profile != "release" || stamp.profile == "release")
    })
}

fn collect_files(
    workspace_root: &Path,
    directory: &Path,
    inputs: &mut Vec<PathBuf>,
    exclude_build_outputs: bool,
) -> io::Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let relative = path
            .strip_prefix(workspace_root)
            .map_err(io::Error::other)?;
        if exclude_build_outputs
            && (relative == Path::new("web/dist")
                || relative.starts_with("web/dist/")
                || relative == Path::new(STAMP_RELATIVE_PATH))
        {
            continue;
        }
        if path.is_dir() {
            collect_files(workspace_root, &path, inputs, exclude_build_outputs)?;
        } else if path.is_file() {
            inputs.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_inputs_cover_front_end_sources_and_configuration() {
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask is inside the workspace");
        let inputs = tracked_inputs(workspace_root).expect("tracked inputs");
        let relative = inputs
            .iter()
            .map(|path| path.strip_prefix(workspace_root).expect("workspace input"))
            .collect::<Vec<_>>();

        for required in [
            "Cargo.lock",
            "Cargo.toml",
            "app/Cargo.toml",
            "app/src/lib.rs",
            "core/Cargo.toml",
            "core/src/lib.rs",
            "web/Cargo.toml",
            "web/Trunk.toml",
            "web/index.html",
            "web/snippets/tauri-bridge.js",
            "web/src/main.rs",
            "web/style/tailwind.css",
            "web/tailwind.config.js",
        ] {
            assert!(
                relative.contains(&Path::new(required)),
                "missing {required}"
            );
        }
        assert!(relative.iter().all(|path| !path.starts_with("web/dist")));
    }

    #[test]
    fn stamp_round_trips() {
        let stamp = FrontendBuildStamp {
            fingerprint: "abc123".to_owned(),
            distribution_fingerprint: "def456".to_owned(),
            built_at_unix: 42,
            profile: "release".to_owned(),
        };
        assert_eq!(FrontendBuildStamp::parse(&stamp.encode()).unwrap(), stamp);
    }

    #[test]
    fn release_requires_a_matching_release_stamp() {
        let debug = FrontendBuildStamp {
            fingerprint: "current".to_owned(),
            distribution_fingerprint: "distribution".to_owned(),
            built_at_unix: 1,
            profile: "debug".to_owned(),
        };
        let release = FrontendBuildStamp {
            profile: "release".to_owned(),
            ..debug.clone()
        };

        assert!(stamp_is_fresh(
            Some(&debug),
            "current",
            Some("distribution"),
            "debug"
        ));
        assert!(!stamp_is_fresh(
            Some(&debug),
            "current",
            Some("distribution"),
            "release"
        ));
        assert!(stamp_is_fresh(
            Some(&release),
            "current",
            Some("distribution"),
            "release"
        ));
        assert!(!stamp_is_fresh(
            Some(&release),
            "current",
            Some("changed"),
            "release"
        ));
        assert!(!stamp_is_fresh(
            Some(&release),
            "changed",
            Some("distribution"),
            "release"
        ));
        assert!(!stamp_is_fresh(
            None,
            "current",
            Some("distribution"),
            "release"
        ));
    }
}
