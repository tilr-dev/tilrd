//! Use cases: release.sh builds a CDN layout consumed by tilrd update, and CI
//! keeps the release gates and packaging contract.

use std::fs;
#[path = "common/cdn.rs"]
mod cdn;
use cdn::serve_files;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

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
        let dir = std::env::temp_dir().join(format!(
            "tilrd-release-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::create_dir(dir.join("scripts")).unwrap();
        fs::create_dir(dir.join("bin")).unwrap();
        for script in ["release.sh", "install.sh"] {
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
  printf '#!/bin/sh\nprintf "%%s\\n" "%s 9.8.7-%s (%s)"\n' "$exe" "$TILR_BUILD_COMMIT" "$platform" > "dist/$platform/$exe"
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
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "release@test.invalid"]);
        git(&dir, &["config", "user.name", "Release Test"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "fixture"]);
        Self { dir }
    }

    fn run(&self, args: &[&str], account: bool) -> Output {
        let mut command = Command::new("sh");
        command
            .current_dir(&self.dir)
            .arg("scripts/release.sh")
            .args(args);
        command.env(
            "PATH",
            format!(
                "{}:{}",
                self.dir.join("bin").display(),
                std::env::var("PATH").unwrap()
            ),
        );
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
        let output = Command::new("git")
            .current_dir(&self.dir)
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .unwrap();
        format!("9.8.7-{}", String::from_utf8(output.stdout).unwrap().trim())
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
    let f = Fixture::new();
    let output = f.run(&["--publish", "linux-aarch64"], false);
    assert_exit(&output, 2);
    assert!(String::from_utf8_lossy(&output.stderr).contains("CLOUDFLARE_ACCOUNT_ID"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("README"));
    assert!(!f.dir.join("build.log").exists());
    assert!(f.log().is_empty());
}

#[test]
fn publishing_uploads_the_archive_and_its_checksum_before_latest() {
    let f = Fixture::new();
    assert_exit(&f.run(&["--publish", "linux-aarch64"], true), 0);
    let version = f.version();
    let lines: Vec<_> = f.log().lines().map(str::to_owned).collect();
    assert_eq!(lines.len(), 4, "{lines:?}");
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
            &lines[2],
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
        4,
        "failed validation uploaded files"
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
    assert_eq!(uploads.len(), 7, "{uploads:?}");
    assert_eq!(uploads[2], "releases/linux-aarch64/latest");
    assert_eq!(uploads[5], "releases/linux-x86_64/latest");
    assert_eq!(uploads[6], "install.sh");

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
    let output = Command::new(binary).arg("--version").output().unwrap();
    assert!(output.status.success());
    let version_line = String::from_utf8(output.stdout).unwrap();
    let native = version_line
        .split_whitespace()
        .nth(2)
        .unwrap()
        .trim_matches(['(', ')']);
    let built = if native.starts_with("linux-") {
        native
    } else {
        "linux-aarch64"
    };
    assert_exit(&f.run(&[built], false), 0);

    // release.sh currently builds Linux only. On macOS, mirror its archive under
    // the native platform name so the native updater can consume the same layout.
    if native != built {
        let version = f.version();
        let native_dir = f.cdn().join(native);
        fs::create_dir_all(&native_dir).unwrap();
        fs::copy(
            f.cdn().join(built).join("latest"),
            native_dir.join("latest"),
        )
        .unwrap();
        let source = archive(&f, built);
        let target = f
            .cdn()
            .join(&version)
            .join(format!("tilr-{version}-{native}.tar.gz"));
        fs::copy(&source, &target).unwrap();
        fs::copy(
            source.with_extension("gz.sha256"),
            target.with_extension("gz.sha256"),
        )
        .unwrap();
    }

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

fn assert_ci_gates(ci: &str) {
    let ci = active(ci);
    let lint = job(&ci, "lint");
    let test = job(&ci, "test");
    let deny = job(&ci, "deny");
    let secrets = job(&ci, "secrets");
    let msrv = job(&ci, "msrv");
    for gate in [
        "rustup toolchain install 1.99 --profile minimal",
        "rustup default 1.99",
        "- run: cargo check --workspace --all-targets --features tilrd/failpoints --locked",
    ] {
        assert!(msrv.contains(gate), "missing MSRV gate: {gate}");
    }
    let checkout = secrets.split("- name: Install gitleaks").next().unwrap();
    assert!(checkout.contains("uses: actions/checkout@"));
    assert!(checkout.contains("fetch-depth: 0"));
    for gate in [
        "- run: cargo fmt --all --check",
        "- run: cargo clippy --workspace --all-targets --locked -- -D warnings",
        "- run: cargo clippy --workspace --all-targets --features tilrd/failpoints --locked -- -D warnings",
    ] {
        assert!(lint.contains(gate), "missing CI gate: {gate}");
    }
    assert!(
        test.contains("- run: cargo nextest run --workspace --features tilrd/failpoints --locked")
    );
    assert!(deny.contains("- run: '\"$CARGO_DENY\" --locked check'"));
    assert!(secrets.contains("- run: '\"$GITLEAKS\" git --redact --no-banner --exit-code 1 .'"));
    assert!(test.contains("os: [ubuntu-latest, macos-15]"));
}

#[test]
fn ci_runs_every_gate_of_agents_md() {
    let ci = text_file(".github/workflows/ci.yml");
    assert_ci_gates(&ci);
    let removed = ci.replace(&format!("  msrv:\n{}", job(&ci, "msrv")), "");
    assert_ne!(removed, ci);
    assert!(std::panic::catch_unwind(|| assert_ci_gates(&removed)).is_err());
    for clause in [
        "rustup toolchain install 1.99 --profile minimal",
        "rustup default 1.99",
        "- run: cargo check --workspace --all-targets --features tilrd/failpoints --locked",
        "fetch-depth: 0",
        "- run: cargo fmt --all --check",
        "- run: cargo clippy --workspace --all-targets --locked -- -D warnings",
        "- run: cargo clippy --workspace --all-targets --features tilrd/failpoints --locked -- -D warnings",
        "- run: cargo nextest run --workspace --features tilrd/failpoints --locked",
        "- run: '\"$CARGO_DENY\" --locked check'",
        "- run: '\"$GITLEAKS\" git --redact --no-banner --exit-code 1 .'",
    ] {
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
fn release_please_bumps_the_workspace_version_and_lock() {
    let config: serde_json::Value =
        serde_json::from_str(&text_file("release-please-config.json")).unwrap();
    assert_eq!(config["release-type"], "simple");
    let package = &config["packages"]["."];
    assert_eq!(package["changelog-path"], "CHANGELOG.md");
    let files = package["extra-files"].as_array().unwrap();
    assert!(
        files.iter().any(|file| file["path"] == "Cargo.toml"
            && file["jsonpath"] == "$.workspace.package.version")
    );
    // Every workspace crate, so a new member cannot be left at the old version in Cargo.lock.
    let workspace: toml::Value = toml::from_str(&text_file("Cargo.toml")).unwrap();
    let members = workspace["workspace"]["members"].as_array().unwrap();
    assert!(!members.is_empty());
    for member in members {
        let manifest: toml::Value = toml::from_str(&text_file(&format!(
            "{}/Cargo.toml",
            member.as_str().unwrap()
        )))
        .unwrap();
        let name = manifest["package"]["name"].as_str().unwrap();
        assert!(
            files.iter().any(|file| file["path"] == "Cargo.lock"
                && file["jsonpath"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("name.value=='{name}'"))),
            "{name}"
        );
    }
    let release = active(&text_file(".github/workflows/release.yml"));
    let please = job(&release, "release-please");
    for clause in [
        "googleapis/release-please-action@",
        "config-file: release-please-config.json",
        "manifest-file: .release-please-manifest.json",
    ] {
        assert!(please.contains(clause));
    }
}

#[test]
fn a_created_release_back_merges_main_into_dev() {
    let release = active(&text_file(".github/workflows/release.yml"));
    let back_merge = job(&release, "back-merge");
    for clause in [
        "needs: release-please",
        "if: needs.release-please.outputs.release_created == 'true'",
        "permissions: {}",
        "actions/create-github-app-token@",
        "client-id: ${{ vars.TILR_BOT_CLIENT_ID }}",
        "private-key: ${{ secrets.TILR_BOT_PRIVATE_KEY }}",
        "repositories: ${{ github.event.repository.name }}",
        "permission-contents: write",
        "permission-pull-requests: write",
        "GH_TOKEN: ${{ steps.app.outputs.token }}",
        "GH_REPO: ${{ github.repository }}",
        "TAG: ${{ needs.release-please.outputs.tag_name }}",
        "shell: bash",
        "gh pr list --base dev --head main --state open --json number",
        "gh pr create --base dev --head main --title \"chore: back-merge $TAG\"",
        "if ! gh pr merge \"$pr\" --merge; then",
    ] {
        assert!(
            back_merge.contains(clause),
            "missing back-merge contract: {clause}"
        );
    }
    assert!(!back_merge.contains("continue-on-error:"));
    assert!(!back_merge.contains("actions/checkout@"));
    assert!(!back_merge.contains("--squash"));
    assert!(!back_merge.contains("--delete-branch"));
}

#[test]
fn back_merge_creates_or_reuses_a_pr_and_keeps_conflicts_open() {
    let release = text_file(".github/workflows/release.yml");
    let script = job(&release, "back-merge")
        .split_once("        run: |\n")
        .unwrap()
        .1
        .lines()
        .map(|line| line.strip_prefix("          ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    for existing in ["", "42"] {
        for conflict in [false, true] {
            let fixture = Fixture::new();
            executable(
                &fixture.dir.join("bin/gh"),
                r#"#!/bin/sh
set -eu
printf '%s|' "$@" >> "$GH_LOG"
printf '\n' >> "$GH_LOG"
case "$*" in
  'pr list '*) printf '%s\n' "$EXISTING_PR" ;;
  'pr create '*) printf '%s\n' 'https://github.com/fixture/repo/pull/43' ;;
  'pr merge '*) exit "$MERGE_EXIT" ;;
  *) exit 99 ;;
esac
"#,
            );
            let log = fixture.dir.join("gh.log");
            let output = Command::new("bash")
                .args(["-euo", "pipefail", "-c", &script])
                .current_dir(&fixture.dir)
                .env(
                    "PATH",
                    format!(
                        "{}:{}",
                        fixture.dir.join("bin").display(),
                        std::env::var("PATH").unwrap()
                    ),
                )
                .env("GH_LOG", &log)
                .env("EXISTING_PR", existing)
                .env("MERGE_EXIT", if conflict { "1" } else { "0" })
                .env("TAG", "v1.2.3")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            let calls = fs::read_to_string(log).unwrap();
            let mut expected = String::from(
                "pr|list|--base|dev|--head|main|--state|open|--json|number|--jq|.[0].number // empty|\n",
            );
            let pr = if existing.is_empty() {
                expected.push_str("pr|create|--base|dev|--head|main|--title|chore: back-merge v1.2.3|--body|Return release v1.2.3 from main to dev.|\n");
                "https://github.com/fixture/repo/pull/43"
            } else {
                existing
            };
            expected.push_str(&format!("pr|merge|{pr}|--merge|\n"));
            // No close/delete call follows a failed merge, and the shell still succeeds.
            assert_eq!(calls, expected);
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert_eq!(stdout.contains("::warning::Back-merge PR"), conflict);
        }
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
        "cargo build --release --locked -p tilrd -p tilr --target",
        "scripts/release.sh \"$PLATFORM\"",
        "scripts/package.sh \"$TAG\" \"$TARGET\" \"$BIN_DIR\" dist/github",
        "name: tilr-${{ matrix.target }}",
        "name: cdn-${{ matrix.platform }}",
        "if: matrix.platform != ''",
    ] {
        assert!(build.contains(clause), "{clause}");
    }
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
    assert!(upload.contains("needs: [release-please, build]"));
    assert!(upload.contains("gh release upload"));
}

#[test]
fn a_release_builds_and_packages_every_target() {
    let release = text_file(".github/workflows/release.yml");
    let package = text_file("scripts/package.sh");
    assert_release_contract(&release, &package);
    for clause in [
        "if: matrix.native",
        "${TAG#v}-$(git rev-parse --short HEAD)",
    ] {
        let removed = release.replace(clause, "");
        assert_ne!(removed, release);
        assert!(std::panic::catch_unwind(|| assert_release_contract(&removed, &package)).is_err());
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
        "needs: [release-please, upload]",
        "environment: cdn",
        "vars.RELEASE_LINUX_ARM64 == 'true' && 'linux-aarch64 linux-x86_64' || 'linux-x86_64'",
        "pattern: cdn-*",
        "merge-multiple: true",
        "for platform in $PLATFORMS",
        "want=\"${TAG#v}-$(git rev-parse --short HEAD)\"",
        "test \"$(cat \"dist/cdn/releases/$platform/latest\")\" = \"$want\"",
        "AWS_ACCESS_KEY_ID: ${{ secrets.AWS_ACCESS_KEY_ID }}",
        "AWS_SECRET_ACCESS_KEY: ${{ secrets.AWS_SECRET_ACCESS_KEY }}",
        "run: aws --version",
        "CLOUDFLARE_ACCOUNT_ID:",
        "scripts/release.sh --upload-only $PLATFORMS",
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
    let config = active(&text_file(".cargo/config.toml"));
    assert!(config.contains("[env]"));
    assert!(config.contains(
        "DEVELOPER_DIR = { value = \"/Library/Developer/CommandLineTools\", force = false }"
    ));
    assert!(config.contains("SDKROOT = { value = \"/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk\", force = false }"));
    // Docker mounts the source but not the host worktree's Git metadata.
    let git_available = Command::new("git")
        .current_dir(root())
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .unwrap()
        .status
        .success();
    if git_available {
        let tracked = Command::new("git")
            .current_dir(root())
            .args(["ls-files", "--error-unmatch", ".cargo/config.toml"])
            .output()
            .unwrap();
        assert!(
            tracked.status.success(),
            "Cargo SDK defaults must be tracked"
        );
    } else {
        assert!(
            text_file(".gitignore")
                .lines()
                .any(|line| line == "!/.cargo/config.toml")
        );
    }

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
        assert!(section.contains("SDKROOT=/Applications/Xcode_16.3.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX15.4.sdk"));
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
