//! Use cases: release.sh builds a CDN layout consumed by tilrd update, and CI
//! keeps the release gates and packaging contract.

use std::fs;
#[path = "common/cdn.rs"]
mod cdn;
use cdn::serve_files;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/version.rs"]
mod version;
use version::version_and_platform;

#[test]
fn manifest_signing_supports_keg_only_openssl_and_reports_unsupported_tools() {
    let output = Command::new("python3")
        .arg(root().join("scripts/test_sign_update_manifest.py"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn stable_releases_propose_desktop_pins_with_real_git() {
    let output = Command::new("python3")
        .arg(root().join("scripts/test_bump_desktop_pin.py"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn release_upstream_checks_real_git_histories() {
    let output = Command::new("python3")
        .arg(root().join("scripts/test_release_upstream.py"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn release_upstream_workflow_is_trusted() {
    let workflow = text_file(".github/workflows/release-upstream.yml");
    for required in [
        "pull_request_target:",
        "branches: ['release/**']",
        "name: release-upstream",
        "ref: main",
        "persist-credentials: false",
        "contents: read",
        "git init --bare",
        "+refs/heads/main:refs/inspection/main",
        "+$BASE_SHA:refs/inspection/base",
        "+$HEAD_SHA:refs/inspection/head",
        "python3 scripts/release_upstream.py",
        "--event \"$GITHUB_EVENT_PATH\"",
    ] {
        assert!(workflow.contains(required), "missing {required}");
    }
    assert!(!workflow.contains("secrets."));
    assert!(!workflow.contains("ref: ${{ github.event.pull_request.head"));
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn text_file(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap()
}

struct Fixture {
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.dir) {
            eprintln!("could not remove fixture {}: {error}", self.dir.display());
        }
    }
}

impl Fixture {
    fn new() -> Self {
        let dir = scratch::scratch_dir();
        fs::create_dir(dir.join("scripts")).unwrap();
        fs::create_dir(dir.join("bin")).unwrap();
        for script in [
            "release.sh",
            "sign-update-manifest.py",
            "install.sh",
            "macos-build.sh",
            "cdn-layout.sh",
        ] {
            fs::copy(
                root().join("scripts").join(script),
                dir.join("scripts").join(script),
            )
            .unwrap();
        }
        fs::write(
            dir.join("Cargo.toml"),
            "[workspace.package]\nversion = \"9.8.7\"\n",
        )
        .unwrap();
        for name in ["LICENSE-APACHE", "LICENSE-MIT", "README.md"] {
            fs::write(dir.join(name), format!("fixture {name}\n")).unwrap();
        }
        executable(
            &dir.join("scripts/linux-build.sh"),
            r#"#!/bin/sh
set -eu
case "$TILR_LINUX_PLATFORM" in
  linux/arm64) platform=linux-aarch64 ;;
  linux/amd64) platform=linux-x86_64 ;;
  *) exit 9 ;;
esac
printf '%s\n' "$platform" >> build.log
mkdir -p "dist/$platform"
for exe in tilrd tilr; do
  printf '#!/bin/sh\nprintf "%%s\\n" "%s %s-%s (%s)"\n' "$exe" "$TILR_BUILD_VERSION" "$TILR_BUILD_COMMIT" "$platform" > "dist/$platform/$exe"
  chmod +x "dist/$platform/$exe"
done
"#,
        );
        executable(
            &dir.join("bin/aws"),
            r#"#!/bin/sh
[ "$AWS_ACCESS_KEY_ID" = fixture-key ] && [ "$AWS_SECRET_ACCESS_KEY" = fixture-secret ] || exit 9
printf '%s|' "$@" >> "$AWS_LOG"
printf '\n' >> "$AWS_LOG"
if [ -f "$AWS_FAIL" ] && [ "$4" = "s3://tilr-cdn/$(cat "$AWS_FAIL")" ]; then
  exit 1
fi
"#,
        );
        executable(&dir.join("bin/uname"), "#!/bin/sh\necho Darwin\n");
        executable(
            &dir.join("bin/cargo"),
            r#"#!/bin/sh
set -eu
[ "$*" = "build --release --locked -p tilrd -p tilr --target aarch64-apple-darwin" ] ||
[ "$*" = "build --release --locked -p tilrd -p tilr --target x86_64-apple-darwin" ] || exit 9
for target; do :; done
case "$target" in
 aarch64-apple-darwin) platform=macos-aarch64 ;;
 x86_64-apple-darwin) platform=macos-x86_64 ;;
esac
printf '%s\n' "$platform" >> build.log
mkdir -p "target/$target/release"
for exe in tilrd tilr; do
 printf '#!/bin/sh\nprintf "%%s\\n" "%s %s-%s (%s)"\n' "$exe" "$TILR_BUILD_VERSION" "$TILR_BUILD_COMMIT" "$platform" > "target/$target/release/$exe"
 chmod +x "target/$target/release/$exe"
done
"#,
        );
        executable(
            &dir.join("bin/codesign"),
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> signing.log
[ "${SIGN_FAIL:-}" != 1 ] || exit 1
for binary; do :; done
if [ "$1" = --verify ]; then
 [ "${VERIFY_FAIL:-}" != 1 ] || exit 1
 tail -1 "$binary" | grep -q '^# Developer ID signed$'
else
 [ "$*" = "--force --sign Developer ID Application: Fixture --options runtime --timestamp $binary" ] || exit 9
 printf '# Developer ID signed\n' >> "$binary"
fi
"#,
        );
        executable(
            &dir.join("bin/xcrun"),
            r#"#!/bin/sh
set -eu
[ "$1 $2" = 'notarytool submit' ] || exit 9
[ "$4 $5 $6 $7 $8" = '--keychain-profile fixture --wait --output-format json' ] || exit 9
python3 - "$3" <<'CHECK'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    assert sorted(z.namelist()) == ['tilr', 'tilrd']
    assert all(z.read(n).endswith(b'# Developer ID signed\n') for n in z.namelist())
CHECK
[ "${NOTARY_FAIL:-}" != 1 ] || exit 1
printf '{"status":"%s"}\n' "${NOTARY_STATUS:-Accepted}"
"#,
        );
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "release@test.invalid"]);
        git(&dir, &["config", "user.name", "Release Test"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "fixture"]);
        Self { dir }
    }

    fn run(&self, args: &[&str], account: bool) -> Output {
        self.run_env(args, account, &[])
    }

    fn run_env(&self, args: &[&str], account: bool, env: &[(&str, &str)]) -> Output {
        let mut command = Command::new("sh");
        command
            .current_dir(&self.dir)
            .arg("scripts/release.sh")
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        command.env(
            "PATH",
            format!(
                "{}:{}",
                self.dir.join("bin").display(),
                std::env::var("PATH").unwrap()
            ),
        );
        let seed_der = [
            vec![
                0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
                0x04, 0x20,
            ],
            vec![0; 32],
        ]
        .concat();
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(seed_der);
        command.env(
            "TILR_UPDATE_SIGNING_KEY",
            format!("-----BEGIN PRIVATE KEY-----\n{encoded}\n-----END PRIVATE KEY-----\n"),
        );
        command.env(
            "TILR_UPDATE_PUBLIC_KEY",
            "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29",
        );
        command.env("TILR_SIGN_IDENTITY", "Developer ID Application: Fixture");
        command.env("TILR_NOTARY_PROFILE", "fixture");
        command.env_remove("TILR_SIGN_KEYCHAIN");
        command.env_remove("TILR_BUILD_VERSION");
        for name in ["SIGN_FAIL", "VERIFY_FAIL", "NOTARY_FAIL", "NOTARY_STATUS"] {
            command.env_remove(name);
        }
        command.envs(env.iter().copied());
        command.env("AWS_LOG", self.dir.join("aws.log"));
        command.env("AWS_FAIL", self.dir.join("aws.fail"));
        command.env("AWS_ACCESS_KEY_ID", "fixture-key");
        command.env("AWS_SECRET_ACCESS_KEY", "fixture-secret");
        command.env_remove("AWS_SESSION_TOKEN");
        command.env_remove("CLOUDFLARE_ACCOUNT_ID");
        if account {
            command.env("CLOUDFLARE_ACCOUNT_ID", "fixture-account");
        }
        command.output().unwrap()
    }

    fn version(&self) -> String {
        self.version_of("9.8.7")
    }

    fn version_of(&self, package: &str) -> String {
        let output = Command::new("git")
            .current_dir(&self.dir)
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .unwrap();
        format!(
            "{package}-{}",
            String::from_utf8(output.stdout).unwrap().trim()
        )
    }

    fn cdn(&self) -> PathBuf {
        self.dir.join("dist/cdn/releases")
    }

    fn log(&self) -> String {
        fs::read_to_string(self.dir.join("aws.log")).unwrap_or_default()
    }

    /// The stub aws fails to upload `name` (an object of tilr-cdn).
    fn fail_upload(&self, name: &str) {
        fs::write(self.dir.join("aws.fail"), name).unwrap();
    }

    /// The objects uploaded, in order.
    fn uploads(&self) -> Vec<String> {
        self.log()
            .lines()
            .map(|line| {
                line.split('|')
                    .nth(3)
                    .unwrap()
                    .strip_prefix("s3://tilr-cdn/")
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }
}

fn executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .current_dir(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn assert_exit(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
}

fn archive(fixture: &Fixture, platform: &str) -> PathBuf {
    let version = fixture.version();
    fixture
        .cdn()
        .join(&version)
        .join(format!("tilr-{version}-{platform}.tar.gz"))
}

#[test]
fn a_dirty_tree_builds_no_release() {
    let f = Fixture::new();
    fs::write(f.dir.join("README.md"), "changed\n").unwrap();
    let output = f.run(&[], false);
    assert_exit(&output, 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("uncommitted changes"));
    assert!(!f.dir.join("dist/cdn").exists());
    assert!(!f.dir.join("build.log").exists());
}

#[test]
fn a_release_is_named_after_the_package_version_and_the_commit() {
    let f = Fixture::new();
    assert_exit(&f.run(&[], false), 0);
    let version = f.version();
    let archive = archive(&f, "linux-aarch64");
    assert!(archive.is_file());
    assert!(archive.with_extension("gz.sha256").is_file());
    assert_eq!(
        fs::read_to_string(f.cdn().join("linux-aarch64/latest")).unwrap(),
        format!("{version}\n")
    );
    assert_eq!(
        fs::read_to_string(f.dir.join("build.log")).unwrap(),
        "linux-aarch64\n"
    );
}

#[test]
fn a_release_is_named_after_the_build_version_when_given() {
    let f = Fixture::new();
    let output = f.run_env(
        &["linux-x86_64", "macos-aarch64"],
        false,
        &[("TILR_BUILD_VERSION", "9.9.0-rc.2")],
    );
    assert_exit(&output, 0);
    let version = f.version_of("9.9.0-rc.2");
    for platform in ["linux-x86_64", "macos-aarch64"] {
        assert_eq!(
            fs::read_to_string(f.cdn().join(platform).join("latest")).unwrap(),
            format!("{version}\n")
        );
        assert!(
            f.cdn()
                .join(&version)
                .join(format!("tilr-{version}-{platform}.tar.gz"))
                .is_file()
        );
        let tilrd = f.dir.join("dist").join(platform).join("tilrd");
        let said = Command::new(&tilrd).output().unwrap().stdout;
        assert_eq!(
            String::from_utf8(said).unwrap(),
            format!("tilrd {version} ({platform})\n")
        );
    }
}

#[test]
fn each_platform_given_is_built_with_its_own_latest() {
    let f = Fixture::new();
    assert_exit(&f.run(&["linux-x86_64", "linux-aarch64"], false), 0);
    for platform in ["linux-x86_64", "linux-aarch64"] {
        assert!(archive(&f, platform).is_file());
        assert_eq!(
            fs::read_to_string(f.cdn().join(platform).join("latest")).unwrap(),
            format!("{}\n", f.version())
        );
    }
    assert_eq!(
        fs::read_to_string(f.dir.join("build.log")).unwrap(),
        "linux-x86_64\nlinux-aarch64\n"
    );
}

#[test]
fn both_macos_architectures_publish_the_signed_bytes_and_checked_layout() {
    let f = Fixture::new();
    assert_exit(
        &f.run(&["--publish", "macos-aarch64", "macos-x86_64"], true),
        0,
    );
    let version = f.version();
    let mut expected = Vec::new();
    for platform in ["macos-aarch64", "macos-x86_64"] {
        let file = archive(&f, platform);
        let checksum = Command::new("shasum")
            .current_dir(file.parent().unwrap())
            .args(["-a", "256", "-c"])
            .arg(file.file_name().unwrap().to_str().unwrap().to_owned() + ".sha256")
            .output()
            .unwrap();
        assert!(checksum.status.success(), "{checksum:?}");
        for exe in ["tilrd", "tilr"] {
            let unpacked = Command::new("tar")
                .args(["-xOf"])
                .arg(&file)
                .arg(exe)
                .output()
                .unwrap();
            assert!(unpacked.status.success(), "{unpacked:?}");
            let signed = fs::read(f.dir.join("dist").join(platform).join(exe)).unwrap();
            assert_eq!(unpacked.stdout, signed);
            assert!(signed.ends_with(b"# Developer ID signed\n"));
        }
        assert_eq!(
            fs::read_to_string(f.cdn().join(platform).join("latest")).unwrap(),
            format!("{version}\n")
        );
        let key = format!("releases/{version}/tilr-{version}-{platform}.tar.gz");
        expected.extend([
            key.clone(),
            format!("{key}.sha256"),
            format!("releases/{platform}/latest.manifest.json"),
            format!("releases/{platform}/latest"),
        ]);
    }
    expected.push("install.sh".into());
    assert_eq!(f.uploads(), expected);
    fs::remove_file(f.dir.join("aws.log")).unwrap();
    assert_exit(
        &f.run_env(
            &["--upload-only", "macos-aarch64", "macos-x86_64"],
            true,
            &[("TILR_SIGN_IDENTITY", ""), ("TILR_NOTARY_PROFILE", "")],
        ),
        0,
    );
    assert_eq!(f.uploads(), expected);
}

#[test]
fn macos_signing_and_notarization_failures_publish_nothing() {
    for env in [
        ("SIGN_FAIL", "1"),
        ("VERIFY_FAIL", "1"),
        ("NOTARY_FAIL", "1"),
        ("NOTARY_STATUS", "Invalid"),
        ("NOTARY_STATUS", "\""),
        ("TILR_SIGN_IDENTITY", "-"),
        ("TILR_NOTARY_PROFILE", ""),
    ] {
        let f = Fixture::new();
        let output = f.run_env(&["--publish", "macos-aarch64"], true, &[env]);
        assert!(!output.status.success(), "{env:?}: {output:?}");
        assert!(!archive(&f, "macos-aarch64").exists(), "{env:?}");
        assert!(!f.cdn().join("macos-aarch64/latest").exists(), "{env:?}");
        assert!(f.uploads().is_empty(), "{env:?}");
    }
}

#[test]
fn an_unknown_platform_is_refused() {
    let f = Fixture::new();
    let output = f.run(&["linux-aarch64", "unknown"], false);
    assert_exit(&output, 2);
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown platform: unknown"));
    assert!(!f.dir.join("build.log").exists());
    assert!(!f.dir.join("dist/cdn").exists());
}

#[test]
fn publishing_without_the_account_id_does_nothing() {
    for mode in ["--publish", "--upload-only"] {
        let f = Fixture::new();
        let output = f.run(&[mode, "linux-aarch64"], false);
        assert_exit(&output, 2);
        assert!(String::from_utf8_lossy(&output.stderr).contains("CLOUDFLARE_ACCOUNT_ID"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("README"));
        assert!(!f.dir.join("build.log").exists());
        assert!(f.log().is_empty());
    }
}

#[test]
fn publishing_uploads_the_archive_and_its_checksum_before_latest() {
    let f = Fixture::new();
    assert_exit(&f.run(&["--publish", "linux-aarch64"], true), 0);
    let version = f.version();
    let lines: Vec<_> = f.log().lines().map(str::to_owned).collect();
    assert_eq!(lines.len(), 5, "{lines:?}");
    let base = format!("releases/{version}/tilr-{version}-linux-aarch64.tar.gz");
    for (line, name, content_type, cache) in [
        (
            &lines[0],
            base.as_str(),
            "application/gzip",
            "public, max-age=31536000, immutable",
        ),
        (
            &lines[1],
            format!("{base}.sha256").as_str(),
            "text/plain; charset=utf-8",
            "public, max-age=31536000, immutable",
        ),
        (
            &lines[3],
            "releases/linux-aarch64/latest",
            "text/plain; charset=utf-8",
            "no-cache",
        ),
    ] {
        assert!(
            line.starts_with(&format!("s3|cp|dist/cdn/{name}|s3://tilr-cdn/{name}|")),
            "{line}"
        );
        assert!(
            line.contains(&format!("|--content-type|{content_type}|")),
            "{line}"
        );
        assert!(
            line.contains(&format!("|--cache-control|{cache}|")),
            "{line}"
        );
        assert!(
            line.contains(
                "|--endpoint-url|https://fixture-account.r2.cloudflarestorage.com|--region|auto|"
            ),
            "{line}"
        );
    }
}

#[test]
fn beta_publication_uploads_only_archives_checksums_and_selected_beta_pointers() {
    let f = Fixture::new();
    let args = [
        "--publish",
        "--channel",
        "beta",
        "linux-x86_64",
        "linux-aarch64",
        "macos-aarch64",
        "macos-x86_64",
    ];
    let env = [("TILR_BUILD_VERSION", "9.9.0-rc.2")];
    assert_exit(&f.run_env(&args, true, &env), 0);
    let version = f.version_of("9.9.0-rc.2");
    let mut expected = Vec::new();
    for platform in [
        "linux-x86_64",
        "linux-aarch64",
        "macos-aarch64",
        "macos-x86_64",
    ] {
        let name = format!("releases/{version}/tilr-{version}-{platform}.tar.gz");
        expected.extend([
            name.clone(),
            format!("{name}.sha256"),
            format!("releases/{platform}/beta.manifest.json"),
            format!("releases/{platform}/beta"),
        ]);
    }
    assert_eq!(f.uploads(), expected);
    for line in f.log().lines().filter(|line| line.contains("/beta|")) {
        assert!(line.contains("|--cache-control|no-cache|"));
    }
    fs::remove_file(f.dir.join("aws.log")).unwrap();
    fs::remove_file(f.dir.join("build.log")).unwrap();
    assert_exit(
        &f.run(
            &["--upload-only", "--channel", "beta", "linux-x86_64"],
            true,
        ),
        0,
    );
    assert!(!f.dir.join("build.log").exists());
    assert_eq!(f.uploads(), expected[..4]);
    fs::remove_file(f.dir.join("aws.log")).unwrap();
    let name = format!("tilr-{version}-linux-x86_64.tar.gz");
    fs::write(f.cdn().join(&version).join(name), "corrupt").unwrap();
    assert_exit(
        &f.run(
            &["--upload-only", "--channel", "beta", "linux-x86_64"],
            true,
        ),
        1,
    );
    assert!(f.uploads().is_empty());
}

#[test]
fn beta_publication_failure_does_not_advance_the_pointer_or_publish_the_installer() {
    let f = Fixture::new();
    assert_exit(&f.run(&["linux-x86_64"], false), 0);
    let name = format!("releases/{0}/tilr-{0}-linux-x86_64.tar.gz", f.version());
    f.fail_upload(&format!("{name}.sha256"));
    assert_exit(
        &f.run(
            &["--upload-only", "--channel", "beta", "linux-x86_64"],
            true,
        ),
        1,
    );
    assert_eq!(f.uploads(), [name.clone(), format!("{name}.sha256")]);
}

#[test]
fn upload_only_checks_what_was_built_and_leaves_other_platforms_alone() {
    let f = Fixture::new();
    assert_exit(&f.run(&["linux-aarch64", "linux-x86_64"], false), 0);
    assert_exit(&f.run(&["--upload-only", "linux-aarch64"], true), 0);
    assert!(!f.log().contains("releases/linux-x86_64/latest"));

    fs::remove_file(f.cdn().join("linux-aarch64/latest")).unwrap();
    let output = f.run(&["--upload-only", "linux-aarch64"], true);
    assert_exit(&output, 1);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("nothing built for linux-aarch64: no ")
    );
    fs::write(
        f.cdn().join("linux-aarch64/latest"),
        format!("{}\n", f.version()),
    )
    .unwrap();

    fs::write(f.cdn().join("linux-x86_64/latest"), "other-version\n").unwrap();
    let output = f.run(&["--upload-only", "linux-aarch64", "linux-x86_64"], true);
    assert_exit(&output, 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!(
        "linux-x86_64 is built as other-version, not {}",
        f.version()
    )));
    fs::write(
        f.cdn().join("linux-x86_64/latest"),
        format!("{}\n", f.version()),
    )
    .unwrap();

    fs::write(archive(&f, "linux-aarch64"), "corrupt").unwrap();
    let output = f.run(&["--upload-only", "linux-aarch64"], true);
    assert_exit(&output, 1);
    assert!(String::from_utf8_lossy(&output.stdout).contains("tilr-"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("linux-aarch64.tar.gz: FAILED"));
    assert_eq!(
        f.log().lines().count(),
        5,
        "failed validation uploaded files"
    );
    assert_eq!(
        fs::read_to_string(f.dir.join("build.log")).unwrap(),
        "linux-aarch64\nlinux-x86_64\n"
    );
}

#[test]
fn the_installer_is_uploaded_after_latest() {
    let f = Fixture::new();
    assert_exit(&f.run(&["linux-aarch64", "linux-x86_64"], false), 0);
    let version = f.version();
    let archive = |platform: &str| format!("releases/{version}/tilr-{version}-{platform}.tar.gz");

    assert_exit(&f.run(&["--upload-only", "linux-x86_64"], true), 0);
    assert_eq!(
        f.uploads(),
        [
            archive("linux-x86_64"),
            format!("{}.sha256", archive("linux-x86_64")),
            "releases/linux-x86_64/latest.manifest.json".to_owned(),
            "releases/linux-x86_64/latest".to_owned(),
            "install.sh".to_owned(),
        ]
    );
    let line = f.log().lines().last().unwrap().to_owned();
    assert!(
        line.starts_with("s3|cp|scripts/install.sh|s3://tilr-cdn/install.sh|"),
        "{line}"
    );
    assert!(
        line.contains("|--content-type|text/x-shellscript; charset=utf-8|"),
        "{line}"
    );
    assert!(
        line.contains("|--cache-control|public, max-age=300|"),
        "{line}"
    );
    assert!(
        line.contains(
            "|--endpoint-url|https://fixture-account.r2.cloudflarestorage.com|--region|auto|"
        ),
        "{line}"
    );

    // With two platforms, after both latests.
    fs::remove_file(f.dir.join("aws.log")).unwrap();
    assert_exit(
        &f.run(&["--upload-only", "linux-aarch64", "linux-x86_64"], true),
        0,
    );
    let uploads = f.uploads();
    assert_eq!(uploads.len(), 9, "{uploads:?}");
    assert_eq!(uploads[3], "releases/linux-aarch64/latest");
    assert_eq!(uploads[7], "releases/linux-x86_64/latest");
    assert_eq!(uploads[8], "install.sh");

    // A failed upload stops it before the installer.
    fs::remove_file(f.dir.join("aws.log")).unwrap();
    f.fail_upload("releases/linux-x86_64/latest");
    let output = f.run(&["--upload-only", "linux-x86_64"], true);
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(f.uploads().last().unwrap(), "releases/linux-x86_64/latest");
    assert!(!f.uploads().contains(&"install.sh".to_owned()));
}

#[test]
fn the_layout_release_sh_writes_is_what_tilrd_update_reads() {
    let f = Fixture::new();
    let binary = Path::new(env!("CARGO_BIN_EXE_tilrd"));
    let (_, native) = version_and_platform();
    assert_exit(&f.run(&["--publish", &native], true), 0);

    let installed = f.dir.join("installed");
    fs::create_dir(&installed).unwrap();
    let exe = installed.join("tilrd");
    // cp writes only in its own process, so concurrent test children cannot
    // inherit a writable descriptor and keep this executable busy on Linux.
    assert!(
        Command::new("cp")
            .arg(binary)
            .arg(&exe)
            .status()
            .unwrap()
            .success()
    );
    let cdn = serve_files(f.dir.join("dist/cdn"));
    let output = Command::new(&exe)
        .args(["update", "--cdn", &cdn])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains(&f.version()));
    let updated = Command::new(&exe).arg("--version").output().unwrap();
    assert!(updated.status.success());
    assert!(String::from_utf8_lossy(&updated.stdout).contains(&f.version()));
    assert!(installed.join("tilr").is_file());
}

fn active(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

fn job<'a>(workflow: &'a str, name: &str) -> &'a str {
    let header = format!("\n  {name}:\n");
    let start = workflow
        .find(&header)
        .unwrap_or_else(|| panic!("missing job {name}"))
        + header.len();
    let rest = &workflow[start..];
    let end = rest
        .lines()
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len() + 1;
            Some((start, line))
        })
        .find(|(_, line)| {
            line.starts_with("  ") && !line.starts_with("    ") && line.ends_with(':')
        })
        .map_or(rest.len(), |(offset, _)| offset);
    &rest[..end]
}

