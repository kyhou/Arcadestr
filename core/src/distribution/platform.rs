// Canonical platform vocabulary and protocol-tag normalization.

use super::domain::PlatformTarget;

pub fn normalize_os(raw: &str) -> Option<String> {
    let canonical = match raw.to_ascii_lowercase().as_str() {
        "windows" | "win" | "win32" | "win64" => "windows",
        "linux" => "linux",
        "macos" | "osx" | "darwin" | "mac" => "macos",
        _ => return None,
    };

    Some(canonical.to_owned())
}

pub fn normalize_arch(raw: &str) -> Option<String> {
    let canonical = match raw.to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" | "x64" | "x86-64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        _ => return None,
    };

    Some(canonical.to_owned())
}

pub fn parse_platform_tag(value: &str) -> Option<PlatformTarget> {
    let (raw_os, raw_arch) = value.split_once('-')?;
    Some(PlatformTarget {
        os: normalize_os(raw_os)?,
        arch: normalize_arch(raw_arch)?,
    })
}

pub fn host_platform() -> PlatformTarget {
    PlatformTarget {
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{host_platform, normalize_arch, normalize_os, parse_platform_tag};
    use crate::distribution::PlatformTarget;

    #[test]
    fn os_canonical_values_and_every_alias_normalize_case_insensitively() {
        let cases = [
            ("windows", "windows"),
            ("linux", "linux"),
            ("macos", "macos"),
            ("win", "windows"),
            ("win32", "windows"),
            ("win64", "windows"),
            ("osx", "macos"),
            ("darwin", "macos"),
            ("mac", "macos"),
            ("WINDOWS", "windows"),
        ];

        for (raw, canonical) in cases {
            assert_eq!(
                normalize_os(raw),
                Some(canonical.to_owned()),
                "input: {raw}"
            );
        }
    }

    #[test]
    fn arch_canonical_values_and_every_alias_normalize_case_insensitively() {
        let cases = [
            ("x86_64", "x86_64"),
            ("aarch64", "aarch64"),
            ("amd64", "x86_64"),
            ("x64", "x86_64"),
            ("x86-64", "x86_64"),
            ("arm64", "aarch64"),
            ("X64", "x86_64"),
        ];

        for (raw, canonical) in cases {
            assert_eq!(
                normalize_arch(raw),
                Some(canonical.to_owned()),
                "input: {raw}"
            );
        }
    }

    #[test]
    fn platform_tag_splits_once_before_normalizing_both_components() {
        assert_eq!(
            parse_platform_tag("linux-x86-64"),
            Some(PlatformTarget {
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
            })
        );
        assert_eq!(
            parse_platform_tag("WINDOWS-X64"),
            Some(PlatformTarget {
                os: "windows".to_owned(),
                arch: "x86_64".to_owned(),
            })
        );
    }

    #[test]
    fn unrecognized_or_incomplete_platform_values_are_non_matches() {
        assert_eq!(normalize_os("freebsd"), None);
        assert_eq!(normalize_arch("riscv64"), None);
        assert_eq!(parse_platform_tag("freebsd-riscv64"), None);
        assert_eq!(parse_platform_tag("linux"), None);
        assert_eq!(parse_platform_tag("linux-"), None);
    }

    #[test]
    fn host_platform_returns_compile_time_target_values() {
        assert_eq!(
            host_platform(),
            PlatformTarget {
                os: std::env::consts::OS.to_owned(),
                arch: std::env::consts::ARCH.to_owned(),
            }
        );
    }
}
