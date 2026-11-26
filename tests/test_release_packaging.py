"""Release asset contracts; no builds, downloads, or publication."""

import hashlib
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class ReleaseAssets(unittest.TestCase):
    def test_license_declarations_and_tauri_resources_agree(self):
        import tomllib

        manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
        self.assertEqual(manifest["workspace"]["package"]["license"], "MIT OR Apache-2.0")
        config = json.loads((ROOT / "desktop/tauri.conf.json").read_text())
        resources = config["bundle"]["resources"]
        for name in ["LICENSE-MIT", "LICENSE-APACHE"]:
            self.assertEqual(resources[f"../{name}"], f"licenses/{name}")
            self.assertTrue((ROOT / name).is_file())

    def test_arch_package_function_installs_both_license_texts(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            source = work / "arcadestr-1.2.3"
            source.mkdir()
            for name in [
                "LICENSE-MIT", "LICENSE-APACHE",
                "desktop/icons/icon.png", "packaging/arch/arcadestr.desktop",
            ]:
                destination = source / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / name, destination)
            binary = source / "target/release/arcadestr-desktop"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"binary fixture")
            recipe = (ROOT / "packaging/arch/PKGBUILD.in").read_text()
            (work / "PKGBUILD").write_text(
                recipe.replace("@VERSION@", "1.2.3").replace("@SHA256@", "0" * 64)
            )
            package = work / "pkg"
            subprocess.run(
                ["bash", "-ec", 'source ./PKGBUILD; pkgdir="$1"; package',
                 "package-test", str(package)],
                cwd=work, check=True, capture_output=True,
            )
            for name in ["LICENSE-MIT", "LICENSE-APACHE"]:
                installed = package / "usr/share/licenses/arcadestr" / name
                self.assertEqual(installed.read_bytes(), (ROOT / name).read_bytes())

    def test_linux_collection_requires_deb_and_rpm_not_appimage(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            for kind, name in [
                ("deb", "Arcadestr_1.2.3_amd64.deb"),
                ("rpm", "Arcadestr-1.2.3-1.x86_64.rpm"),
            ]:
                bundle = work / "target/release/bundle" / kind
                bundle.mkdir(parents=True)
                (bundle / name).write_bytes(b"fixture")
            subprocess.run(
                ["bash", str(ROOT / "scripts/release/collect-bundles.sh"),
                 "linux-x86_64", "1.2.3", "dist"],
                cwd=work, check=True, capture_output=True,
            )
            self.assertEqual(
                sorted(path.suffix for path in (work / "dist").iterdir()),
                [".deb", ".rpm"],
            )

    def test_assembly_includes_arch_package_and_rejects_old_appimage(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            assets = work / "assets"
            assets.mkdir()
            names = [
                "arcadestr-1.2.3-1-x86_64.pkg.tar.zst",
                "Arcadestr-1.2.3-linux-x86_64.deb",
                "Arcadestr-1.2.3-linux-x86_64.rpm",
                "Arcadestr-1.2.3-macos-arm64.dmg",
                "Arcadestr-1.2.3-macos-x86_64.dmg",
                "Arcadestr-1.2.3-windows-x86_64-setup.exe",
                "Arcadestr-1.2.3-windows-x86_64.msi",
            ]
            for name in names:
                (assets / name).write_bytes(name.encode())
            command = [
                "bash", str(ROOT / "scripts/release/assemble.sh"),
                "1.2.3", "v1.2.3", "HEAD", str(assets), str(work / "notes.md"),
            ]
            subprocess.run(command, cwd=ROOT, check=True, capture_output=True)
            checksums = (assets / "SHA256SUMS").read_text().splitlines()
            self.assertEqual(len(checksums), 7)
            for line in checksums:
                digest, name = line.split()
                self.assertEqual(digest, hashlib.sha256((assets / name).read_bytes()).hexdigest())
            (assets / "SHA256SUMS").unlink()
            (assets / names[0]).unlink()
            (assets / "Arcadestr-1.2.3-linux-x86_64.AppImage").write_bytes(b"old")
            result = subprocess.run(command, cwd=ROOT, capture_output=True)
            self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
