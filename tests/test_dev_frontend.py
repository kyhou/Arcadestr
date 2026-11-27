"""Exercise Trunk's frontend asset pipeline without Rust builds or application data.

Run: python3 -B -m unittest discover -s tests -p test_dev_frontend.py -v
"""

import json
from pathlib import Path
import re
import shutil
import socket
import subprocess
import tempfile
import time
import unittest
from urllib.request import urlopen


ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(shutil.which("trunk"), "Trunk is required for dev-server smoke tests")
class DevFrontendTests(unittest.TestCase):
    def test_release_css_hides_topbar_search_label(self):
        temporary_root = "/tmp/opencode" if Path("/tmp/opencode").is_dir() else None
        with tempfile.TemporaryDirectory(prefix="arcadestr-release-css-", dir=temporary_root) as directory:
            directory = Path(directory)
            web = directory / "web"
            for name in [
                "web/Trunk.toml",
                "web/style/tailwind.css",
                "web/tailwind.config.js",
                "web/assets/fonts/material-symbols-outlined.woff2",
                "app/src/ui_v2/components/topbar.rs",
            ]:
                destination = directory / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / name, destination)

            # Keep the real CSS and font pipelines, but omit the Rust build.
            html = (ROOT / "web/index.html").read_text()
            (web / "index.html").write_text(
                "\n".join(line for line in html.splitlines() if 'rel="rust"' not in line)
            )
            result = subprocess.run(
                ["trunk", "build", "--release", "--skip-version-check", "--no-color"],
                cwd=web, capture_output=True, text=True, timeout=90,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            styles = list((web / "dist").glob("tailwind-*.css"))
            self.assertEqual(len(styles), 1)
            css = styles[0].read_text()
            self.assertIsNotNone(
                re.search(r"\.sr-only\s*\{[^}]*position:\s*absolute", css),
                "Release CSS did not generate the sr-only utility used by the search label",
            )
            self.assertIn("@font-face", css)
            self.assertTrue((web / "dist/fonts/material-symbols-outlined.woff2").is_file())

    def test_boot_document_and_wasm_are_not_cached(self):
        temporary_root = "/tmp/opencode" if Path("/tmp/opencode").is_dir() else None
        with tempfile.TemporaryDirectory(prefix="arcadestr-dev-http-", dir=temporary_root) as directory:
            directory = Path(directory)
            (directory / "index.html").write_text(
                '<!doctype html><html><head><title>Startup fixture</title>'
                '<link data-trunk rel="copy-file" href="startup.wasm">'
                '</head><body>Startup fixture</body></html>'
            )
            wasm = b"\x00asm\x01\x00\x00\x00"
            (directory / "startup.wasm").write_bytes(wasm)
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                port = listener.getsockname()[1]
            with (directory / "trunk.log").open("w+") as log:
                process = subprocess.Popen(
                    ["trunk", "serve", "--config", str(ROOT / "web/Trunk.toml"),
                     "--port", str(port), "--address", "127.0.0.1",
                     "--dist", str(directory / "dist"), "--offline=true",
                     "--skip-version-check", "--no-autoreload=true", str(directory / "index.html")],
                    cwd=directory, stdout=log, stderr=subprocess.STDOUT,
                )
                try:
                    base = f"http://127.0.0.1:{port}"
                    deadline = time.monotonic() + 20
                    while True:
                        try:
                            with urlopen(base, timeout=1) as response:
                                self.assertIn(b"Startup fixture", response.read())
                            break
                        except OSError:
                            if process.poll() is not None or time.monotonic() > deadline:
                                log.seek(0)
                                self.fail(f"Fixture dev server did not start:\n{log.read()}")
                            time.sleep(0.05)
                    for path in ["/", "/startup.wasm"]:
                        with urlopen(base + path, timeout=3) as response:
                            self.assertIn(
                                "no-store", response.headers.get("Cache-Control", "").lower(),
                                "Cached HTML may reference deleted hashed WASM after a rebuild",
                            )
                            if path.endswith(".wasm"):
                                self.assertEqual(response.read(), wasm)
                                self.assertIn("application/wasm", response.headers.get("Content-Type", ""))
                finally:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()

    def test_entry_url_bypasses_previously_cached_document_without_changing_origin(self):
        config = json.loads((ROOT / "desktop/tauri.conf.json").read_text())
        self.assertEqual(config["build"]["devUrl"], "http://localhost:5173/?tauri-dev=1")


if __name__ == "__main__":
    unittest.main()
