// Pure validators for protocol-neutral distribution values.

use super::errors::{ArgumentError, DigestError, DisplayNameError, PathError, ProfileIdError};

const MAX_PATH_CHARACTERS: usize = 200;
const MAX_SEGMENT_CHARACTERS: usize = 100;
const MAX_ARGUMENTS: usize = 32;
const MAX_ARGUMENT_CHARACTERS: usize = 1024;
const MAX_DISPLAY_NAME_CHARACTERS: usize = 64;
const MAX_PROFILE_ID_CHARACTERS: usize = 64;

pub fn parse_arguments_json(value: &str) -> Result<Vec<String>, ArgumentError> {
    let arguments =
        serde_json::from_str::<Vec<String>>(value).map_err(|_| ArgumentError::InvalidJsonShape)?;
    validate_arguments(&arguments)?;
    Ok(arguments)
}

pub fn validate_arguments(arguments: &[String]) -> Result<(), ArgumentError> {
    if arguments.len() > MAX_ARGUMENTS {
        return Err(ArgumentError::TooMany);
    }

    if arguments
        .iter()
        .any(|argument| argument.chars().count() > MAX_ARGUMENT_CHARACTERS)
    {
        return Err(ArgumentError::ElementTooLong);
    }

    if arguments.iter().any(|argument| argument.contains('\0')) {
        return Err(ArgumentError::ContainsNul);
    }

    Ok(())
}

pub fn validate_display_name(value: &str) -> Result<(), DisplayNameError> {
    let character_count = value.chars().count();
    if !(1..=MAX_DISPLAY_NAME_CHARACTERS).contains(&character_count) {
        return Err(DisplayNameError::InvalidLength);
    }

    // Unicode control characters include C0, DEL, and C1. Format characters
    // such as ZWJ and RTL marks remain valid protocol text.
    if value.chars().any(char::is_control) {
        return Err(DisplayNameError::ControlCharacter);
    }

    Ok(())
}

pub(super) fn validate_profile_id(value: &str) -> Result<(), ProfileIdError> {
    let bytes = value.as_bytes();
    let Some((first, rest)) = bytes.split_first() else {
        return Err(ProfileIdError::InvalidGrammar);
    };

    let first_is_valid = first.is_ascii_lowercase() || first.is_ascii_digit();
    let rest_is_valid = rest.iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
    });

    if bytes.len() > MAX_PROFILE_ID_CHARACTERS || !first_is_valid || !rest_is_valid {
        return Err(ProfileIdError::InvalidGrammar);
    }

    Ok(())
}

pub(super) fn validate_sha256_digest(value: &str) -> Result<(), DigestError> {
    let is_lowercase_hex = value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));

    if !crate::is_sha256_hex(value) || !is_lowercase_hex {
        return Err(DigestError::InvalidSha256);
    }

    Ok(())
}

pub(super) fn validate_relative_path(value: &str) -> Result<(), PathError> {
    if value.starts_with('/') {
        return Err(PathError::Rule1Absolute);
    }

    let segments: Vec<&str> = value.split('/').collect();

    if segments.contains(&"..") {
        return Err(PathError::Rule2ParentSegment);
    }

    if value.contains('\\') {
        return Err(PathError::Rule3Backslash);
    }

    if value.contains(':') {
        return Err(PathError::Rule4ColonOrPrefix);
    }

    if value.chars().any(|character| character <= '\u{001f}') {
        return Err(PathError::Rule5ControlCharacter);
    }

    if segments
        .iter()
        .any(|segment| segment.is_empty() || *segment == ".")
    {
        return Err(PathError::Rule6EmptyOrDotSegment);
    }

    if segments
        .iter()
        .any(|segment| segment.ends_with(' ') || segment.ends_with('.'))
    {
        return Err(PathError::Rule7TrailingSpaceOrPeriod);
    }

    if segments.iter().any(|segment| is_reserved_name(segment)) {
        return Err(PathError::Rule8ReservedDeviceName);
    }

    if value.chars().count() > MAX_PATH_CHARACTERS
        || segments
            .iter()
            .any(|segment| segment.chars().count() > MAX_SEGMENT_CHARACTERS)
    {
        return Err(PathError::Rule9LengthExceeded);
    }

    Ok(())
}

fn is_reserved_name(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or_default();
    let uppercase = stem.to_ascii_uppercase();

    matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || has_reserved_numbered_prefix(&uppercase, "COM")
        || has_reserved_numbered_prefix(&uppercase, "LPT")
}

fn has_reserved_numbered_prefix(value: &str, prefix: &str) -> bool {
    value
        .strip_prefix(prefix)
        .is_some_and(|suffix| matches!(suffix.as_bytes(), [b'0'..=b'9']))
}

#[cfg(test)]
mod tests {
    use super::super::{ProfileId, RelativePath, Sha256Digest, WorkingDir};
    use super::{
        parse_arguments_json, validate_display_name, ArgumentError, DigestError, DisplayNameError,
        PathError, ProfileIdError,
    };

