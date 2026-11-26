# Releases

Arcadestr desktop releases are GitHub pre-releases with installers for Linux,
Windows and macOS, built by `.github/workflows/release.yml` from a `vX.Y.Z`
tag. Building never needs the adp-server repository. The release version is
the workspace version in `Cargo.toml` (mirrored in `desktop/tauri.conf.json`).
It is independent of the ADP protocol version (`0.3.0`).

## Installing a release

### Downloads

The next release replaces AppImage with the Arch package below. The existing
`v0.1.0` release is unchanged and does not contain an Arch package.

| Platform | File | Notes |
|---|---|---|
| Arch Linux x86-64 | `arcadestr-X.Y.Z-1-x86_64.pkg.tar.zst` | Native Arch package built with `makepkg`; uses system WebKitGTK and graphics libraries |
| Linux x86-64 | `Arcadestr-X.Y.Z-linux-x86_64.deb` | Debian/Ubuntu (`libc6 >= 2.39`) |
| Linux x86-64 | `Arcadestr-X.Y.Z-linux-x86_64.rpm` | Fedora/openSUSE with glibc 2.39+ |
| Windows x86-64 | `Arcadestr-X.Y.Z-windows-x86_64-setup.exe` | Installer (NSIS) |
| Windows x86-64 | `Arcadestr-X.Y.Z-windows-x86_64.msi` | Windows Installer package |
| macOS Intel | `Arcadestr-X.Y.Z-macos-x86_64.dmg` | |
| macOS Apple Silicon | `Arcadestr-X.Y.Z-macos-arm64.dmg` | |

The Linux packages need WebKitGTK 4.1, declared as a package dependency.
Windows needs the WebView2 runtime,
which ships with Windows 10 and 11; the installers download it if it is
missing. Linux ARM64 and Windows ARM64 builds are not produced.

### Verify the download

Download the installer and `SHA256SUMS` from the same release.

```bash
# Linux
sha256sum --check --ignore-missing SHA256SUMS
# macOS
shasum -a 256 --check --ignore-missing SHA256SUMS
```

```powershell
# Windows: compare with the line for this file in SHA256SUMS
(Get-FileHash .\Arcadestr-X.Y.Z-windows-x86_64-setup.exe -Algorithm SHA256).Hash.ToLower()
```

A mismatch means the download is corrupt or not the published file. Do not
install it.

### Unsigned installers

Releases are **not code-signed** and are not notarized by Apple.
`SHA256SUMS` lets you check that a file is the one GitHub published; it does
not prove who built it.

- **Windows:** SmartScreen may show "Windows protected your PC". Choose
  **More info**, then **Run anyway**. Some antivirus products quarantine
  unsigned installers.
- **macOS:** the app is ad-hoc signed only, with no developer identity.
  Gatekeeper blocks the first launch. Open it with Control-click → **Open**,
  or use **System Settings → Privacy & Security → Open Anyway**. If macOS
  reports the app as damaged, remove the download quarantine after
  copying it to Applications, then open it again:

  ```bash
  xattr -dr com.apple.quarantine /Applications/Arcadestr.app
  ```

- **Linux:** the Arch, `.deb` and `.rpm` packages are not GPG-signed.

### Arch Linux and Manjaro

On an up-to-date Arch Linux system, install the downloaded package with:

```bash
sudo pacman -U ./arcadestr-X.Y.Z-1-x86_64.pkg.tar.zst
arcadestr
```

Manjaro may lag behind Arch's library versions. Do not force dependency
installation or perform a partial upgrade if the package requires newer
libraries. This native package does not bundle WebKitGTK, Wayland or EGL.
Rendering on Manjaro Wayland still requires a runtime check on that system.
The validation build linked OpenSSL 4 (`libssl.so.4`/`libcrypto.so.4`).
Manjaro installations with OpenSSL 3 cannot run that binary; rebuild on the
target distribution or wait for matching libraries rather than forcing it.
The recipe's shared-library dependencies let `makepkg` record the actual
OpenSSL SONAME requirements in the package.

AppImage is no longer built: the published `v0.1.0` AppImage fails to render
on a reported Manjaro Wayland setup while the native release binary works.
Existing release assets and checksums are unchanged.

There are no automatic updates. Install a newer release over the old one;
your accounts and library data are kept.

## Publishing a release

