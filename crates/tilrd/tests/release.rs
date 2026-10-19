//! Use cases: release.sh builds a CDN layout consumed by tilrd update, and CI
//! keeps the release gates and packaging contract.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
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
        fs::remove_dir_all(&self.dir).unwrap();
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
        fs::copy(
            root().join("scripts/release.sh"),
            dir.join("scripts/release.sh"),
        )
        .unwrap();
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
            &dir.join("bin/wrangler"),
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$WRANGLER_LOG\"\n",
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
        command.env("WRANGLER_LOG", self.dir.join("wrangler.log"));
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
        fs::read_to_string(self.dir.join("wrangler.log")).unwrap_or_default()
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
    assert!(!f.dir.join("build.log").exists());
    assert!(f.log().is_empty());
}

#[test]
fn publishing_uploads_the_archive_and_its_checksum_before_latest() {
    let f = Fixture::new();
    assert_exit(&f.run(&["--publish", "linux-aarch64"], true), 0);
    let version = f.version();
    let lines: Vec<_> = f.log().lines().map(str::to_owned).collect();
    assert_eq!(lines.len(), 3, "{lines:?}");
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
            line.starts_with(&format!("r2 object put tilr-cdn/{name} --file ")),
            "{line}"
        );
        assert!(
            line.contains(&format!("--content-type {content_type}")),
            "{line}"
        );
        assert!(line.contains(&format!("--cache-control {cache}")), "{line}");
        assert!(line.ends_with("--remote"), "{line}");
    }
}

#[test]
fn upload_only_checks_what_was_built_and_leaves_other_platforms_alone() {
    let f = Fixture::new();
    assert_exit(&f.run(&["linux-aarch64", "linux-x86_64"], false), 0);
    assert_exit(&f.run(&["--upload-only", "linux-aarch64"], true), 0);
    assert!(!f.log().contains("releases/linux-x86_64/latest"));

    fs::remove_file(f.cdn().join("linux-aarch64/latest")).unwrap();
    assert_exit(&f.run(&["--upload-only", "linux-aarch64"], true), 1);
    fs::write(
        f.cdn().join("linux-aarch64/latest"),
        format!("{}\n", f.version()),
    )
    .unwrap();

    fs::write(f.cdn().join("linux-x86_64/latest"), "other-version\n").unwrap();
    assert_exit(
        &f.run(&["--upload-only", "linux-aarch64", "linux-x86_64"], true),
        1,
    );
    fs::write(
        f.cdn().join("linux-x86_64/latest"),
        format!("{}\n", f.version()),
    )
    .unwrap();

    fs::write(archive(&f, "linux-aarch64"), "corrupt").unwrap();
    assert_exit(&f.run(&["--upload-only", "linux-aarch64"], true), 1);
    assert_eq!(
        f.log().lines().count(),
        3,
        "failed validation uploaded files"
    );
}

