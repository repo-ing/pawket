"""Start an extracted Pawket package from an unrelated working directory."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener
from zipfile import ZipFile


def wait_for_address(process, log):
    deadline = time.monotonic() + 15
    while process.poll() is None and time.monotonic() < deadline:
        text = log.read_text(encoding="utf-8")
        match = re.search(r"Pawket is ready at (http://127\.0\.0\.1:\d+)", text)
        if match:
            return match[1]
        time.sleep(0.1)
    raise RuntimeError(f"Pawket did not start: {log.read_text(encoding='utf-8')}")


def check_app(address):
    # Loopback is the only destination in this test.
    opener = build_opener(ProxyHandler({}))

    def fetch(path):
        with opener.open(address + path, timeout=5) as response:
            return response.headers.get_content_type(), response.read()

    _, page = fetch("/")
    token = re.search(rb'value="([a-f0-9]{64})"', page)
    assert b"<title>Pawket" in page and token, "Expected the Pawket page and pairing code"
    assert json.loads(fetch("/health")[1])["status"] == "ok"
    assert fetch("/style.css")[0] == "text/css"
    assert fetch("/bootstrap.js")[0] == "text/javascript"
    assert fetch("/pkg/pawket_browser.js")[0] == "text/javascript"
    wasm_type, wasm = fetch("/pkg/pawket_browser_bg.wasm")
    assert wasm_type == "application/wasm" and wasm.startswith(b"\0asm")

    request = Request(
        address + "/v1/inspect",
        data=b'{"url":"https://127.0.0.1/example.mp4"}',
        headers={
            "Content-Type": "application/json",
            "X-Pawket-Token": token[1].decode(),
        },
    )
    try:
        opener.open(request, timeout=5).close()
    except HTTPError as error:
        with error:
            assert error.code == 422
            assert json.load(error)["error"]["code"] == "blocked_address"
    else:
        raise AssertionError("Expected the paired API to reject a loopback download")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        with ZipFile(args.archive) as archive:
            for entry in archive.infolist():
                path = Path(archive.extract(entry, root))
                if os.name != "nt":
                    path.chmod(entry.external_attr >> 16 & 0o777)
        package = root / "Pawket"
        assert (package / "extension/manifest.json").is_file()
        assert (package / "START.md").is_file()
        binary = package / ("pawket.exe" if os.name == "nt" else "pawket")
        assert os.access(binary, os.X_OK)
        cwd = root / "separate working directory"
        cwd.mkdir()
        log = root / "pawket.log"
        with log.open("w", encoding="utf-8") as stream:
            process = subprocess.Popen(
                [str(binary), "serve", "--port", "0", "--output", str(root / "data")],
                cwd=cwd,
                stdin=subprocess.DEVNULL,
                stdout=stream,
                stderr=subprocess.STDOUT,
            )
            try:
                check_app(wait_for_address(process, log))
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
    print(f"Verified {args.archive.name}")


if __name__ == "__main__":
    main()
