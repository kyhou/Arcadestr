// Adapter from signed ADP listing events into protocol-neutral install plans.

use std::collections::HashSet;

use nostr::JsonUtil;

use super::domain::{
    ArtifactSource, DiscardedProfile, GameCoordinate, LaunchProfile, LaunchType, PackageFormat,
    PlatformEligibility, PlatformTarget, ProfileId, RelativePath, ResolveOutcome, ResolvedArtifact,
    ResolvedInstallPlan, Sha256Digest, WorkingDir,
};
use super::errors::{NoLaunchTierReason, ProfileIdError, ProfileRejection, ResolveError};
use super::platform::parse_platform_tag;
use super::validation::{parse_arguments_json, validate_display_name};

pub fn resolve_install_plan(
    event: &nostr::Event,
    host: &PlatformTarget,
) -> Result<ResolveOutcome, ResolveError> {
    let game_coordinate = resolve_coordinate(event)?;
    let digest = resolve_digest(event)?;
    resolve_launch_tier_claim(event)?;
    let package_format = resolve_package_format(event)?;
    require_launch_profiles(event)?;
    resolve_platform_eligibility(event, host)?;
    let (launch_profiles, discarded_profiles) = resolve_launch_profiles(event)?;
    let publisher_default_profile_id =
        resolve_publisher_default_profile_id(event, host, &launch_profiles);
    let sources = resolve_sources(event)?;

    Ok(ResolveOutcome {
        plan: ResolvedInstallPlan {
            game_coordinate,
            artifact: ResolvedArtifact {
                digest,
                package_format,
                sources,
            },
            launch_profiles,
            publisher_default_profile_id,
            source_event_json: event.as_json(),
        },
        discarded_profiles,
    })
}

fn resolve_publisher_default_profile_id(
    event: &nostr::Event,
    target: &PlatformTarget,
    launch_profiles: &[LaunchProfile],
) -> Option<ProfileId> {
    let tags = matching_tags(event, "launch_default");
    if tags.iter().any(|tag| tag.len() == 2) && tags.iter().any(|tag| tag.len() > 2) {
        return None;
    }
    let target = PlatformTarget {
        os: super::platform::normalize_os(&target.os)?,
        arch: super::platform::normalize_arch(&target.arch)?,
    };
    let mut claims = Vec::new();
    for tag in tags {
        if tag.len() < 2 {
            // A structurally broken legacy/global claim affects every target.
            claims.push(None);
            continue;
        }
        let target_claim_count = if tag.len() == 2 {
            1
        } else {
            tag.iter()
                .skip(2)
                .filter_map(|value| parse_platform_tag(value))
                .filter(|candidate| candidate == &target)
                .count()
        };
        if target_claim_count == 0 {
            continue;
        }
        if target_claim_count > 1 {
            claims.push(None);
            continue;
        }
        let profile_id = tag
            .get(1)
            .and_then(|raw_profile_id| ProfileId::parse(raw_profile_id).ok());
        let valid = profile_id.and_then(|profile_id| {
            launch_profiles
                .iter()
                .any(|profile| profile.id == profile_id && profile.launch_type == LaunchType::Game)
                .then_some(profile_id)
        });
        claims.push(valid);
    }
    let [claim] = claims.as_slice() else {
        return None;
    };
    claim.clone()
}

fn resolve_launch_tier_claim(event: &nostr::Event) -> Result<(), ResolveError> {
    let package_count = matching_tags(event, "package").len();
    let launch_count = matching_tags(event, "launch").len();

    if package_count == 0 && launch_count == 0 {
        return Err(ResolveError::NoLaunchTier {
            reason: NoLaunchTierReason::DownloadOnly,
        });
    }
    if package_count > 1 {
        return Err(ResolveError::NoLaunchTier {
            reason: NoLaunchTierReason::MultiplePackages,
        });
    }
    if package_count == 0 {
        return Err(ResolveError::NoLaunchTier {
            reason: NoLaunchTierReason::MissingPackage,
        });
    }

    Ok(())
}

