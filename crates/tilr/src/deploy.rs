//! Install through the user's SSH, with one private multiplexed connection.
use std::fs::{self, File};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};

#[derive(clap::Args)]
pub struct Args {
    /// Start tilrd at boot as a service of the remote user.
    #[arg(long)]
    service: bool,
    /// A release archive, with its .sha256 next to it.
    #[arg(long)]
    archive: Option<PathBuf>,
    /// Where releases are fetched from.
    #[arg(long, env = "TILR_CDN")]
    cdn: Option<String>,
    /// SSH destination (user@host or a host from your SSH config).
    dest: String,
}

fn platform(uname: &str) -> Option<&'static str> {
    match uname.trim() {
        "Linux x86_64" => Some("linux-x86_64"),
        "Linux aarch64" => Some("linux-aarch64"),
        "Darwin arm64" => Some("macos-aarch64"),
        _ => None,
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

struct Connection {
    dir: PathBuf,
    dest: String,
}

impl Connection {
    fn command(&self) -> Command {
        let mut command = Command::new("ssh");
        command
            .args(["-o", "ControlMaster=auto", "-o", "ControlPersist=60", "-o"])
            .arg(format!("ControlPath={}/%C", self.dir.display()))
            .arg("--")
            .arg(&self.dest);
        command
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // Close our master before removing its private socket directory.
        let _ = Command::new("ssh")
            .arg("-o")
            .arg(format!("ControlPath={}/%C", self.dir.display()))
            .args(["-O", "exit", "--"])
            .arg(&self.dest)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

pub fn run(args: &Args) -> Result<()> {
    ensure!(
        !args.dest.is_empty()
            && !args.dest.starts_with('-')
            && !args.dest.chars().any(char::is_control),
        "invalid SSH destination"
    );
    // /tmp keeps ControlPath below Unix socket path limits, including on macOS.
    let dir = PathBuf::from(format!(
        "/tmp/tilr-deploy-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let connection = Connection {
        dir,
        dest: args.dest.clone(),
    };
    let output = connection
        .command()
        .arg("uname -sm")
        .stdin(Stdio::inherit())
        .output()
        .context("run ssh uname -sm")?;
    ensure!(
        output.status.success(),
        "ssh platform query failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let uname = String::from_utf8(output.stdout).context("read remote platform")?;
    let Some(platform) = platform(&uname) else {
        bail!(
            "{} runs {}, for which there is no release",
            args.dest,
            uname.trim()
        );
    };
    let tilrd = std::env::current_exe()?.with_file_name("tilrd");
    let out = connection.dir.join("release");
    let mut fetch = Command::new(tilrd);
    fetch
        .args(["fetch-release", "--platform", platform, "--out"])
        .arg(&out);
    if let Some(cdn) = &args.cdn {
        fetch.args(["--cdn", cdn]);
    }
    if let Some(archive) = &args.archive {
        fetch.arg("--archive").arg(archive);
    }
    let output = fetch.output().context("run tilrd fetch-release")?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    // Use the directory itself, not a path parsed from output (paths can contain newlines).
    let archive = fs::read_dir(&out)?
        .next()
        .context("fetch-release wrote no archive")??
        .path();
    let script = INSTALL.replace("@SERVICE@", if args.service { "1" } else { "" });
    let output = connection
        .command()
        .arg(format!("sh -c {}", quote(&script)))
        .stdin(Stdio::from(File::open(archive)?))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .context("install over ssh")?;
    ensure!(output.status.success(), "remote install failed");
    let token = String::from_utf8(output.stdout).context("read remote token")?;
    let token = token.trim();
    ensure!(!token.is_empty(), "remote tilr returned no token");
    let dest = if args
        .dest
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "@._-:[]".contains(c))
    {
        args.dest.clone()
    } else {
        quote(&args.dest)
    };
    println!("ssh -t {dest} ~/.local/bin/tilr");
    println!("ssh -N -L 7890:127.0.0.1:7890 {dest}");
    println!("{token}");
    Ok(())
}

const INSTALL: &str = r#"
set -eu
bin="$HOME/.local/bin"
made=
dir=$bin
while [ ! -e "$dir" ]; do made=$dir; dir=$(dirname "$dir"); done
tmp=
swapped=
committed=
cleanup() {
  result=$?
  trap - EXIT HUP INT TERM
  if [ -n "$swapped" ] && [ -z "$committed" ]; then
    for name in tilr tilrd; do
      if [ -e "$tmp/$name.old" ] || [ -L "$tmp/$name.old" ]; then mv -f "$tmp/$name.old" "$bin/$name"; else rm -f "$bin/$name"; fi
    done
  fi
  [ -z "$tmp" ] || rm -rf "$tmp"
  if [ -z "$committed" ] && [ -n "$made" ]; then rm -rf "$made"; fi
  exit "$result"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
mkdir -p "$bin"
tmp=$(mktemp -d "$bin/.tilr-deploy.XXXXXXXX")
cat > "$tmp/archive.tar.gz"
mkdir "$tmp/release"
for name in tilr tilrd; do
  # Both archive spellings accepted by the local validator are portable tar paths.
  tar -xzf "$tmp/archive.tar.gz" -C "$tmp/release" "$name" 2>/dev/null ||
    tar -xzf "$tmp/archive.tar.gz" -C "$tmp/release" "./$name"

  [ -f "$tmp/release/$name" ] && [ ! -L "$tmp/release/$name" ]
  chmod 755 "$tmp/release/$name"
  "$tmp/release/$name" --version >/dev/null
  [ ! -d "$bin/$name" ]
  if [ -e "$bin/$name" ] || [ -L "$bin/$name" ]; then cp -pP "$bin/$name" "$tmp/$name.old"; fi
done
version=$("$tmp/release/tilr" --version)
version=${version#* }
version=${version%% *}
swapped=1
mv -f "$tmp/release/tilr" "$bin/tilr"
mv -f "$tmp/release/tilrd" "$bin/tilrd"
if [ -n '@SERVICE@' ]; then "$bin/tilr" service install >&2; fi
token=$("$bin/tilr" token)
[ -n "$token" ]
committed=1
echo "installed tilr $version in ~/.local/bin" >&2
case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo '~/.local/bin is not on your PATH; add this line to ~/.profile and log in again:' >&2
     echo 'export PATH="$HOME/.local/bin:$PATH"' >&2 ;;
esac
printf '%s\n' "$token"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uname_names_the_platform() {
        assert_eq!(platform("Linux x86_64\n"), Some("linux-x86_64"));
        assert_eq!(platform("Linux aarch64"), Some("linux-aarch64"));
        assert_eq!(platform("Darwin arm64"), Some("macos-aarch64"));
        assert_eq!(platform("SunOS i86pc"), None);
    }
}