fn serve_files(root: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().take(3) {
            let mut stream = stream.unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            let path = request.split_whitespace().nth(1).unwrap();
            let file = root.join(path.trim_start_matches('/'));
            let body = fs::read(file).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    url
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
    fs::copy(binary, &exe).unwrap();
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
    assert!(docker.contains("install-tools.sh /opt/tilr-tools zig"));
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
fn build_tools_are_pinned_by_checksum_and_used_everywhere() {
    let tools = active(&text_file("scripts/install-tools.sh"));
    assert!(tools.contains("ZIG_VERSION=0.15.2"));
    assert!(tools.contains("PROTOC_VERSION=36.2"));
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
        for key in ["zig_sha256=", "protoc_sha256="] {
            let hash = branch.split(key).nth(1).unwrap().lines().next().unwrap();
            assert_eq!(hash.len(), 64, "{platform} {key}");
            assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }
    assert!(tools.contains("fetch \"https://ziglang.org/download/"));
    assert!(tools.contains("\"$zig_sha256\""));
    assert!(tools.contains("\"$protoc_sha256\""));
    assert!(tools.contains("[ \"$got\" = \"$2\" ]"));
    for workflow in [".github/workflows/ci.yml", ".github/workflows/release.yml"] {
        assert!(
            active(&text_file(workflow)).contains("scripts/install-tools.sh"),
            "{workflow}"
        );
    }
    assert!(
        active(&text_file("scripts/linux.Dockerfile"))
            .contains("install-tools.sh /opt/tilr-tools zig")
    );
    let ci = active(&text_file(".github/workflows/ci.yml"));
    for gate in ["DENY_SHA256:", "GITLEAKS_SHA256:", "sha256sum -c -"] {
        assert!(ci.contains(gate), "{gate}");
    }
}

fn assert_ci_gates(ci: &str) {
    let ci = active(ci);
    let lint = job(&ci, "lint");
    let test = job(&ci, "test");
    let deny = job(&ci, "deny");
    let secrets = job(&ci, "secrets");
    for gate in [
        "- run: cargo fmt --all --check",
        "- run: cargo clippy --workspace --all-targets --locked -- -D warnings",
        "- run: cargo clippy --workspace --all-targets --features tilrd/failpoints --locked -- -D warnings",
    ] {
        assert!(lint.contains(gate), "missing CI gate: {gate}");
    }
    assert!(test.contains("- run: cargo test --workspace --features tilrd/failpoints --locked"));
    assert!(deny.contains("- run: '\"$CARGO_DENY\" --locked check'"));
    assert!(secrets.contains("- run: '\"$GITLEAKS\" git --redact --no-banner --exit-code 1 .'"));
    assert!(test.contains("os: [ubuntu-latest, macos-15]"));
}

#[test]
fn ci_runs_every_gate_of_agents_md() {
    let ci = text_file(".github/workflows/ci.yml");
    assert_ci_gates(&ci);
    let removed = ci.replace("      - run: cargo fmt --all --check\n", "");
    assert_ne!(removed, ci);
    assert!(std::panic::catch_unwind(|| assert_ci_gates(&removed)).is_err());
}

#[test]
fn ci_smoke_tests_both_executables() {
    let ci = active(&text_file(".github/workflows/ci.yml"));
    let test = job(&ci, "test");
    for clause in [
        "cargo build -p tilrd -p tilr --locked",
        "for exe in tilrd tilr",
        "--version",
        "--help",
        "git rev-parse --short HEAD",
        "cargo metadata --format-version 1 --no-deps --locked",
    ] {
        assert!(test.contains(clause), "missing smoke check: {clause}");
    }
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
    for name in ["tilr-proto", "tilr-screen", "tilrd", "tilr", "tilr-driver"] {
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
fn a_release_builds_and_packages_every_target() {
    let release = active(&text_file(".github/workflows/release.yml"));
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
    let upload = job(&release, "upload");
    assert!(upload.contains("needs: [release-please, build]"));
    assert!(upload.contains("gh release upload"));
}

#[test]
fn the_cdn_job_runs_after_the_assets_and_uploads_only_what_was_built() {
    let release = active(&text_file(".github/workflows/release.yml"));
    let cdn = job(&release, "cdn");
    for clause in [
        "needs: [release-please, upload]",
        "environment: cdn",
        "vars.RELEASE_LINUX_ARM64 == 'true' && 'linux-aarch64 linux-x86_64' || 'linux-x86_64'",
        "pattern: cdn-*",
        "merge-multiple: true",
        "for platform in $PLATFORMS",
        "dist/cdn/releases/$platform/latest",
        "CLOUDFLARE_API_TOKEN:",
        "CLOUDFLARE_ACCOUNT_ID:",
        "scripts/release.sh --upload-only $PLATFORMS",
    ] {
        assert!(cdn.contains(clause), "{clause}");
    }
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
            "name: Install Zig and protoc",
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