fn require_launch_profiles(event: &nostr::Event) -> Result<(), ResolveError> {
    if matching_tags(event, "launch").is_empty() {
        return Err(ResolveError::NoLaunchTier {
            reason: NoLaunchTierReason::MissingLaunchProfiles,
        });
    }
    Ok(())
}

fn resolve_coordinate(event: &nostr::Event) -> Result<GameCoordinate, ResolveError> {
    if event.kind.as_u16() != 30_402 {
        return Err(ResolveError::MalformedCoordinate);
    }

    let d_tag = event
        .tags
        .identifier()
        .filter(|value| !value.is_empty())
        .ok_or(ResolveError::MalformedCoordinate)?;

    Ok(GameCoordinate {
        kind: event.kind.as_u16(),
        publisher_pubkey_hex: event.pubkey.to_hex(),
        d_tag: d_tag.to_owned(),
    })
}

fn resolve_digest(event: &nostr::Event) -> Result<Sha256Digest, ResolveError> {
    let value = first_tag_value(event, "file_hash").ok_or_else(|| ResolveError::NoLaunchTier {
        reason: NoLaunchTierReason::MissingFileHash,
    })?;

    Sha256Digest::parse(value).map_err(|_| ResolveError::NoLaunchTier {
        reason: NoLaunchTierReason::InvalidFileHash,
    })
}

fn resolve_package_format(event: &nostr::Event) -> Result<PackageFormat, ResolveError> {
    let package_tags = matching_tags(event, "package");
    if package_tags.is_empty() {
        return Err(ResolveError::NoLaunchTier {
            reason: NoLaunchTierReason::MissingPackage,
        });
    }
    if package_tags.len() > 1 {
        return Err(ResolveError::NoLaunchTier {
            reason: NoLaunchTierReason::MultiplePackages,
        });
    }

    let declared = package_tags[0].get(1).map_or("", String::as_str);
    match declared {
        "zip" => Ok(PackageFormat::Zip),
        "tar.zst" => Ok(PackageFormat::TarZstd),
        "tar.gz" => Ok(PackageFormat::TarGz),
        _ => Err(ResolveError::UnsupportedPackageFormat {
            declared: declared.to_owned(),
        }),
    }
}

fn resolve_platform_eligibility(
    event: &nostr::Event,
    host: &PlatformTarget,
) -> Result<(), ResolveError> {
    let platform_tags = matching_tags(event, "platform");
    if platform_tags.is_empty() {
        return Err(ResolveError::NoLaunchTier {
            reason: NoLaunchTierReason::MissingPlatform,
        });
    }

    let declared = platform_tags
        .iter()
        .map(|tag| tag.get(1).map_or("", String::as_str).to_owned())
        .collect::<Vec<_>>();
    let eligibility = PlatformEligibility {
        targets: declared
            .iter()
            .filter_map(|value| parse_platform_tag(value))
            .collect(),
    };

    if eligibility.targets.iter().any(|target| target == host) {
        return Ok(());
    }

    Err(ResolveError::UnsupportedPlatform {
        declared,
        host: host.clone(),
    })
}

fn resolve_launch_profiles(
    event: &nostr::Event,
) -> Result<(Vec<LaunchProfile>, Vec<DiscardedProfile>), ResolveError> {
    let parsed = matching_tags(event, "launch")
        .into_iter()
        .map(parse_launch_tag)
        .collect::<Vec<_>>();

    reject_duplicate_profile_ids(&parsed)?;

    let mut profiles = Vec::new();
    let mut discarded = Vec::new();
    for parsed_profile in parsed {
        match parsed_profile.profile {
            Ok(profile) => profiles.push(profile),
            Err(reason) => discarded.push(DiscardedProfile {
                raw_profile_id: parsed_profile.raw_profile_id,
                reason,
            }),
        }
    }

    if !profiles
        .iter()
        .any(|profile| profile.launch_type == LaunchType::Game)
    {
        return Err(ResolveError::NoLaunchTier {
            reason: NoLaunchTierReason::NoGameProfile,
        });
    }

    Ok((profiles, discarded))
}

