//! Keep the documentation's scenario-to-test map executable in Cargo and CI.

use std::path::Path;
use std::process::Command;

fn run_script(script: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("python3")
        .arg(root.join("scripts").join(script))
        .output()
        .expect("run use-case reference checker");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn every_use_case_test_reference_exists() {
    run_script("check_use_cases.py");
}

#[test]
fn missing_use_case_test_references_are_rejected() {
    run_script("test_check_use_cases.py");
}