    #[test]
    fn arguments_accept_json_string_arrays_at_both_limits() {
        assert_eq!(parse_arguments_json("[]"), Ok(Vec::new()));
        assert_eq!(
            parse_arguments_json(r#"["--flag"]"#),
            Ok(vec!["--flag".to_owned()])
        );

        let thirty_two = vec!["argument"; 32];
        let thirty_three = vec!["argument"; 33];
        assert_eq!(
            parse_arguments_json(&serde_json::to_string(&thirty_two).expect("serializes")),
            Ok(thirty_two.into_iter().map(str::to_owned).collect())
        );
        assert_eq!(
            parse_arguments_json(&serde_json::to_string(&thirty_three).expect("serializes")),
            Err(ArgumentError::TooMany)
        );

        let argument_1024 = "a".repeat(1024);
        let argument_1025 = "a".repeat(1025);
        assert_eq!(
            parse_arguments_json(&serde_json::to_string(&[&argument_1024]).expect("serializes")),
            Ok(vec![argument_1024])
        );
        assert_eq!(
            parse_arguments_json(&serde_json::to_string(&[argument_1025]).expect("serializes")),
            Err(ArgumentError::ElementTooLong)
        );
    }

    #[test]
    fn arguments_reject_non_string_arrays_non_arrays_malformed_json_and_nul() {
        for value in [
            r#"["--flag", 3]"#,
            r#"{"a":1}"#,
            r#""--flag""#,
            r#"["--flag""#,
        ] {
            assert_eq!(
                parse_arguments_json(value),
                Err(ArgumentError::InvalidJsonShape),
                "input: {value}"
            );
        }

        let nul_argument = serde_json::to_string(&["a\0b"]).expect("serializes");
        assert_eq!(
            parse_arguments_json(&nul_argument),
            Err(ArgumentError::ContainsNul)
        );
    }

    #[test]
    fn profile_id_enforces_grammar_and_both_length_boundaries() {
        let id_64 = format!("a{}", "b".repeat(63));
        let id_65 = format!("a{}", "b".repeat(64));

        assert_eq!(
            ProfileId::parse("a").map(|id| id.as_str().to_owned()),
            Ok("a".to_owned())
        );
        assert_eq!(
            ProfileId::parse(&id_64).map(|id| id.as_str().to_owned()),
            Ok(id_64)
        );
        assert_eq!(
            ProfileId::parse(&id_65),
            Err(ProfileIdError::InvalidGrammar)
        );

        for invalid in ["", "-game", "Game", "game.profile", "gameé"] {
            assert_eq!(
                ProfileId::parse(invalid),
                Err(ProfileIdError::InvalidGrammar),
                "input: {invalid}"
            );
        }
    }

    #[test]
    fn display_name_enforces_length_and_unicode_control_rules() {
        assert_eq!(validate_display_name("A"), Ok(()));
        assert_eq!(validate_display_name(&"a".repeat(64)), Ok(()));
        assert_eq!(
            validate_display_name(""),
            Err(DisplayNameError::InvalidLength)
        );
        assert_eq!(
            validate_display_name(&"a".repeat(65)),
            Err(DisplayNameError::InvalidLength)
        );

        for control in ['\0', '\n', '\u{007f}', '\u{0085}'] {
            assert_eq!(
                validate_display_name(&format!("A{control}")),
                Err(DisplayNameError::ControlCharacter)
            );
        }

        assert_eq!(validate_display_name("A\u{200d}\u{200f}"), Ok(()));
    }

    #[test]
    fn sha256_digest_reuses_shared_validation_and_requires_lowercase() {
        let lowercase = "a".repeat(64);
        let uppercase = "A".repeat(64);

        assert_eq!(
            Sha256Digest::parse(&lowercase).map(|digest| digest.as_str().to_owned()),
            Ok(lowercase)
        );
        assert!(crate::is_sha256_hex(&uppercase));
        assert_eq!(
            Sha256Digest::parse(&uppercase),
            Err(DigestError::InvalidSha256)
        );
        assert_eq!(
            Sha256Digest::parse(&"a".repeat(63)),
            Err(DigestError::InvalidSha256)
        );
        assert_eq!(
            Sha256Digest::parse(&format!("{}g", "a".repeat(63))),
            Err(DigestError::InvalidSha256)
        );
    }

    #[test]
    fn portable_path_rules_cover_every_required_case_for_both_path_types() {
        let path_201 = "a".repeat(201);
        let segment_101 = format!("{}/b", "a".repeat(101));
        let cases = vec![
            ("bin/Game.exe".to_owned(), None, None),
            (
                ".".to_owned(),
                Some(PathError::Rule6EmptyOrDotSegment),
                None,
            ),
            (
                "./bin".to_owned(),
                Some(PathError::Rule6EmptyOrDotSegment),
                Some(PathError::Rule6EmptyOrDotSegment),
            ),
            (
                "/bin/Game.exe".to_owned(),
                Some(PathError::Rule1Absolute),
                Some(PathError::Rule1Absolute),
            ),
            (
                "../escape".to_owned(),
                Some(PathError::Rule2ParentSegment),
                Some(PathError::Rule2ParentSegment),
            ),
            (
                "a/../../escape".to_owned(),
                Some(PathError::Rule2ParentSegment),
                Some(PathError::Rule2ParentSegment),
            ),
            (
                "a\\..\\..\\escape".to_owned(),
                Some(PathError::Rule3Backslash),
                Some(PathError::Rule3Backslash),
            ),
            (
                "bin\\Game.exe".to_owned(),
                Some(PathError::Rule3Backslash),
                Some(PathError::Rule3Backslash),
            ),
            (
                "C:/Game.exe".to_owned(),
                Some(PathError::Rule4ColonOrPrefix),
                Some(PathError::Rule4ColonOrPrefix),
            ),
            (
                "bin/Game\0.exe".to_owned(),
                Some(PathError::Rule5ControlCharacter),
                Some(PathError::Rule5ControlCharacter),
            ),
            (
                "a//b".to_owned(),
                Some(PathError::Rule6EmptyOrDotSegment),
                Some(PathError::Rule6EmptyOrDotSegment),
            ),
            (
                "a/./b".to_owned(),
                Some(PathError::Rule6EmptyOrDotSegment),
                Some(PathError::Rule6EmptyOrDotSegment),
            ),
            (
                "bin./Game.exe".to_owned(),
                Some(PathError::Rule7TrailingSpaceOrPeriod),
                Some(PathError::Rule7TrailingSpaceOrPeriod),
            ),
            (
                "bin /Game.exe".to_owned(),
                Some(PathError::Rule7TrailingSpaceOrPeriod),
                Some(PathError::Rule7TrailingSpaceOrPeriod),
            ),
            (
                "CON/Game.exe".to_owned(),
                Some(PathError::Rule8ReservedDeviceName),
                Some(PathError::Rule8ReservedDeviceName),
            ),
            (
                "con.txt".to_owned(),
                Some(PathError::Rule8ReservedDeviceName),
                Some(PathError::Rule8ReservedDeviceName),
            ),
            (
                "COM1".to_owned(),
                Some(PathError::Rule8ReservedDeviceName),
                Some(PathError::Rule8ReservedDeviceName),
            ),
            ("console.txt".to_owned(), None, None),
            (
                path_201,
                Some(PathError::Rule9LengthExceeded),
                Some(PathError::Rule9LengthExceeded),
            ),
            (
                segment_101,
                Some(PathError::Rule9LengthExceeded),
                Some(PathError::Rule9LengthExceeded),
            ),
        ];

        assert_eq!(
            cases.len(),
            20,
            "the required path table must remain complete"
        );

        let mut relative_assertions = 0;
        let mut working_dir_assertions = 0;
        let mut covered_rules = [false; 9];

        for (input, relative_error, working_dir_error) in &cases {
            mark_covered_rule(&mut covered_rules, relative_error.as_ref());
            mark_covered_rule(&mut covered_rules, working_dir_error.as_ref());

            let expected_relative = expected_result(input, relative_error);
            let expected_working_dir = expected_result(input, working_dir_error);
            let relative = RelativePath::parse(input).map(|path| path.as_str().to_owned());
            let working_dir = WorkingDir::parse(input);
            let working_dir_value = working_dir
                .as_ref()
                .map(|path| path.as_str().to_owned())
                .map_err(Clone::clone);

            assert_eq!(relative, expected_relative, "RelativePath input: {input:?}");
            relative_assertions += 1;
            assert_eq!(
                working_dir_value, expected_working_dir,
                "WorkingDir input: {input:?}"
            );
            working_dir_assertions += 1;

            if input == "." {
                assert!(working_dir.is_ok_and(|path| path.is_root()));
            }
        }

        assert_eq!(relative_assertions, 20);
        assert_eq!(working_dir_assertions, 20);
        assert!(covered_rules.into_iter().all(|covered| covered));
    }

    fn expected_result(input: &str, error: &Option<PathError>) -> Result<String, PathError> {
        error.clone().map_or_else(|| Ok(input.to_owned()), Err)
    }

    fn mark_covered_rule(covered: &mut [bool; 9], error: Option<&PathError>) {
        if let Some(error) = error {
            covered[rule_number(error) - 1] = true;
        }
    }

    fn rule_number(error: &PathError) -> usize {
        match error {
            PathError::Rule1Absolute => 1,
            PathError::Rule2ParentSegment => 2,
            PathError::Rule3Backslash => 3,
            PathError::Rule4ColonOrPrefix => 4,
            PathError::Rule5ControlCharacter => 5,
            PathError::Rule6EmptyOrDotSegment => 6,
            PathError::Rule7TrailingSpaceOrPeriod => 7,
            PathError::Rule8ReservedDeviceName => 8,
            PathError::Rule9LengthExceeded => 9,
        }
    }
}