/// Policy-neutral parse result for one raw positional `launch` tag.
pub struct ParsedLaunchTag {
    /// The claimed ID exactly as supplied, including an invalid spelling.
    pub raw_profile_id: Option<String>,
    /// The validated ID used by callers for duplicate policy.
    pub valid_profile_id: Option<ProfileId>,
    /// The validated profile or its individual rejection reason.
    pub profile: Result<LaunchProfile, ProfileRejection>,
}

/// Parses one raw positional `launch` tag without applying discard or tier policy.
pub fn parse_launch_tag(tag: &[String]) -> ParsedLaunchTag {
    let raw_profile_id = tag.get(1).cloned();
    let valid_profile_id = raw_profile_id
        .as_deref()
        .and_then(|value| ProfileId::parse(value).ok());
    let profile = parse_launch_profile(tag, valid_profile_id.clone());

    ParsedLaunchTag {
        raw_profile_id,
        valid_profile_id,
        profile,
    }
}

/// Parses a launch profile after independently validating its claimed ID.
pub fn parse_launch_profile(
    tag: &[String],
    valid_profile_id: Option<ProfileId>,
) -> Result<LaunchProfile, ProfileRejection> {
    if tag.len() < 7 {
        return Err(ProfileRejection::MissingRequiredElement);
    }

    let id = valid_profile_id.ok_or(ProfileRejection::ProfileId(ProfileIdError::InvalidGrammar))?;
    let executable = RelativePath::parse(&tag[2]).map_err(ProfileRejection::Executable)?;
    let working_dir = WorkingDir::parse(&tag[3]).map_err(ProfileRejection::WorkingDir)?;
    let arguments = parse_arguments_json(&tag[4]).map_err(ProfileRejection::Arguments)?;
    let launch_type = parse_launch_type(&tag[5]);
    validate_display_name(&tag[6]).map_err(ProfileRejection::DisplayName)?;

    Ok(LaunchProfile {
        id,
        display_name: tag[6].clone(),
        executable,
        working_dir,
        arguments,
        launch_type,
    })
}

fn reject_duplicate_profile_ids(parsed: &[ParsedLaunchTag]) -> Result<(), ResolveError> {
    let mut seen = HashSet::new();
    for id in parsed
        .iter()
        .filter_map(|profile| profile.valid_profile_id.as_ref())
    {
        if !seen.insert(id.as_str()) {
            return Err(ResolveError::NoLaunchTier {
                reason: NoLaunchTierReason::DuplicateProfileId {
                    profile_id: id.as_str().to_owned(),
                },
            });
        }
    }

    Ok(())
}

/// Maps a declared launch type, treating unknown values as [`LaunchType::Tool`].
pub fn parse_launch_type(value: &str) -> LaunchType {
    match value {
        "game" => LaunchType::Game,
        "config" => LaunchType::Config,
        "server" => LaunchType::Server,
        "editor" => LaunchType::Editor,
        "tool" => LaunchType::Tool,
        _ => LaunchType::Tool,
    }
}

fn resolve_sources(event: &nostr::Event) -> Result<Vec<ArtifactSource>, ResolveError> {
    let sources = matching_tags(event, "server")
        .into_iter()
        .filter_map(|tag| tag.get(1))
        .map(|base_url| ArtifactSource::Adp {
            base_url: base_url.clone(),
        })
        .collect::<Vec<_>>();

    if sources.is_empty() {
        return Err(ResolveError::NoSource);
    }

    Ok(sources)
}

fn first_tag_value<'a>(event: &'a nostr::Event, name: &str) -> Option<&'a str> {
    matching_tags(event, name)
        .into_iter()
        .next()
        .and_then(|tag| tag.get(1))
        .map(String::as_str)
}

fn matching_tags<'a>(event: &'a nostr::Event, name: &str) -> Vec<&'a [String]> {
    event
        .tags
        .iter()
        .map(nostr::Tag::as_slice)
        .filter(move |tag| tag.first().is_some_and(|value| value == name))
        .collect()
}

