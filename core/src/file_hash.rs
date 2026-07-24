//! Streaming file hashing helpers for ADP build artifacts.

use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

const HASH_BUFFER_SIZE: usize = 1024 * 1024;

/// Computes a lowercase SHA-256 hex digest for a file.
///
/// The file is read in fixed-size chunks so large game archives are not buffered in memory.
///
/// # Errors
/// Returns any I/O error encountered while opening or reading `path`.
pub async fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    sha256_file_with_progress(path, |_, _| {}).await
}

/// Computes a lowercase SHA-256 hex digest while reporting bytes read and total bytes.
///
/// # Errors
/// Returns any I/O error encountered while opening, inspecting, or reading `path`.
pub async fn sha256_file_with_progress<F>(
    path: &Path,
    mut on_progress: F,
) -> Result<String, std::io::Error>
where
    F: FnMut(u64, u64),
{
    let mut file = tokio::fs::File::open(path).await?;
    let total_bytes = file.metadata().await?.len();
    let mut bytes_hashed = 0_u64;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; HASH_BUFFER_SIZE];

    on_progress(0, total_bytes);

    loop {
        let bytes_read = file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
        bytes_hashed = bytes_hashed.saturating_add(bytes_read as u64);
        on_progress(bytes_hashed, total_bytes);
    }

    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use sha2::{Digest, Sha256};

    #[tokio::test]
    async fn sha256_file_hashes_known_fixture() {
        let mut file = tempfile::NamedTempFile::new().expect("temp file should be created");
        file.write_all(b"hello arcadestr\n")
            .expect("fixture should write");

        let hash = sha256_file(file.path())
            .await
            .expect("hash should be computed");

        assert_eq!(
            hash,
            "359baa58f6775514f3cb5cc7fe69227f49beb60b3c88263c279f6d9efb64dfd9"
        );
    }

    #[tokio::test]
    async fn sha256_file_hashes_large_file_with_chunked_reader() {
        let mut file = tempfile::NamedTempFile::new().expect("temp file should be created");
        let bytes: Vec<u8> = (0..(1_048_576 + 17)).map(|i| (i % 251) as u8).collect();
        file.write_all(&bytes).expect("fixture should write");
        let expected = hex::encode(Sha256::digest(&bytes));

        let hash = sha256_file(file.path())
            .await
            .expect("hash should be computed");

        assert_eq!(hash, expected);
    }

    #[tokio::test]
    async fn sha256_file_reports_chunked_progress() {
        let mut file = tempfile::NamedTempFile::new().expect("temp file should be created");
        let bytes = vec![7_u8; HASH_BUFFER_SIZE + 17];
        file.write_all(&bytes).expect("fixture should write");
        let mut updates = Vec::new();

        sha256_file_with_progress(file.path(), |bytes_hashed, total_bytes| {
            updates.push((bytes_hashed, total_bytes));
        })
        .await
        .expect("hash should be computed");

        assert_eq!(updates.first(), Some(&(0, bytes.len() as u64)));
        assert_eq!(
            updates.last(),
            Some(&(bytes.len() as u64, bytes.len() as u64))
        );
        assert!(updates.len() >= 3, "large fixture should report each chunk");
    }

    #[tokio::test]
    async fn sha256_file_returns_io_error_for_missing_path() {
        let missing = std::env::temp_dir().join("arcadestr-missing-file-for-hash-test.bin");

        let result = sha256_file(&missing).await;

        assert!(result.is_err());
    }
}
