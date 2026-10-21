use std::process::Command;

#[test]
fn shared_ghostty_build_reuses_complete_toolchain_prefixes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("python3")
        .arg(root.join("scripts/test-ghostty-vt.py"))
        .output()
        .expect("run shared Ghostty build fixture");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