#[cfg(test)]
mod tests {
    use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag};

    use super::resolve_install_plan;
    use crate::distribution::{
        LaunchType, NoLaunchTierReason, PathError, PlatformTarget, ProfileId, ProfileRejection,
        ResolveError,
    };

    type RawTag = Vec<String>;

    #[test]
    fn case_01_three_valid_profiles_with_one_game_are_all_resolved() {
        let mut tags = base_tags();
        tags.push(launch_tag(
            "config",
            "Config.exe",
            ".",
            "[]",
            "config",
            "Configure",
        ));
        tags.push(launch_tag(
            "server",
            "Server.exe",
            ".",
            "[]",
            "server",
            "Host",
        ));

        let event = signed_event(tags);
        let outcome = resolve_install_plan(&event, &linux_host()).expect("listing resolves");

        assert_eq!(outcome.plan.launch_profiles.len(), 3);
        assert_eq!(outcome.plan.publisher_default_profile_id, None);
        assert!(outcome.discarded_profiles.is_empty());
        assert_eq!(
            outcome
                .plan
                .launch_profiles
                .iter()
                .filter(|profile| profile.launch_type == LaunchType::Game)
                .count(),
            1
        );

        let decoded = Event::from_json(&outcome.plan.source_event_json).expect("event JSON parses");
        assert_eq!(decoded, event);
        assert!(decoded.verify().is_ok());
    }

    #[test]
    fn case_02_invalid_profile_path_is_discarded_without_failing_tier() {
        let mut tags = base_tags();
        tags.push(launch_tag(
            "broken",
            "../escape",
            ".",
            "[]",
            "config",
            "Broken",
        ));

        let outcome =
            resolve_install_plan(&signed_event(tags), &linux_host()).expect("tier survives");

        assert_eq!(outcome.plan.launch_profiles.len(), 1);
        assert_eq!(outcome.discarded_profiles.len(), 1);
        assert_eq!(
            outcome.discarded_profiles[0].raw_profile_id.as_deref(),
            Some("broken")
        );
        assert_eq!(
            outcome.discarded_profiles[0].reason,
            ProfileRejection::Executable(PathError::Rule2ParentSegment)
        );
    }

    #[test]
    fn case_03_duplicate_valid_profile_ids_fail_the_launch_tier() {
        let mut tags = base_tags();
        tags.push(launch_tag(
            "default",
            "Config.exe",
            ".",
            "[]",
            "config",
            "Configure",
        ));

        assert_duplicate_default(resolve_install_plan(&signed_event(tags), &linux_host()));
    }

    #[test]
    fn case_04_duplicate_valid_id_fails_even_when_one_profile_is_otherwise_invalid() {
        let mut tags = base_tags();
        tags.push(launch_tag(
            "default",
            "../escape",
            ".",
            "[]",
            "config",
            "Broken",
        ));

        assert_duplicate_default(resolve_install_plan(&signed_event(tags), &linux_host()));
    }

    #[test]
    fn case_05_config_only_listing_has_no_launch_tier() {
        let mut tags = base_tags();
        find_tag_mut(&mut tags, "launch")[5] = "config".to_owned();

        assert_eq!(
            resolve_install_plan(&signed_event(tags), &linux_host()).unwrap_err(),
            ResolveError::NoLaunchTier {
                reason: NoLaunchTierReason::NoGameProfile,
            }
        );
    }

    #[test]
    fn download_only_listing_has_an_intentional_capability_reason() {
        let mut tags = base_tags();
        remove_tags(&mut tags, "package");
        remove_tags(&mut tags, "launch");
        remove_tags(&mut tags, "platform");

        assert_eq!(
            resolve_install_plan(&signed_event(tags), &linux_host()).unwrap_err(),
            ResolveError::NoLaunchTier {
                reason: NoLaunchTierReason::DownloadOnly,
            }
        );
    }

    #[test]
    fn package_without_launch_profiles_is_incomplete_not_download_only() {
        let mut tags = base_tags();
        remove_tags(&mut tags, "launch");

        assert_eq!(
            resolve_install_plan(&signed_event(tags), &linux_host()).unwrap_err(),
            ResolveError::NoLaunchTier {
                reason: NoLaunchTierReason::MissingLaunchProfiles,
            }
        );
    }

    #[test]
    fn launch_profiles_without_package_are_incomplete() {
        let mut tags = base_tags();
        remove_tags(&mut tags, "package");

        assert_eq!(
            resolve_install_plan(&signed_event(tags), &linux_host()).unwrap_err(),
            ResolveError::NoLaunchTier {
                reason: NoLaunchTierReason::MissingPackage,
            }
        );
    }

    #[test]
    fn case_06_unknown_launch_type_becomes_tool_without_discard() {
        let mut tags = base_tags();
        tags.push(launch_tag(
            "mystery",
            "Tool.exe",
            ".",
            "[]",
            "future-type",
            "Mystery",
        ));

        let outcome =
            resolve_install_plan(&signed_event(tags), &linux_host()).expect("listing resolves");
        let mystery = outcome
            .plan
            .launch_profiles
            .iter()
            .find(|profile| profile.id.as_str() == "mystery")
            .expect("unknown profile retained");

        assert_eq!(mystery.launch_type, LaunchType::Tool);
        assert!(outcome.discarded_profiles.is_empty());
    }

    #[test]
    fn case_07_eighth_tag_element_is_ignored() {
        let mut tags = base_tags();
        find_tag_mut(&mut tags, "launch").push("future-extension".to_owned());

        let outcome =
            resolve_install_plan(&signed_event(tags), &linux_host()).expect("listing resolves");

        assert_eq!(outcome.plan.launch_profiles.len(), 1);
        assert!(outcome.discarded_profiles.is_empty());
    }

    #[test]
    fn case_08_six_element_launch_tag_is_discarded() {
        let mut tags = base_tags();
        tags.push(
            ["launch", "short", "Tool.exe", ".", "[]", "config"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        );

        let outcome =
            resolve_install_plan(&signed_event(tags), &linux_host()).expect("tier survives");

        assert_eq!(outcome.discarded_profiles.len(), 1);
        assert_eq!(
            outcome.discarded_profiles[0].raw_profile_id.as_deref(),
            Some("short")
        );
        assert_eq!(
            outcome.discarded_profiles[0].reason,
            ProfileRejection::MissingRequiredElement
        );
    }

    #[test]
    fn case_09_two_package_tags_have_no_launch_tier() {
        let mut tags = base_tags();
        tags.push(raw_tag(&["package", "tar.gz"]));

        assert_eq!(
            resolve_install_plan(&signed_event(tags), &linux_host()).unwrap_err(),
            ResolveError::NoLaunchTier {
                reason: NoLaunchTierReason::MultiplePackages,
            }
        );
    }

    #[test]
    fn case_10_missing_platform_tag_has_no_launch_tier() {
        let mut tags = base_tags();
        remove_tags(&mut tags, "platform");

        assert_eq!(
            resolve_install_plan(&signed_event(tags), &linux_host()).unwrap_err(),
            ResolveError::NoLaunchTier {
                reason: NoLaunchTierReason::MissingPlatform,
            }
        );
    }

    #[test]
    fn case_11_declared_platform_without_host_match_is_unsupported() {
        let mut tags = base_tags();
        find_tag_mut(&mut tags, "platform")[1] = "macos-aarch64".to_owned();
        let host = linux_host();

        assert_eq!(
            resolve_install_plan(&signed_event(tags), &host).unwrap_err(),
            ResolveError::UnsupportedPlatform {
                declared: vec!["macos-aarch64".to_owned()],
                host,
            }
        );
    }

    #[test]
    fn case_12_windows_x64_alias_matches_x86_64_host() {
        let mut tags = base_tags();
        find_tag_mut(&mut tags, "platform")[1] = "windows-x64".to_owned();
        let host = PlatformTarget {
            os: "windows".to_owned(),
            arch: "x86_64".to_owned(),
        };

        assert!(resolve_install_plan(&signed_event(tags), &host).is_ok());
    }

    #[test]
    fn case_13_unknown_platform_is_unsupported_not_a_parse_error() {
        let mut tags = base_tags();
        find_tag_mut(&mut tags, "platform")[1] = "freebsd-riscv64".to_owned();
        let host = linux_host();

        assert_eq!(
            resolve_install_plan(&signed_event(tags), &host).unwrap_err(),
            ResolveError::UnsupportedPlatform {
                declared: vec!["freebsd-riscv64".to_owned()],
                host,
            }
        );
    }

    #[test]
    fn case_14_dot_executable_is_discarded_while_another_game_keeps_tier_alive() {
        let mut tags = base_tags();
        tags.push(launch_tag("broken", ".", ".", "[]", "game", "Broken"));

        let outcome =
            resolve_install_plan(&signed_event(tags), &linux_host()).expect("tier survives");

        assert_eq!(outcome.plan.launch_profiles.len(), 1);
        assert_eq!(outcome.discarded_profiles.len(), 1);
        assert_eq!(
            outcome.discarded_profiles[0].raw_profile_id.as_deref(),
            Some("broken")
        );
        assert_eq!(
            outcome.discarded_profiles[0].reason,
            ProfileRejection::Executable(PathError::Rule6EmptyOrDotSegment)
        );
    }

    #[test]
    fn case_15_dot_working_directory_is_a_valid_root() {
        let outcome = resolve_install_plan(&signed_event(base_tags()), &linux_host())
            .expect("listing resolves");

        assert_eq!(outcome.plan.launch_profiles.len(), 1);
        assert!(outcome.plan.launch_profiles[0].working_dir.is_root());
        assert_eq!(outcome.plan.launch_profiles[0].working_dir.as_str(), ".");
    }

    #[test]
    fn publisher_default_resolves_only_for_one_well_formed_game_reference() {
        let mut valid = base_tags();
        valid.push(raw_tag(&["launch_default", "default"]));
        let outcome = resolve_install_plan(&signed_event(valid), &linux_host())
            .expect("valid publisher default resolves");
        assert_eq!(
            outcome
                .plan
                .publisher_default_profile_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("default")
        );

        let malformed_defaults = [
            vec![raw_tag(&["launch_default"])],
            vec![raw_tag(&["launch_default", "Bad"])],
            vec![raw_tag(&["launch_default", "default", "extra"])],
            vec![
                raw_tag(&["launch_default", "default"]),
                raw_tag(&["launch_default", "default"]),
            ],
            vec![raw_tag(&["launch_default", "missing"])],
        ];
        for launch_defaults in malformed_defaults {
            let mut tags = base_tags();
            tags.extend(launch_defaults);
            let outcome = resolve_install_plan(&signed_event(tags), &linux_host())
                .expect("invalid preference does not suppress launch tier");
            assert_eq!(outcome.plan.publisher_default_profile_id, None);
            assert_eq!(outcome.plan.launch_profiles.len(), 1);
        }

        let mut non_game = base_tags();
        non_game.push(launch_tag(
            "configure",
            "Config.exe",
            ".",
            "[]",
            "config",
            "Configure",
        ));
        non_game.push(raw_tag(&["launch_default", "configure"]));
        let outcome = resolve_install_plan(&signed_event(non_game), &linux_host())
            .expect("non-game preference does not suppress launch tier");
        assert_eq!(outcome.plan.publisher_default_profile_id, None);
        assert_eq!(outcome.plan.launch_profiles.len(), 2);
    }

    #[test]
    fn scoped_publisher_defaults_match_the_full_normalized_target() {
        let mut tags = base_tags();
        tags.push(raw_tag(&["platform", "linux-aarch64"]));
        tags.push(launch_tag(
            "arm-play",
            "bin/arm-game",
            ".",
            "[]",
            "game",
            "Play ARM",
        ));
        tags.extend([
            raw_tag(&["launch_default", "default", "LINUX-amd64"]),
            raw_tag(&["launch_default", "arm-play", "linux-arm64"]),
        ]);

        let x86 = resolve_install_plan(&signed_event(tags.clone()), &linux_host())
            .expect("x86 target resolves");
        assert_eq!(
            x86.plan
                .publisher_default_profile_id
                .as_ref()
                .map(ProfileId::as_str),
            Some("default")
        );
        let arm = resolve_install_plan(
            &signed_event(tags),
            &PlatformTarget {
                os: "linux".to_owned(),
                arch: "aarch64".to_owned(),
            },
        )
        .expect("ARM target resolves");
        assert_eq!(
            arm.plan
                .publisher_default_profile_id
                .as_ref()
                .map(ProfileId::as_str),
            Some("arm-play")
        );
    }

    #[test]
    fn defects_for_other_targets_do_not_suppress_current_target_default() {
        let mut tags = base_tags();
        tags.extend([
            raw_tag(&["launch_default", "default", "linux-x86_64"]),
            raw_tag(&["launch_default", "missing", "linux-aarch64"]),
            raw_tag(&["launch_default", "Bad", "unknown-target"]),
        ]);

        let outcome = resolve_install_plan(&signed_event(tags), &linux_host())
            .expect("launch tier survives unrelated defects");
        assert_eq!(
            outcome
                .plan
                .publisher_default_profile_id
                .as_ref()
                .map(ProfileId::as_str),
            Some("default")
        );
    }

    #[test]
    fn mixed_global_and_scoped_defaults_are_ignored_for_every_target() {
        let mut tags = base_tags();
        tags.extend([
            raw_tag(&["launch_default", "default"]),
            raw_tag(&["launch_default", "default", "linux-aarch64"]),
        ]);

        let outcome = resolve_install_plan(&signed_event(tags), &linux_host())
            .expect("mixed default scopes do not suppress the launch tier");
        assert_eq!(outcome.plan.publisher_default_profile_id, None);
    }

    #[test]
    fn duplicate_or_malformed_current_target_claim_resolves_to_none() {
        for extra in [
            raw_tag(&["launch_default", "default", "linux-x86_64"]),
            raw_tag(&["launch_default", "missing", "linux-x86_64"]),
            raw_tag(&["launch_default", "default", "linux-x86_64", "linux-amd64"]),
        ] {
            let mut tags = base_tags();
            tags.push(raw_tag(&["launch_default", "default", "linux-x86_64"]));
            tags.push(extra);
            let outcome = resolve_install_plan(&signed_event(tags), &linux_host())
                .expect("default defects do not suppress the launch tier");
            assert_eq!(outcome.plan.publisher_default_profile_id, None);
        }
    }

    fn assert_duplicate_default(result: Result<crate::distribution::ResolveOutcome, ResolveError>) {
        assert_eq!(
            result.unwrap_err(),
            ResolveError::NoLaunchTier {
                reason: NoLaunchTierReason::DuplicateProfileId {
                    profile_id: "default".to_owned(),
                },
            }
        );
    }

    fn base_tags() -> Vec<RawTag> {
        vec![
            raw_tag(&["d", "game"]),
            vec!["file_hash".to_owned(), "a".repeat(64)],
            raw_tag(&["package", "zip"]),
            raw_tag(&["platform", "linux-x86_64"]),
            launch_tag("default", "bin/Game.exe", ".", "[]", "game", "Play"),
            raw_tag(&["server", "https://dist.example.com"]),
        ]
    }

    fn launch_tag(
        id: &str,
        executable: &str,
        working_dir: &str,
        arguments: &str,
        launch_type: &str,
        display_name: &str,
    ) -> RawTag {
        raw_tag(&[
            "launch",
            id,
            executable,
            working_dir,
            arguments,
            launch_type,
            display_name,
        ])
    }

    fn raw_tag(values: &[&str]) -> RawTag {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn signed_event(tags: Vec<RawTag>) -> Event {
        let parsed_tags = tags
            .into_iter()
            .map(|values| Tag::parse(values).expect("fixture tag parses"));

        EventBuilder::new(Kind::Custom(30_402), "listing")
            .tags(parsed_tags)
            .sign_with_keys(&Keys::generate())
            .expect("fixture event signs")
    }

    fn linux_host() -> PlatformTarget {
        PlatformTarget {
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
        }
    }

    fn find_tag_mut<'a>(tags: &'a mut [RawTag], name: &str) -> &'a mut RawTag {
        tags.iter_mut()
            .find(|tag| tag.first().is_some_and(|value| value == name))
            .expect("fixture tag exists")
    }

    fn remove_tags(tags: &mut Vec<RawTag>, name: &str) {
        tags.retain(|tag| !tag.first().is_some_and(|value| value == name));
    }
}
