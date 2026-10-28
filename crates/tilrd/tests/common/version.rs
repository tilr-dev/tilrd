use std::process::Command;

/// Version and platform of the executable under test, as `--version` prints
/// them: `tilrd 0.1.0-26449dd (linux-aarch64)`.
pub fn version_and_platform() -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_tilrd"))
        .env_remove("RUST_LOG")
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let line = String::from_utf8(output.stdout).unwrap();
    let mut words = line.split_whitespace().skip(1);
    let version = words.next().unwrap().to_owned();
    let platform = words.next().unwrap().trim_matches(['(', ')']).to_owned();
    (version, platform)
}