1. Set the version in `Cargo.toml` (`[workspace.package] version`) and in
   `desktop/tauri.conf.json` to `X.Y.Z`. Run `cargo build --locked` so
   `Cargo.lock` agrees, and commit to `main`. Versions must be plain
   `X.Y.Z`; MSI packages cannot carry pre-release suffixes.
2. Push `main`, then tag that commit and push the tag:

   ```bash
   git tag -a vX.Y.Z -m "Arcadestr vX.Y.Z"
   git push origin vX.Y.Z
   ```

The tag push starts the release workflow. It refuses tags that are not
`vX.Y.Z`, that point outside `main`, or whose version differs from either
manifest. Tags are never moved or reused; fix a bad release with a new
version.

### What the workflow does

1. **resolve** finds the tag's exact commit and checks the versions.
2. **build** runs one native job per platform: `ubuntu-24.04`,
   `windows-2025`, `macos-15-intel` and `macos-15` (Apple Silicon). Each job:
   - installs Rust 1.98.0 with the WASM target, Trunk 0.21.14 and Tauri CLI
     2.10.1;
   - builds with `tauri-apps/tauri-action`, whose `beforeBuildCommand` runs
     `cargo run --locked -p arcadestr-xtask -- frontend-build --release`;
   - lets `desktop/build.rs` reject a stale frontend, as in any release build;
   - builds the desktop binary with `--locked`;
   - checks the binary's architecture, the release frontend stamp, and that
     the debug-only MCP bridge capability is absent;
   - renames the bundles to the names above.
   A separate Arch job runs `scripts/release/build-arch.sh` in a pinned
   `archlinux:base-devel` container. It exports only the selected Git commit,
   generates `PKGBUILD` from `packaging/arch/PKGBUILD.in` with its source
   SHA-256 (never `SKIP`), updates the container with `pacman -Syu`, and runs
   `makepkg` as an unprivileged user. `prepare()` fetches locked Cargo
   dependencies; `build()` uses the normal release frontend stamp and frozen
   Cargo builds. The package installs only `/usr/bin/arcadestr`, a desktop
   entry, icon and both license texts; system WebKitGTK and graphics libraries
   are not bundled.
   `namcap`, `desktop-file-validate`, architecture and stamp checks run before
   uploading. `pkgrel` starts at 1; packaging-only rebuilds of a version must
   increment it in the recipe and expected asset list.

   The recipe follows the [Arch package guidelines](https://wiki.archlinux.org/title/Arch_package_guidelines)
   and [Rust guidelines](https://wiki.archlinux.org/title/Rust_package_guidelines).
   Its SPDX license field matches Cargo's `MIT OR Apache-2.0` declaration.
   `LICENSE-MIT` and `LICENSE-APACHE` are installed under
   `/usr/share/licenses/arcadestr/`; the Tauri installers carry the same texts
   as bundled `licenses/` resources. Third-party licenses are unchanged.
   The Arch job refuses to upload on any `namcap` packaging error.
   This is not an AUR submission.

   To build the same package locally after committing the source and recipe:

   ```bash
   bash scripts/release/build-arch.sh HEAD X.Y.Z dist
   ```

   Docker is required. The script archives the specified commit, not the
   working tree, so uncommitted changes are never shipped. Arch's repository
   toolchain (`cargo`, `rust-wasm`, `trunk`) is used inside this job; it is
   independent of the fixed toolchain used by the other release jobs.
3. **assemble** requires every platform, checks the exact set of seven
   installers, writes `SHA256SUMS`, and generates notes from the commits
   since the previous tag.
4. **publish** is the only job with write permission. It creates one draft
   release with every asset, then publishes it as a pre-release.

A failed or cancelled platform build stops the run before anything is
published, so a partial release never appears.

### Retrying

Re-run the failed jobs from the Actions page, or run the workflow manually
with the same tag. Publication is idempotent:

- no release yet: it is created;
- a draft left by an interrupted run: its assets and notes are replaced and it
  is published;
- already published: it is left unchanged (rebuilt installers are not
  bit-identical, so published assets are never replaced).

### Dry run

Run the workflow manually with **dry run** enabled and any branch, tag or
commit. It builds and assembles every installer and keeps the result as a
workflow artifact named `release`, but never publishes. Use a dry run of
`main` before tagging when release tooling or platform dependencies change.

## Debug-only functionality

The MCP bridge and its capability are compiled only into debug builds. Every
release build checks that the capability is absent, and
`mcp_bridge_exposure_tests` guard the source.
