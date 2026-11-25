//! Per-test scratch files that are removed when the test ends.
//! (Mirrors `arcadestr_core::test_helpers::scratch`, which is test-private.)
//!
//! Each test runs on its own thread (libtest spawns one per test, also with
//! `--test-threads=1`), and `#[tokio::test]` drives the test body on that
//! thread. The scratch directory is a thread-local [`tempfile::TempDir`], so
//! it is created on first use and deleted when the test's thread exits, even
//! if the test panics. Paths requested twice in one test (for example, to
//! reopen a database after a simulated restart) share the same directory.
//!
//! Request paths from the test thread itself, not from tasks spawned onto
//! other threads, whose directory would end with that thread.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

thread_local! {
    static SCRATCH: RefCell<Option<tempfile::TempDir>> = const { RefCell::new(None) };
}

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A new, not-yet-existing path in the current test's scratch directory.
pub fn scratch_path(prefix: &str, extension: &str) -> PathBuf {
    SCRATCH.with(|cell| {
        let mut scratch = cell.borrow_mut();
        let dir =
            scratch.get_or_insert_with(|| tempfile::tempdir().expect("test scratch directory"));
        dir.path().join(format!(
            "{prefix}-{}.{extension}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    })
}