#[test]
fn the_itests_rust_image_supports_the_workspace_msrv() {
    let workspace: toml::Value = toml::from_str(&text_file("Cargo.toml")).unwrap();
    let msrv = workspace["workspace"]["package"]["rust-version"]
        .as_str()
        .unwrap();
    let module = text_file("itests/src/tilr_itests/main.py");
    let image = module
        .lines()
        .find_map(|line| line.strip_prefix("RUST_IMAGE = \""))
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let tag = image.strip_prefix("rust:").unwrap();
    let version = tag.split('-').next().unwrap();
    let parse = |version: &str| {
        let mut parts = version.split('.').map(|part| part.parse::<u32>().unwrap());
        let result = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap_or(0),
        );
        assert!(parts.next().is_none());
        result
    };
    assert!(
        parse(version) >= parse(msrv),
        "itests image {image} is older than workspace MSRV {msrv}"
    );
}

#[test]
fn the_linux_build_runs_in_the_pinned_debian_12_image() {
    let docker = active(&text_file("scripts/linux.Dockerfile"));
    let from = docker
        .lines()
        .find(|line| line.starts_with("FROM "))
        .unwrap();
    assert!(from.starts_with("FROM rust:1.99-bookworm@sha256:"));
    assert_eq!(from.split("sha256:").nth(1).unwrap().len(), 64);
    assert!(docker.contains("protobuf-compiler"));
    assert!(docker.contains("install-tools.sh /opt/tilr-tools zig nextest"));
    let linux = active(&text_file("scripts/linux.sh"));
    assert!(linux.contains("scripts/linux.Dockerfile"));
    assert!(linux.contains("docker build"));
    assert!(linux.contains("TILR_LINUX_PLATFORM"));
    assert!(linux.contains("-e TILR_BUILD_COMMIT"));
    assert!(linux.contains("-e TILR_BUILD_VERSION"));
    let build = active(&text_file("scripts/linux-build.sh"));
    assert!(build.contains("scripts/linux.sh"));
    assert!(build.contains("-p tilrd -p tilr"));
    assert!(build.contains("/src/dist/linux-$(uname -m)"));
    assert!(build.contains("cp /target/release/tilrd /target/release/tilr"));
}

