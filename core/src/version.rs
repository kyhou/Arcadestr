// Application version and revision tracking
// Increment REVISION every time you make changes to verify builds

/// Application version (follows semantic versioning)
pub const VERSION: &str = "0.1.0";

/// Build revision - increment this every time you make changes
/// This helps verify that the latest code is actually running
pub const REVISION: u32 = 10;

/// Get full version string including revision
pub fn full_version() -> String {
    format!("{} (revision {})", VERSION, REVISION)
}

/// Get just the revision number as string
pub fn revision_string() -> String {
    REVISION.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_format() {
        let full = full_version();
        assert!(full.contains(VERSION));
        assert!(full.contains(&REVISION.to_string()));
    }
}
