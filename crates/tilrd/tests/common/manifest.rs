//! Public test-vector key, trusted only by debug builds.
use std::path::Path;

pub fn write_manifest(root: &Path, platform: &str, version: &str, pointer: &str, checksum: &str) {
    let payload = serde_json::json!({
        "version": version, "platform": platform,
        "archive": format!("tilr-{version}-{platform}.tar.gz"), "sha256": checksum
    })
    .to_string();
    let key = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[0; 32]).unwrap();
    let signature: String = key
        .sign(payload.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let path = root.join("releases").join(platform);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join(format!("{pointer}.manifest.json")),
        serde_json::json!({"payload": payload, "signature": signature}).to_string(),
    )
    .unwrap();
}
