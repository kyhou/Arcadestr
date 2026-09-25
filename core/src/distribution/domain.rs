// Validated domain values consumed after distribution metadata resolution.

use serde::{Deserialize, Deserializer, Serialize};

use super::errors::{PathError, ProfileIdError};

/// The boundary type consumed by every downstream installation stage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedInstallPlan {
    pub game_coordinate: GameCoordinate,
    pub artifact: ResolvedArtifact,
    pub launch_profiles: Vec<LaunchProfile>,
    #[serde(default)]
    pub publisher_default_profile_id: Option<ProfileId>,
    pub source_event_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedArtifact {
    pub digest: Sha256Digest,
    pub package_format: PackageFormat,
    pub sources: Vec<ArtifactSource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameCoordinate {
    pub kind: u16,
    pub publisher_pubkey_hex: String,
    pub d_tag: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LaunchProfile {
    pub id: ProfileId,
    pub display_name: String,
    pub executable: RelativePath,
    pub working_dir: WorkingDir,
    pub arguments: Vec<String>,
    pub launch_type: LaunchType,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchType {
    Game,
    Config,
    Server,
    Editor,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageFormat {
    Zip,
    TarZstd,
    TarGz,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactSource {
    Adp { base_url: String },
}

/// A validated portable package path. The installation root (`.`) is excluded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct RelativePath(String);

impl RelativePath {
    pub fn parse(value: &str) -> Result<Self, PathError> {
        super::validation::validate_relative_path(value)?;
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RelativePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// A validated working directory, including `.` for the installation root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct WorkingDir(String);

impl WorkingDir {
    pub fn parse(value: &str) -> Result<Self, PathError> {
        if value == "." {
            return Ok(Self(value.to_owned()));
        }

        RelativePath::parse(value).map(|path| Self(path.0))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0 == "."
    }
}

impl<'de> Deserialize<'de> for WorkingDir {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ProfileId(String);

impl ProfileId {
    pub fn parse(value: &str) -> Result<Self, ProfileIdError> {
        super::validation::validate_profile_id(value)?;
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProfileId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn parse(value: &str) -> Result<Self, super::errors::DigestError> {
        super::validation::validate_sha256_digest(value)?;
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformTarget {
    pub os: String,
    pub arch: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformEligibility {
    pub targets: Vec<PlatformTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveOutcome {
    pub plan: ResolvedInstallPlan,
    pub discarded_profiles: Vec<DiscardedProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscardedProfile {
    pub raw_profile_id: Option<String>,
    pub reason: super::errors::ProfileRejection,
}

#[derive(Deserialize)]
struct UnvalidatedLaunchProfile {
    id: ProfileId,
    display_name: String,
    executable: RelativePath,
    working_dir: WorkingDir,
    arguments: Vec<String>,
    launch_type: LaunchType,
}

impl<'de> Deserialize<'de> for LaunchProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let profile = UnvalidatedLaunchProfile::deserialize(deserializer)?;
        super::validation::validate_display_name(&profile.display_name)
            .map_err(serde::de::Error::custom)?;
        super::validation::validate_arguments(&profile.arguments)
            .map_err(serde::de::Error::custom)?;

        Ok(Self {
            id: profile.id,
            display_name: profile.display_name,
            executable: profile.executable,
            working_dir: profile.working_dir,
            arguments: profile.arguments,
            launch_type: profile.launch_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{
        ArtifactSource, GameCoordinate, LaunchProfile, LaunchType, PackageFormat, ProfileId,
        RelativePath, ResolvedArtifact, ResolvedInstallPlan, Sha256Digest, WorkingDir,
    };

    #[test]
    fn resolved_plan_round_trips_with_transparent_newtype_strings() {
        let plan = sample_plan();
        let value = serde_json::to_value(&plan).expect("plan serializes");

        assert_eq!(value["artifact"]["digest"], Value::String("a".repeat(64)));
        assert_eq!(value["launch_profiles"][0]["id"], "default");
        assert_eq!(value["launch_profiles"][0]["executable"], "bin/Game.exe");
        assert_eq!(value["launch_profiles"][0]["working_dir"], ".");
        assert_eq!(value["publisher_default_profile_id"], "default");

        let decoded: ResolvedInstallPlan =
            serde_json::from_value(value.clone()).expect("validated plan deserializes");
        assert_eq!(decoded, plan);

        let mut legacy = value;
        legacy
            .as_object_mut()
            .expect("plan serializes as an object")
            .remove("publisher_default_profile_id");
        let decoded_legacy: ResolvedInstallPlan =
            serde_json::from_value(legacy).expect("legacy plan remains compatible");
        assert_eq!(decoded_legacy.publisher_default_profile_id, None);
    }

    #[test]
    fn hostile_relative_path_snapshots_fail_deserialization() {
        let traversal = snapshot_json(&"a".repeat(64), "default", "Play", "../escape", ".", "[]");
        let dot_executable = snapshot_json(&"a".repeat(64), "default", "Play", ".", ".", "[]");

        assert!(serde_json::from_str::<ResolvedInstallPlan>(&traversal).is_err());
        assert!(serde_json::from_str::<ResolvedInstallPlan>(&dot_executable).is_err());
    }

    #[test]
    fn every_validated_newtype_rejects_hostile_snapshot_values() {
        let uppercase_digest =
            snapshot_json(&"A".repeat(64), "default", "Play", "Game.exe", ".", "[]");
        let uppercase_profile_id =
            snapshot_json(&"a".repeat(64), "Default", "Play", "Game.exe", ".", "[]");
        let invalid_working_dir = snapshot_json(
            &"a".repeat(64),
            "default",
            "Play",
            "Game.exe",
            "../working",
            "[]",
        );

        assert!(serde_json::from_str::<ResolvedInstallPlan>(&uppercase_digest).is_err());
        assert!(serde_json::from_str::<ResolvedInstallPlan>(&uppercase_profile_id).is_err());
        assert!(serde_json::from_str::<ResolvedInstallPlan>(&invalid_working_dir).is_err());
    }

    #[test]
    fn launch_profile_snapshot_revalidates_display_name_and_arguments() {
        let long_display_name = snapshot_json(
            &"a".repeat(64),
            "default",
            &"a".repeat(65),
            "Game.exe",
            ".",
            "[]",
        );
        let too_many_arguments = serde_json::to_string(&vec!["arg"; 33]).expect("serializes");
        let invalid_arguments = snapshot_json(
            &"a".repeat(64),
            "default",
            "Play",
            "Game.exe",
            ".",
            &too_many_arguments,
        );

        assert!(serde_json::from_str::<ResolvedInstallPlan>(&long_display_name).is_err());
        assert!(serde_json::from_str::<ResolvedInstallPlan>(&invalid_arguments).is_err());
    }

    fn sample_plan() -> ResolvedInstallPlan {
        ResolvedInstallPlan {
            game_coordinate: GameCoordinate {
                kind: 30_402,
                publisher_pubkey_hex: "b".repeat(64),
                d_tag: "game".to_owned(),
            },
            artifact: ResolvedArtifact {
                digest: Sha256Digest::parse(&"a".repeat(64)).expect("valid digest"),
                package_format: PackageFormat::Zip,
                sources: vec![ArtifactSource::Adp {
                    base_url: "https://dist.example.com".to_owned(),
                }],
            },
            launch_profiles: vec![LaunchProfile {
                id: ProfileId::parse("default").expect("valid profile ID"),
                display_name: "Play".to_owned(),
                executable: RelativePath::parse("bin/Game.exe").expect("valid executable"),
                working_dir: WorkingDir::parse(".").expect("valid working directory"),
                arguments: vec!["--flag".to_owned()],
                launch_type: LaunchType::Game,
            }],
            publisher_default_profile_id: Some(
                ProfileId::parse("default").expect("valid profile ID"),
            ),
            source_event_json: "signed-event".to_owned(),
        }
    }

    fn snapshot_json(
        digest: &str,
        profile_id: &str,
        display_name: &str,
        executable: &str,
        working_dir: &str,
        arguments_json: &str,
    ) -> String {
        format!(
            r#"{{
                "game_coordinate": {{
                    "kind": 30402,
                    "publisher_pubkey_hex": "{publisher}",
                    "d_tag": "game"
                }},
                "artifact": {{
                    "digest": "{digest}",
                    "package_format": "Zip",
                    "sources": [{{"Adp": {{"base_url": "https://dist.example.com"}}}}]
                }},
                "launch_profiles": [{{
                    "id": "{profile_id}",
                    "display_name": "{display_name}",
                    "executable": "{executable}",
                    "working_dir": "{working_dir}",
                    "arguments": {arguments_json},
                    "launch_type": "Game"
                }}],
                "source_event_json": "signed-event"
            }}"#,
            publisher = "b".repeat(64),
        )
    }
}