#[test]
fn the_linux_dev_container_builds_with_line_tables_only_debuginfo() {
    // The host's ~/.cargo/config.toml is not visible in the container.
    let linux = active(&text_file("scripts/linux.sh"));
    assert!(
        linux.contains(
            r#"-e CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-line-tables-only}""#
        ),
        "scripts/linux.sh must pass CARGO_PROFILE_DEV_DEBUG, line-tables-only by default, into the container"
    );
}

#[test]
fn build_tools_are_pinned_by_checksum_and_used_everywhere() {
    let tools = active(&text_file("scripts/install-tools.sh"));
    assert!(tools.contains("ZIG_VERSION=0.15.2"));
    assert!(tools.contains("PROTOC_VERSION=36.2"));
    assert!(tools.contains("NEXTEST_VERSION=0.9."));
    assert!(tools.contains("set -- zig protoc nextest"));
    for platform in [
        "Linux-x86_64",
        "Linux-aarch64",
        "Darwin-arm64",
        "Darwin-x86_64",
    ] {
        let branch = tools
            .split(platform)
            .nth(1)
            .unwrap()
            .split(";;")
            .next()
            .unwrap();
        for key in ["zig_sha256=", "protoc_sha256=", "nextest_sha256="] {
            let hash = branch.split(key).nth(1).unwrap().lines().next().unwrap();
            assert_eq!(hash.len(), 64, "{platform} {key}");
            assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }
    assert!(tools.contains("fetch \"https://ziglang.org/download/"));
    assert!(tools.contains("\"$zig_sha256\""));
    assert!(tools.contains("\"$protoc_sha256\""));
    assert!(tools.contains(
        "fetch \"https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-$NEXTEST_VERSION/"
    ));
    assert!(tools.contains("\"$nextest_sha256\""));
    assert!(tools.contains("\"$dir/cargo-nextest\""));
    assert!(tools.contains("[ \"$got\" = \"$2\" ]"));
    for workflow in [".github/workflows/ci.yml", ".github/workflows/release.yml"] {
        assert!(
            active(&text_file(workflow)).contains("scripts/install-tools.sh"),
            "{workflow}"
        );
    }
    let docker = active(&text_file("scripts/linux.Dockerfile"));
    assert!(docker.contains("install-tools.sh /opt/tilr-tools zig nextest"));
    assert!(docker.contains("ln -s /opt/tilr-tools/cargo-nextest /usr/local/bin/cargo-nextest"));
    let ci = active(&text_file(".github/workflows/ci.yml"));
    // The test job installs the default set, cargo-nextest included.
    assert!(
        job(&ci, "test").contains("scripts/install-tools.sh \"$RUNNER_TEMP/tools\"\n"),
        "the test job must install cargo-nextest"
    );
    for gate in ["DENY_SHA256:", "GITLEAKS_SHA256:", "sha256sum -c -"] {
        assert!(ci.contains(gate), "{gate}");
    }
}

/// AGENTS.md, README, the container's default command and CI run the tests with one and
/// the same command, so the static CI check above checks what developers run.
#[test]
fn the_test_command_is_the_same_everywhere() {
    for file in [
        "AGENTS.md",
        "README.md",
        "USE_CASES.md",
        "scripts/linux.sh",
        ".github/workflows/ci.yml",
    ] {
        let text = text_file(file);
        assert!(
            text.contains("cargo nextest run --workspace --features tilrd/failpoints --locked"),
            "{file} must name the nextest command"
        );
        assert!(
            !text.contains("cargo test --workspace"),
            "{file} still names cargo test"
        );
    }
}

/// Release documentation must be present in both the build and runtime snapshots.
#[test]
fn repository_documentation_matches_the_build_snapshot() {
    for (file, compiled) in [
        ("AGENTS.md", include_str!("../../../AGENTS.md")),
        ("README.md", include_str!("../../../README.md")),
        ("USE_CASES.md", include_str!("../../../USE_CASES.md")),
    ] {
        assert_eq!(text_file(file), compiled, "{file}");
    }
}

const MSRV_GATES: &[&str] = &[
    "rustup toolchain install 1.99 --profile minimal",
    "rustup default 1.99",
    "- run: cargo check --workspace --all-targets --features tilrd/failpoints --locked",
];

const LINT_GATES: &[&str] = &[
    "- run: cargo fmt --all --check",
    "- run: cargo clippy --workspace --all-targets --locked -- -D warnings",
    "- run: cargo clippy --workspace --all-targets --features tilrd/failpoints --locked -- -D warnings",
];

const TEST_GATES: &[&str] =
    &["- run: cargo nextest run --workspace --features tilrd/failpoints --locked"];

const DENY_GATES: &[&str] = &["- run: '\"$CARGO_DENY\" --locked check'"];

const SECRETS_GATES: &[&str] =
    &["- run: '\"$GITLEAKS\" git --redact --no-banner --exit-code 1 --log-opts=HEAD .'"];

// The same job-specific gates drive assertions and deletion mutations.
// Gitleaks must scan only HEAD's history, even with a full-history checkout.
const CI_GATES: &[(&str, &[&str])] = &[
    ("msrv", MSRV_GATES),
    ("lint", LINT_GATES),
    ("test", TEST_GATES),
    ("deny", DENY_GATES),
    ("secrets", SECRETS_GATES),
];

fn assert_ci_gates(ci: &str) {
    let ci = active(ci);
    let on = ci.split("\npermissions:").next().unwrap();
    assert!(on.contains("\n  merge_group:\n"), "CI skips merge groups");
    for event in ["pull_request", "push"] {
        assert!(
            on.contains(&format!("  {event}:\n    branches: [main, 'release/**']\n")),
            "CI skips {event} of main or release/**"
        );
    }
    let secrets = job(&ci, "secrets");
    let test = job(&ci, "test");
    let checkout = secrets.split("- name: Install gitleaks").next().unwrap();
    assert!(checkout.contains("uses: actions/checkout@"));
    assert!(checkout.contains("fetch-depth: 0"));
    for (name, gates) in CI_GATES {
        let section = job(&ci, name);
        for gate in *gates {
            assert!(section.contains(gate), "missing {name} gate: {gate}");
        }
    }
    assert!(test.contains("os: [ubuntu-latest, macos-15]"));
}

#[test]
fn ci_runs_every_gate_of_agents_md() {
    let ci = text_file(".github/workflows/ci.yml");
    assert_ci_gates(&ci);
    let removed = ci.replace(&format!("  msrv:\n{}", job(&ci, "msrv")), "");
    assert_ne!(removed, ci);
    assert!(std::panic::catch_unwind(|| assert_ci_gates(&removed)).is_err());
    for clause in CI_GATES
        .iter()
        .flat_map(|(_, gates)| gates.iter().copied())
        .chain([
            "  merge_group:\n",
            ", 'release/**'",
            "fetch-depth: 0",
            " --log-opts=HEAD",
        ])
    {
        let removed = ci.replace(clause, "");
        assert_ne!(removed, ci, "mutation did not remove {clause}");
        assert!(
            std::panic::catch_unwind(|| assert_ci_gates(&removed)).is_err(),
            "accepted missing {clause}"
        );
    }
}

fn assert_smoke_contract(ci: &str) {
    let ci = active(ci);
    let test = job(&ci, "test");
    for clause in [
        "cargo build -p tilrd -p tilr --locked",
        "for exe in tilrd tilr; do",
        "case \"$(\"target/debug/$exe\" --version)\" in",
        "\"$exe $version-$(git rev-parse --short HEAD) (\"*\")\") ;;",
        "*) \"target/debug/$exe\" --version; exit 1 ;;",
        "\"target/debug/$exe\" --help >/dev/null",
        "git rev-parse --short HEAD",
        "cargo metadata --format-version 1 --no-deps --locked",
    ] {
        assert!(test.contains(clause), "missing smoke check: {clause}");
    }
}

#[test]
fn ci_smoke_tests_both_executables() {
    let ci = text_file(".github/workflows/ci.yml");
    assert_smoke_contract(&ci);
    let removed = ci.replace("$exe $version-", "$exe wrong-");
    assert_ne!(removed, ci);
    assert!(std::panic::catch_unwind(|| assert_smoke_contract(&removed)).is_err());
}

#[test]
fn release_tags_supply_the_version_without_release_configuration() {
    assert!(!root().join("release-please-config.json").exists());
    assert!(!root().join(".release-please-manifest.json").exists());
    let workflow = text_file(".github/workflows/release.yml");
    assert_reusable_release_call(&workflow);
    assert!(job(&workflow, "build").contains("python3 scripts/check-release-version.py \"$TAG\""));
    let result = Command::new("python3")
        .arg(root().join("scripts/test_check_release_version.py"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn assert_reusable_release_call(release: &str) {
    let release = active(release);
    let selection = job(&release, "release");
    let reference = selection
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("uses: tilr-dev/.github/.github/workflows/release-branch.yml@")
        })
        .expect("missing reusable release workflow");
    assert_eq!(
        reference.len(),
        40,
        "release workflow must use a full commit SHA"
    );
    assert!(reference.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(selection.contains(&format!("      workflow-sha: {reference}\n")));
    for clause in [
        "    permissions:\n      contents: read\n",
        "      app-client-id: ${{ vars.TILR_BOT_CLIENT_ID }}\n",
        "    secrets:\n      app-private-key: ${{ secrets.TILR_BOT_PRIVATE_KEY }}\n",
    ] {
        assert!(selection.contains(clause), "release missing {clause}");
    }
    for forbidden in ["    steps:", "    runs-on:", "secrets: inherit", ": write"] {
        assert!(
            !selection.contains(forbidden),
            "caller contains {forbidden}"
        );
    }
}

fn assert_release_branches(release: &str) {
    let release = active(release);
    let on = release.split("\npermissions:").next().unwrap();
    assert!(
        on.contains("on:\n  push:\n    branches: ['release/**']\n    tags: ['v*', '!v*-rc.*']\n"),
        "the release workflow must run on release branches and stable tag pushes"
    );
    assert!(!on.contains("main"), "the release workflow runs on main");
    assert!(!on.contains("pull_request"));
    assert_reusable_release_call(&release);
    assert!(
        !release.contains("\n  back-merge:\n"),
        "a back-merge job is back"
    );
    for forbidden in ["--base dev", "gh pr merge", "gh pr create"] {
        assert!(!release.contains(forbidden), "release.yml runs {forbidden}");
    }
}

#[test]
fn releases_run_only_on_release_branches() {
    let release = text_file(".github/workflows/release.yml");
    assert_release_branches(&release);
    let workflow_sha = release
        .lines()
        .find(|line| line.trim_start().starts_with("workflow-sha:"))
        .unwrap();
    let mismatched = release.replacen(
        workflow_sha,
        &format!("      workflow-sha: {}", "0".repeat(40)),
        1,
    );
    assert_ne!(mismatched, release);
    assert!(std::panic::catch_unwind(|| assert_release_branches(&mismatched)).is_err());
    for (from, to) in [
        ("branches: ['release/**']", "branches: [main]"),
        ("branches: ['release/**']", "branches: ['release/**', main]"),
        ("release-branch.yml@", "release-branch.yml@main"),
        ("      workflow-sha:", "      wrong-sha:"),
        ("      contents: read", "      contents: write"),
        ("tags: ['v*', '!v*-rc.*']", "tags: ['v*']"),
        ("vars.TILR_BOT_CLIENT_ID", "secrets.TILR_BOT_CLIENT_ID"),
        ("      app-private-key:", "      wrong-private-key:"),
        ("  build:\n", "  back-merge:\n    runs-on: x\n  build:\n"),
    ] {
        let mutated = release.replacen(from, to, 1);
        assert_ne!(mutated, release);
        assert!(
            std::panic::catch_unwind(|| assert_release_branches(&mutated)).is_err(),
            "accepted {from} -> {to}"
        );
    }
}

fn assert_candidate_contract(release: &str) {
    let release = active(release);
    assert_reusable_release_call(&release);
    for name in ["build", "upload", "cdn"] {
        assert!(
            job(&release, name).contains("    if: needs.release.outputs.tag != ''\n"),
            "{name} does not run for a candidate"
        );
    }
    for name in ["build", "upload", "cdn"] {
        assert!(
            job(&release, name).contains("TAG: ${{ needs.release.outputs.tag }}"),
            "{name} does not consume the reusable workflow tag"
        );
    }
    let cdn = job(&release, "cdn");
    assert!(
        cdn.contains(
            "CHANNEL: ${{ needs.release.outputs.is_rc == 'false' && 'stable' || 'beta' }}"
        )
    );
    assert!(cdn.contains("scripts/release.sh --upload-only --channel \"$CHANNEL\" $PLATFORMS"));
    let build = job(&release, "build");
    for clause in [
        "TILR_BUILD_VERSION=\"${TAG#v}\" cargo build --release --locked -p tilrd -p tilr --target \"$TARGET\"",
        "TILR_BUILD_VERSION=\"${TAG#v}\" scripts/release.sh \"$PLATFORM\"",
    ] {
        assert!(build.contains(clause), "build missing {clause}");
    }
}

#[test]
fn a_release_candidate_is_built_like_a_release_and_publishes_to_beta() {
    let release = text_file(".github/workflows/release.yml");
    assert_candidate_contract(&release);
    for (from, to) in [
        (
            "    if: needs.release.outputs.tag != ''\n",
            "    if: needs.release.outputs.is_rc == 'false'\n",
        ),
        (
            "    if: needs.release.outputs.tag != ''\n    runs-on: ubuntu-latest\n    timeout-minutes: 15\n",
            "    if: needs.release.outputs.is_rc == 'false'\n    runs-on: ubuntu-latest\n    timeout-minutes: 15\n",
        ),
        (
            "TILR_BUILD_VERSION=\"${TAG#v}\" scripts/release.sh",
            "scripts/release.sh",
        ),
        (
            "needs.release.outputs.tag }}",
            "needs.release.outputs.tag_name }}",
        ),
        (
            "outputs.is_rc == 'false' && 'stable' || 'beta'",
            "outputs.is_rc == 'false' && 'beta' || 'stable'",
        ),
    ] {
        let mutated = release.replacen(from, to, 1);
        assert_ne!(mutated, release, "mutation did not apply: {from}");
        assert!(
            std::panic::catch_unwind(|| assert_candidate_contract(&mutated)).is_err(),
            "accepted {from} -> {to}"
        );
    }
}

fn assert_spec_change_contract(workflow: &str) {
    let workflow = active(workflow);
    assert!(workflow.contains("  pull_request_target:\n    branches: [main]\n"));
    assert!(workflow.contains("  pull-requests: read"));
    assert!(!workflow.contains("actions/checkout"));
    let mut pinned = Vec::new();
    for script in ["spec_merge.py", "spec_change.py"] {
        let url = format!("/scripts/{script} \\");
        let line = workflow
            .lines()
            .find(|line| line.trim_end().ends_with(&url))
            .unwrap_or_else(|| panic!("{script} is not downloaded"));
        let commit = line
            .trim()
            .strip_prefix("https://raw.githubusercontent.com/tilr-dev/gitpm/")
            .and_then(|rest| rest.split_once('/'))
            .map(|(commit, _)| commit)
            .unwrap_or_else(|| panic!("{script} is not from tilr-dev/gitpm"));
        assert!(
            commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
            "{script} is fetched from {commit}, not a fixed commit"
        );
        pinned.push(commit.to_owned());
        let check = format!("  %s\\n' \"$RUNNER_TEMP/{script}\" | sha256sum --check --strict");
        assert!(workflow.contains(&check), "{script} is not checked");
    }
    assert_eq!(
        pinned[0], pinned[1],
        "the shared scripts come from different commits"
    );
    assert!(
        workflow
            .find("python3 \"$RUNNER_TEMP/spec_merge.py\"")
            .unwrap()
            < workflow
                .find("python3 \"$RUNNER_TEMP/spec_change.py\"")
                .unwrap()
    );
    assert!(workflow.contains("--component tilrd"));
    assert!(workflow.contains("--base refs/inspection/base --head refs/inspection/merge"));
}

#[test]
fn the_spec_change_check_compares_main_with_the_test_merge() {
    let workflow = text_file(".github/workflows/spec-change.yml");
    assert_spec_change_contract(&workflow);
    let merge_url = workflow
        .lines()
        .find(|line| line.contains("/scripts/spec_merge.py \\"))
        .unwrap();
    for (from, to) in [
        (
            merge_url.to_owned(),
            merge_url.replace(merge_url.trim().split('/').nth(5).unwrap(), "main"),
        ),
        (
            "--head refs/inspection/merge".to_owned(),
            "--head refs/inspection/head".to_owned(),
        ),
        ("  pull-requests: read".to_owned(), String::new()),
        ("branches: [main]".to_owned(), "branches: [dev]".to_owned()),
    ] {
        let mutated = workflow.replacen(&from, &to, 1);
        assert_ne!(mutated, workflow, "mutation did not apply: {from}");
        assert!(
            std::panic::catch_unwind(|| assert_spec_change_contract(&mutated)).is_err(),
            "accepted {from} -> {to}"
        );
    }
}

fn assert_release_contract(release: &str, package: &str) {
    let release = active(release);
    let build = job(&release, "build");
    for target in [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
    ] {
        assert!(build.contains(target), "{target}");
    }
    for clause in [
        "vars.RELEASE_LINUX_ARM64 == 'true'",
        "\"platform\":\"macos-aarch64\"",
        "\"platform\":\"macos-x86_64\"",
        "scripts/release.sh \"$PLATFORM\"",
        "scripts/package.sh \"$TAG\" \"$TARGET\" \"$BIN_DIR\" dist/github",
        "name: tilr-${{ matrix.target }}",
        "name: cdn-${{ matrix.platform }}",
    ] {
        assert!(build.contains(clause), "{clause}");
    }
    assert!(
        !build.contains("secrets.APPLE_"),
        "build can access Apple credentials"
    );
    assert!(!build.contains("environment:"), "build has an environment");
    assert!(build.contains("name: unsigned-${{ matrix.platform }}"));
    let signing = job(&release, "sign-macos");
    for forbidden in [
        "cargo ",
        "npm ",
        "actions/checkout",
        "scripts/",
        "$GITHUB_ENV",
        "APPLE_ID",
        "APPLE_NOTARY_PASSWORD",
        "--apple-id",
    ] {
        assert!(!signing.contains(forbidden), "signing contains {forbidden}");
    }
    for clause in [
        "needs: build",
        "environment: apple-signing",
        "secrets.APPLE_",
        "name: unsigned-${{ matrix.platform }}",
        "shasum -a 256 -c binaries.sha256",
        "security import",
        "security list-keychains -d user -s \"$keychain\" \"${keychains[@]}\"",
        " -x ",
        "codesign --force --sign",
        "--options runtime --timestamp",
        "secrets.APPLE_API_KEY_P8_BASE64",
        "notarytool store-credentials tilr-release --key \"$RUNNER_TEMP/AuthKey.p8\"",
        "rm -f \"$RUNNER_TEMP/certificate.p12\" \"$RUNNER_TEMP/AuthKey.p8\"",
        "rm \"$RUNNER_TEMP/AuthKey.p8\"\n",
        "notarytool submit",
        "--wait",
        "!= \"Accepted\"",
        "if: always()",
        "security delete-keychain",
        "name: signed-${{ matrix.platform }}",
    ] {
        assert!(signing.contains(clause), "signing missing {clause}");
    }
    assert!(signing.find("shasum -a 256 -c").unwrap() < signing.find("security import").unwrap());
    assert!(
        signing.find("security list-keychains -d user -s").unwrap()
            < signing.find("codesign --force").unwrap()
    );
    assert!(signing.find("!= \"Accepted\"").unwrap() < signing.find("name: signed-").unwrap());
    let macos_package = job(&release, "package-macos");
    assert!(macos_package.contains("needs: [release, sign-macos]"));
    assert!(macos_package.contains("name: signed-${{ matrix.platform }}"));
    assert!(!macos_package.contains("unsigned-"));
    assert!(!macos_package.contains("secrets.APPLE_"));
    assert!(macos_package.contains("scripts/package.sh"));
    assert!(macos_package.contains("scripts/cdn-layout.sh \"$version\" \"$PLATFORM\""));
    assert!(macos_package.contains("name: cdn-${{ matrix.platform }}"));
    let version = build
        .split("- name: Check the version\n")
        .nth(1)
        .expect("missing version check");
    let version = version.split("      - ").next().unwrap();
    for clause in [
        "if: matrix.native",
        "for exe in tilrd tilr; do",
        "case \"$(\"$BIN_DIR/$exe\" --version)\" in",
        "\"$exe ${TAG#v}-$(git rev-parse --short HEAD) (\"*\")\") ;;",
        "*) \"$BIN_DIR/$exe\" --version; exit 1 ;;",
    ] {
        assert!(
            version.contains(clause),
            "missing native version check: {clause}"
        );
    }
    let package = active(package);
    for clause in [
        "NAME=tilr-$TAG-$TARGET",
        "cp \"$BIN_DIR/tilrd\" \"$BIN_DIR/tilr\" \"$STAGE/$NAME/\"",
        "cp \"$ROOT/LICENSE-APACHE\" \"$ROOT/LICENSE-MIT\" \"$ROOT/README.md\" \"$STAGE/$NAME/\"",
        "COPYFILE_DISABLE=1 tar -czf \"$OUT/$NAME.tar.gz\" -C \"$STAGE\" \"$NAME\"",
        "(cd \"$OUT\" && shasum -a 256 \"$NAME.tar.gz\" >\"$NAME.tar.gz.sha256\")",
    ] {
        assert!(
            package.contains(clause),
            "missing packaging contract: {clause}"
        );
    }
    let upload = job(&release, "upload");
    assert!(upload.contains("needs: [release, build, package-macos]"));
    assert!(upload.contains("gh release upload"));
}

#[test]
fn a_release_builds_and_packages_every_target() {
    let release = text_file(".github/workflows/release.yml");
    let package = text_file("scripts/package.sh");
    assert_release_contract(&release, &package);
    for clause in [
        "security list-keychains -d user -s",
        " \"$RUNNER_TEMP/AuthKey.p8\"\n",
        "scripts/cdn-layout.sh \"$version\" \"$PLATFORM\"",
        "if: matrix.native",
        "${TAG#v}-$(git rev-parse --short HEAD)",
    ] {
        let removed = release.replace(clause, "");
        assert_ne!(removed, release);
        assert!(std::panic::catch_unwind(|| assert_release_contract(&removed, &package)).is_err());
    }
    for (from, to) in [
        (
            "  build:\n",
            "  build:\n    env:\n      KEY: ${{ secrets.APPLE_SIGN_IDENTITY }}\n",
        ),
        (
            "  sign-macos:\n",
            "  sign-macos:\n    # comment\n    command: cargo build\n",
        ),
        (
            "  sign-macos:\n",
            "  sign-macos:\n    command: npm install\n",
        ),
        (
            "  sign-macos:\n",
            "  sign-macos:\n    command: actions/checkout@unsafe\n",
        ),
        (
            "  sign-macos:\n",
            "  sign-macos:\n    command: echo KEY >> $GITHUB_ENV\n",
        ),
        (
            "name: signed-${{ matrix.platform }}",
            "name: unsigned-${{ matrix.platform }}",
        ),
        (" -x -T /usr/bin/codesign", " -T /usr/bin/codesign"),
        ("!= \"Accepted\"", "!= \"Invalid\""),
    ] {
        let mutated = release.replace(from, to);
        assert_ne!(mutated, release);
        assert!(
            std::panic::catch_unwind(|| assert_release_contract(&mutated, &package)).is_err(),
            "accepted unsafe workflow mutation: {from} -> {to}"
        );
    }
    for clause in [
        "\"$BIN_DIR/tilrd\"",
        "\"$BIN_DIR/tilr\"",
        "\"$ROOT/LICENSE-APACHE\"",
        "\"$ROOT/LICENSE-MIT\"",
        "\"$ROOT/README.md\"",
        "tar -czf",
        "shasum -a 256",
        "$NAME.tar.gz.sha256",
    ] {
        let removed = package.replace(clause, "");
        assert_ne!(removed, package, "mutation did not remove {clause}");
        assert!(
            std::panic::catch_unwind(|| assert_release_contract(&release, &removed)).is_err(),
            "accepted missing {clause}"
        );
    }
}

fn assert_cdn_contract(release: &str) {
    let release = active(release);
    let cdn = job(&release, "cdn");
    for clause in [
        "needs: [release, upload]",
        "environment: cdn",
        "vars.RELEASE_LINUX_ARM64 == 'true' && 'linux-aarch64 linux-x86_64 macos-aarch64 macos-x86_64' || 'linux-x86_64 macos-aarch64 macos-x86_64'",
        "pattern: cdn-*",
        "merge-multiple: true",
        "for platform in $PLATFORMS",
        "want=\"${TAG#v}-$(git rev-parse --short HEAD)\"",
        "test \"$(cat \"dist/cdn/releases/$platform/latest\")\" = \"$want\"",
        "TILR_UPDATE_SIGNING_KEY: ${{ secrets.TILR_UPDATE_SIGNING_KEY }}",
        "AWS_ACCESS_KEY_ID: ${{ secrets.AWS_ACCESS_KEY_ID }}",
        "AWS_SECRET_ACCESS_KEY: ${{ secrets.AWS_SECRET_ACCESS_KEY }}",
        "run: aws --version",
        "CLOUDFLARE_ACCOUNT_ID:",
        "scripts/release.sh --upload-only --channel \"$CHANNEL\" $PLATFORMS",
    ] {
        assert!(cdn.contains(clause), "{clause}");
    }
}

#[test]
fn the_cdn_job_runs_after_the_assets_and_uploads_only_what_was_built() {
    let release = text_file(".github/workflows/release.yml");
    assert_cdn_contract(&release);
    let removed = release.replace("= \"$want\"", "= wrong");
    assert_ne!(removed, release);
    assert!(std::panic::catch_unwind(|| assert_cdn_contract(&removed)).is_err());
}

#[test]
fn the_pinned_zig_selects_a_compatible_macos_sdk() {
    for (workflow, job_name, next) in [
        (
            ".github/workflows/ci.yml",
            "test",
            "name: Install Zig, protoc and cargo-nextest",
        ),
        (
            ".github/workflows/release.yml",
            "build",
            "name: Build (macOS)",
        ),
    ] {
        let yaml = active(&text_file(workflow));
        let section = job(&yaml, job_name);
        let select = section.find("name: Select macOS 15.4 SDK").unwrap();
        assert!(
            select < section.find(next).unwrap(),
            "{workflow}: SDK selected too late"
        );
        assert!(section.contains("DEVELOPER_DIR=/Applications/Xcode_16.3.app/Contents/Developer"));
        assert!(section.contains(
            "SDKROOT=/Applications/Xcode_16.3.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX15.4.sdk"
        ));
    }
    assert!(!active(&text_file("scripts/linux.Dockerfile")).contains("MacOSX15.4.sdk"));
}

#[test]
fn the_macos_zig_installer_checks_its_sdk_before_download() {
    let output = Command::new("sh")
        .current_dir(root())
        .arg("scripts/test-macos-sdk.sh")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn publication_requires_a_matching_signing_key_before_uploading() {
    let f = Fixture::new();
    assert_exit(&f.run(&["linux-x86_64"], false), 0);
    for env in [
        ("TILR_UPDATE_SIGNING_KEY", ""),
        ("TILR_UPDATE_PUBLIC_KEY", "00"),
        ("TILR_UPDATE_SIGNING_KEY", "not a private key"),
    ] {
        let output = f.run_env(&["--upload-only", "linux-x86_64"], true, &[env]);
        assert!(!output.status.success(), "{env:?}: {output:?}");
        assert!(f.uploads().is_empty());
    }
}

#[test]
fn a_failed_manifest_upload_does_not_advance_the_unsigned_pointer() {
    let f = Fixture::new();
    assert_exit(&f.run(&["linux-x86_64"], false), 0);
    let manifest = "releases/linux-x86_64/beta.manifest.json";
    f.fail_upload(manifest);
    let output = f.run(
        &["--upload-only", "--channel", "beta", "linux-x86_64"],
        true,
    );
    assert!(!output.status.success(), "{output:?}");
    let uploads = f.uploads();
    assert!(!uploads.is_empty(), "{output:?}");
    assert_eq!(uploads.last().unwrap(), manifest, "{output:?}");
    assert!(
        !f.uploads()
            .iter()
            .any(|path| path.ends_with("/beta") || path == "install.sh")
    );
}
