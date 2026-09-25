#[path = "../../build-support/frontend_fingerprint.rs"]
mod frontend_fingerprint;

use std::{
    env, fs,
    io::{self, Write},
    path::Path,
    process::{Command, ExitCode},
    time::{SystemTime, UNIX_EPOCH},
};

use frontend_fingerprint::{
    distribution_fingerprint, fingerprint, FrontendBuildStamp, STAMP_RELATIVE_PATH,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("frontend build failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);
    if arguments.next().as_deref() != Some("frontend-build") {
        return Err("usage: cargo run -p arcadestr-xtask -- frontend-build [--release]".into());
    }
    let release = match arguments.next().as_deref() {
        None => false,
        Some("--release") => true,
        Some(argument) => return Err(format!("unexpected argument: {argument}").into()),
    };
    if let Some(argument) = arguments.next() {
        return Err(format!("unexpected argument: {argument}").into());
    }

    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("xtask must be inside the workspace")?;
    let mut command = Command::new("trunk");
    command.arg("build").current_dir(workspace_root.join("web"));
    if release {
        command.arg("--release");
    }
    let status = command.status()?;
    if !status.success() {
        return Err(format!("trunk exited with {status}").into());
    }

    let stamp = FrontendBuildStamp {
        fingerprint: fingerprint(workspace_root)?,
        distribution_fingerprint: distribution_fingerprint(workspace_root)?,
        built_at_unix: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        profile: if release { "release" } else { "debug" }.to_owned(),
    };
    write_atomically(&workspace_root.join(STAMP_RELATIVE_PATH), &stamp.encode())?;
    println!(
        "frontend {} build stamped {}",
        stamp.profile, stamp.fingerprint
    );
    Ok(())
}

fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("stamp has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    fs::rename(temporary, path)
}
