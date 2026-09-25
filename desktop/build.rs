#[path = "../build-support/frontend_fingerprint.rs"]
mod frontend_fingerprint;

use std::{env, fs, path::Path};

use frontend_fingerprint::{
    distribution_fingerprint, fingerprint, stamp_is_fresh, tracked_inputs, FrontendBuildStamp,
    STAMP_RELATIVE_PATH,
};

const DEBUG_REMEDIATION: &str = "cargo run -p arcadestr-xtask -- frontend-build";
const RELEASE_REMEDIATION: &str = "cargo run -p arcadestr-xtask -- frontend-build --release";
const WINDOWS_APP_MANIFEST: &str = include_str!("windows-app-manifest.xml");

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("Cargo provides CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(&manifest_dir)
        .parent()
        .expect("desktop is inside the workspace");
    let stamp_path = workspace_root.join(STAMP_RELATIVE_PATH);

    let inputs = tracked_inputs(workspace_root).expect("frontend inputs must be readable");
    for input in &inputs {
        println!("cargo:rerun-if-changed={}", input.display());
    }
    println!("cargo:rerun-if-changed={}", stamp_path.display());
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("web/dist").display()
    );

    let current = fingerprint(workspace_root).expect("frontend inputs must be fingerprintable");
    let current_distribution = distribution_fingerprint(workspace_root).ok();
    let stamp = fs::read_to_string(&stamp_path)
        .ok()
        .and_then(|value| FrontendBuildStamp::parse(&value).ok());
    let profile = env::var("PROFILE").unwrap_or_default();
    let fresh = stamp_is_fresh(
        stamp.as_ref(),
        &current,
        current_distribution.as_deref(),
        &profile,
    );
    if profile == "release" && !fresh {
        panic!(
            "STALE FRONTEND: web/dist does not match the current frontend sources. Run exactly: {RELEASE_REMEDIATION}"
        );
    }

    println!(
        "cargo:rustc-env=ARCADESTR_FRONTEND_BUILD_STATUS={}",
        if fresh { "fresh" } else { "stale" }
    );
    println!(
        "cargo:rustc-env=ARCADESTR_FRONTEND_BUILD_FINGERPRINT={}",
        stamp
            .as_ref()
            .map(|stamp| stamp.fingerprint.as_str())
            .unwrap_or("missing")
    );
    println!(
        "cargo:rustc-env=ARCADESTR_FRONTEND_BUILD_UNIX={}",
        stamp
            .as_ref()
            .map(|stamp| stamp.built_at_unix.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
    );
    println!("cargo:rustc-env=ARCADESTR_FRONTEND_BUILD_COMMAND={DEBUG_REMEDIATION}");

    assert!(
        WINDOWS_APP_MANIFEST.contains("<ws2:longPathAware>true</ws2:longPathAware>"),
        "Windows application manifest must keep long-path support enabled"
    );
    let windows = tauri_build::WindowsAttributes::new().app_manifest(WINDOWS_APP_MANIFEST);
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run Tauri build script");
}
