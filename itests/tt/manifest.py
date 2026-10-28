"""Signed local-CDN metadata using the debug build's public test vector."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def write_manifest(root, platform, version):
    root = Path(root)
    name = f"tilr-{version}-{platform}.tar.gz"
    archive = root / "releases" / version / name
    digest = hashlib.sha256(archive.read_bytes()).hexdigest() if archive.exists() else "0" * 64
    payload = json.dumps(dict(version=version, platform=platform, archive=name, sha256=digest),
                         separators=(",", ":"))
    with tempfile.TemporaryDirectory() as tmp:
        key, message = Path(tmp) / "key.der", Path(tmp) / "payload"
        # PKCS#8 wrapping of the public test vector's zero seed; never a production key.
        key.write_bytes(bytes.fromhex("302e020100300506032b657004220420") + bytes(32))
        key.chmod(0o600)
        message.write_bytes(payload.encode())
        signature = subprocess.check_output(["openssl", "pkeyutl", "-sign", "-rawin", "-keyform", "DER",
                                             "-inkey", str(key), "-in", str(message)])
    target = root / "releases" / platform / "latest.manifest.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(dict(payload=payload, signature=signature.hex())))
