#!/usr/bin/env python3
"""Prepare signed channel metadata before any CDN uploads."""
import hashlib
import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    version, pointer, *platforms = sys.argv[1:]
    openssl = os.environ.get("OPENSSL") or (shutil.which("openssl", path="/opt/homebrew/bin:/usr/local/bin") if sys.platform == "darwin" else None) or "openssl"
    key = os.environ.get("TILR_UPDATE_SIGNING_KEY")
    if not key:
        sys.exit("TILR_UPDATE_SIGNING_KEY is not set: provide an Ed25519 PEM private key")
    with tempfile.TemporaryDirectory(prefix="tilr-sign-") as temp:
        key_path = Path(temp) / "key.pem"
        key_path.write_text(key)
        key_path.chmod(0o600)
        public = subprocess.check_output([openssl, "pkey", "-in", str(key_path), "-pubout", "-outform", "DER"])
        expected = os.environ.get("TILR_UPDATE_PUBLIC_KEY", "").lower()
        if len(public) != 44 or public[:12].hex() != "302a300506032b6570032100" or public[12:].hex() != expected:
            sys.exit("signing key does not match TILR_UPDATE_PUBLIC_KEY (hex Ed25519 public key)")
        for platform in platforms:
            name = f"tilr-{version}-{platform}.tar.gz"
            archive = Path("dist/cdn/releases") / version / name
            payload = json.dumps(dict(version=version, platform=platform, archive=name,
                                      sha256=hashlib.sha256(archive.read_bytes()).hexdigest()),
                                 separators=(",", ":"))
            message = Path(temp) / "payload"
            message.write_bytes(payload.encode())
            signature = subprocess.check_output([
                openssl, "pkeyutl", "-sign", "-rawin", "-inkey", str(key_path),
                "-in", str(message)])
            if len(signature) != 64:
                sys.exit("signing key is not Ed25519")
            target = Path("dist/cdn/releases") / platform / f"{pointer}.manifest.json"
            target.write_text(json.dumps(dict(payload=payload, signature=signature.hex())) + "\n")


if __name__ == "__main__":
    main()
