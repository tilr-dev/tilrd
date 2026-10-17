//! `tilr service install|uninstall`: start tilrd at boot, as a service of
//! the user's, only when asked. Without it nothing starts tilrd but `tilr`
//! itself.
//!
//! Linux, with a systemd user manager and lingering (the user's manager runs
//! from boot, not only while the user is logged in): a user unit,
//! `tilr.service` in `~/.config/systemd/user`, `Type=notify` (tilrd says
//! READY=1 once it listens) with `Restart=on-failure`, enabled. Lingering is
//! turned on if it is off and the system lets the user (`loginctl
//! enable-linger`); the unit notes that, so uninstall turns it off again.
//! Without systemd, or where lingering cannot be had: a `@reboot` line in
//! the user's crontab. macOS: a launchd agent,
//! `~/Library/LaunchAgents/sh.tilr.tilrd.plist`.
//!
//! The service runs tilrd from a login shell (`/bin/sh -lc`), so the panes
//! get the PATH and environment of `~/.profile`, as they would over SSH. A
//! tilrd already running (one `tilr` started) is left alone: the service
//! takes over from the next boot. Uninstall removes what install wrote and
//! nothing else; the running server goes on until `tilr kill-server`.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output, Stdio};

use anyhow::{Context, Result, bail};
use tilr_proto::local;

use crate::daemon;

/// The systemd unit, the same name the installer gives its unit.
pub const UNIT: &str = "tilr.service";
/// The launchd agent's label.
pub const LABEL: &str = "sh.tilr.tilrd";
/// First line of the unit and the plist, and the end of the crontab line:
/// what uninstall recognizes as its own.
const MARK: &str = "tilr service";
const HEADER: &str = "# Written by `tilr service install`; `tilr service uninstall` removes it.";
/// In the unit when install turned lingering on.
const LINGER_NOTE: &str = "# tilr service: lingering turned on by install";

pub fn install(socket: &Path) -> Result<ExitCode> {
    let tilrd = daemon::find_tilrd()?;
    let tilrd = fs::canonicalize(&tilrd).with_context(|| format!("resolve {}", tilrd.display()))?;
    check_quotable(&tilrd)?;
    check_quotable(socket)?;
    let running = daemon::running(socket);
    if cfg!(target_os = "macos") {
        return install_launchd(&tilrd, socket, running);
    }
    if systemd_user() {
        match linger()? {
            Linger::On => return install_systemd(&tilrd, false, running),
            Linger::TurnedOn => return install_systemd(&tilrd, true, running),
            Linger::Unavailable(why) => {
                say(&format!(
                    "lingering is not available ({why}): a systemd user service would run only while you are logged in"
                ));
            }
        }
    } else {
        say("no systemd user manager here");
    }
    install_cron(&tilrd, socket, running)
}

pub fn uninstall() -> Result<ExitCode> {
    let mut removed = false;
    if cfg!(target_os = "macos") {
        removed |= uninstall_launchd()?;
    } else {
        removed |= uninstall_systemd()?;
        removed |= uninstall_cron()?;
    }
    if removed {
        say("a tilrd that runs now goes on until `tilr kill-server`");
    } else {
        say("no service was installed: nothing to remove");
    }
    Ok(ExitCode::SUCCESS)
}

// ------------------------------------------------------------------ systemd

/// Whether this user has a systemd manager to talk to.
fn systemd_user() -> bool {
    run("systemctl", &["--user", "show-environment"]).is_ok_and(|out| out.status.success())
}

enum Linger {
    On,
    TurnedOn,
    Unavailable(String),
}

fn linger() -> Result<Linger> {
    let user = user_name();
    match run(
        "loginctl",
        &["show-user", &user, "--property=Linger", "--value"],
    ) {
        Ok(out) if out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "yes" => {
            return Ok(Linger::On);
        }
        Ok(_) => {}
        Err(err) => return Ok(Linger::Unavailable(format!("loginctl: {err}"))),
    }
    match run("loginctl", &["enable-linger", &user]) {
        Ok(out) if out.status.success() => Ok(Linger::TurnedOn),
        Ok(out) => Ok(Linger::Unavailable(format!(
            "loginctl enable-linger {user}: {}",
            first_line(&out.stderr).unwrap_or("refused")
        ))),
        Err(err) => Ok(Linger::Unavailable(format!("loginctl: {err}"))),
    }
}

fn unit_path() -> PathBuf {
    config_home().join("systemd/user").join(UNIT)
}

/// The user unit that runs `tilrd`.
pub fn unit(tilrd: &Path, linger_turned_on: bool) -> String {
    let mut text = format!(
        "{HEADER}\n\
         [Unit]\n\
         Description=tilr terminal server (tilrd)\n\
         \n\
         [Service]\n\
         Type=notify\n\
         # A login shell: the panes get the environment of ~/.profile, as over SSH.\n\
         ExecStart=/bin/sh -lc 'exec \"{}\"'\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        tilrd.display()
    );
    if linger_turned_on {
        text.push_str(LINGER_NOTE);
        text.push('\n');
    }
    text
}

fn install_systemd(tilrd: &Path, linger_turned_on: bool, running: bool) -> Result<ExitCode> {
    let path = unit_path();
    // Lingering on from an earlier install of ours stays ours to turn off.
    let ours_before = fs::read_to_string(&path).is_ok_and(|text| text.contains(LINGER_NOTE));
    write_private(&path, &unit(tilrd, linger_turned_on || ours_before))?;
    systemctl(&["daemon-reload"])?;
    if running {
        systemctl(&["enable", UNIT])?;
    } else {
        systemctl(&["enable", "--now", UNIT])?;
    }
    say(&format!(
        "installed {} (a systemd user service, lingering {}): tilrd starts at boot and again if it fails",
        path.display(),
        if linger_turned_on { "turned on" } else { "on" }
    ));
    if running {
        say("a tilrd runs already (started by tilr); the service runs it from the next boot");
    }
    Ok(ExitCode::SUCCESS)
}

fn uninstall_systemd() -> Result<bool> {
    let path = unit_path();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    if !text.contains(MARK) {
        say(&format!(
            "{} was not written by `tilr service install`: left as it is",
            path.display()
        ));
        return Ok(false);
    }
    // Not stopped: that would end every shell.
    let _ = run("systemctl", &["--user", "disable", UNIT]);
    fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    let _ = run("systemctl", &["--user", "daemon-reload"]);
    say(&format!("removed {}", path.display()));
    if text.contains(LINGER_NOTE) {
        let user = user_name();
        match run("loginctl", &["disable-linger", &user]) {
            Ok(out) if out.status.success() => say("lingering turned off again"),
            _ => say(&format!(
                "could not turn lingering off: sudo loginctl disable-linger {user}"
            )),
        }
    }
    Ok(true)
}

fn systemctl(args: &[&str]) -> Result<()> {
    let mut full = vec!["--user"];
    full.extend_from_slice(args);
    let out = run("systemctl", &full)?;
    if !out.status.success() {
        bail!(
            "systemctl --user {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

// --------------------------------------------------------------------- cron

/// The crontab line: at boot, tilrd for `socket`, its log next to it.
pub fn cron_line(tilrd: &Path, socket: &Path) -> String {
    let dir = socket.parent().unwrap_or(Path::new("/"));
    format!(
        "@reboot /bin/sh -lc 'mkdir -p -m 700 \"{}\" && exec \"{}\" --socket \"{}\" >>\"{}\" 2>&1' # {MARK}",
        dir.display(),
        tilrd.display(),
        socket.display(),
        local::log_path(socket).display()
    )
}

fn install_cron(tilrd: &Path, socket: &Path, running: bool) -> Result<ExitCode> {
    let current = crontab_read()?;
    let mut lines: Vec<&str> = current.lines().filter(|line| !is_ours(line)).collect();
    let line = cron_line(tilrd, socket);
    lines.push(&line);
    crontab_write(&lines)?;
    say(&format!(
        "added to your crontab: {line}\ntilr: tilrd starts at boot (cron @reboot)"
    ));
    if std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|dir| socket.starts_with(dir)) {
        say(&format!(
            "{} is made at login and gone at boot: set TILR_RUNTIME_DIR to a directory that stays, and install again",
            socket.parent().unwrap_or(socket).display()
        ));
    }
    if !running {
        daemon::ensure_running(socket)?;
        say("tilrd started");
    }
    Ok(ExitCode::SUCCESS)
}

fn uninstall_cron() -> Result<bool> {
    let current = match crontab_read() {
        Ok(current) => current,
        // No crontab to look at: nothing of ours in it.
        Err(_) => return Ok(false),
    };
    if !current.lines().any(is_ours) {
        return Ok(false);
    }
    let lines: Vec<&str> = current.lines().filter(|line| !is_ours(line)).collect();
    if lines.iter().all(|line| line.trim().is_empty()) {
        // Install made the crontab: none again.
        let out = run("crontab", &["-r"])?;
        if !out.status.success() {
            bail!(
                "crontab -r: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    } else {
        crontab_write(&lines)?;
    }
    say("removed the @reboot line from your crontab");
    Ok(true)
}

fn is_ours(line: &str) -> bool {
    line.starts_with("@reboot ") && line.ends_with(&format!("# {MARK}"))
}

/// The user's crontab; empty if there is none.
fn crontab_read() -> Result<String> {
    let out = run("crontab", &["-l"]).map_err(|err| {
        anyhow::anyhow!(
            "no way to start tilrd at boot here: no systemd user service with lingering, and no crontab ({err})"
        )
    })?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("no crontab for") {
        return Ok(String::new());
    }
    bail!("crontab -l: {}", err.trim());
}

fn crontab_write(lines: &[&str]) -> Result<()> {
    let mut text = lines.join("\n");
    text.push('\n');
    let mut child = Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("run crontab")?;
    child
        .stdin
        .take()
        .context("crontab's stdin")?
        .write_all(text.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("crontab -: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

// ------------------------------------------------------------------ launchd

fn plist_path() -> PathBuf {
    home()
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

/// The launchd agent that runs tilrd for `socket` (the socket is named: an
/// agent's environment is not a terminal's).
pub fn plist(tilrd: &Path, socket: &Path, log: &Path) -> String {
    let command = format!(
        "exec \"{}\" --socket \"{}\"",
        tilrd.display(),
        socket.display()
    );
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <!-- Written by `tilr service install`; `tilr service uninstall` removes it. -->\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key>\n\t<string>{LABEL}</string>\n\
         \t<key>ProgramArguments</key>\n\
         \t<array>\n\t\t<string>/bin/sh</string>\n\t\t<string>-lc</string>\n\t\t<string>{}</string>\n\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         \t<key>KeepAlive</key>\n\t<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n\t</dict>\n\
         \t<key>StandardErrorPath</key>\n\t<string>{}</string>\n\
         \t<key>StandardOutPath</key>\n\t<string>{}</string>\n\
         </dict>\n\
         </plist>\n",
        xml_escape(&command),
        xml_escape(&log.display().to_string()),
        xml_escape(&log.display().to_string()),
    )
}

fn install_launchd(tilrd: &Path, socket: &Path, running: bool) -> Result<ExitCode> {
    let path = plist_path();
    let log = home().join("Library/Logs/tilrd.log");
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir)?;
    }
    write_private(&path, &plist(tilrd, socket, &log))?;
    let domain = format!("gui/{}", uid());
    if running {
        say(&format!(
            "installed {}: tilrd starts at login (launchd); one runs already (started by tilr), the agent runs it from the next login",
            path.display()
        ));
        return Ok(ExitCode::SUCCESS);
    }
    let _ = run("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
    let out = run(
        "launchctl",
        &["bootstrap", &domain, &path.to_string_lossy()],
    )?;
    if !out.status.success() {
        bail!(
            "launchctl bootstrap {domain} {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    say(&format!(
        "installed {}: tilrd starts at login and again if it fails (launchd); its log is {}",
        path.display(),
        log.display()
    ));
    Ok(ExitCode::SUCCESS)
}

fn uninstall_launchd() -> Result<bool> {
    let path = plist_path();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    if !text.contains(MARK) {
        say(&format!(
            "{} was not written by `tilr service install`: left as it is",
            path.display()
        ));
        return Ok(false);
    }
    // Unloaded with the file removed: launchd stops what it runs, unless
    // the job is told to leave it; tilrd ends its shells. So only the file
    // goes, and launchd forgets the agent at the next login.
    fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    say(&format!("removed {}", path.display()));
    Ok(true)
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ------------------------------------------------------------------ helpers

fn say(text: &str) {
    eprintln!("tilr: {text}");
}

fn run(program: &str, args: &[&str]) -> std::io::Result<Output> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
}

fn first_line(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes)
        .ok()?
        .lines()
        .find(|line| !line.trim().is_empty())
}

/// A path that goes between double quotes in a shell command inside single
/// quotes.
fn check_quotable(path: &Path) -> Result<()> {
    let text = path.to_str().context("a path that is not UTF-8")?;
    if let Some(c) = text.chars().find(|c| "'\"$`\\\n%".contains(*c)) {
        bail!(
            "{} has {c:?} in it, which a service's command line cannot carry",
            path.display()
        );
    }
    Ok(())
}

/// Written whole (a temp file renamed), readable by the user only, with its
/// directory made.
fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path.parent().context("a path with no directory")?;
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let temp = path.with_extension("tilr-new");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(&temp)
        .with_context(|| format!("write {}", temp.display()))?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temp, path).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

fn config_home() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home().join(".config"),
    }
}

fn uid() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

/// The user's login name, as loginctl takes it.
fn user_name() -> String {
    // SAFETY: getpwuid returns a pointer to static storage or null; the
    // name is copied out at once.
    unsafe {
        let pw = libc::getpwuid(libc::geteuid());
        if !pw.is_null() && !(*pw).pw_name.is_null() {
            return std::ffi::CStr::from_ptr((*pw).pw_name)
                .to_string_lossy()
                .into_owned();
        }
    }
    std::env::var("USER").unwrap_or_else(|_| uid().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unit_runs_tilrd_from_a_login_shell_and_tells_systemd_when_ready() {
        let text = unit(Path::new("/home/u/.local/bin/tilrd"), false);
        assert!(text.starts_with(HEADER));
        assert!(text.contains("\nType=notify\n"));
        assert!(text.contains("\nExecStart=/bin/sh -lc 'exec \"/home/u/.local/bin/tilrd\"'\n"));
        assert!(text.contains("\nRestart=on-failure\n"));
        assert!(text.contains("\nWantedBy=default.target\n"));
        assert!(!text.contains(LINGER_NOTE));
        assert!(unit(Path::new("/x/tilrd"), true).contains(LINGER_NOTE));
    }

    #[test]
    fn the_cron_line_is_recognized_as_ours_and_others_are_not() {
        let line = cron_line(
            Path::new("/h/.local/bin/tilrd"),
            Path::new("/tmp/tilr-1000/tilrd.sock"),
        );
        assert_eq!(
            line,
            "@reboot /bin/sh -lc 'mkdir -p -m 700 \"/tmp/tilr-1000\" && exec \"/h/.local/bin/tilrd\" \
             --socket \"/tmp/tilr-1000/tilrd.sock\" >>\"/tmp/tilr-1000/tilrd.log\" 2>&1' # tilr service"
        );
        assert!(is_ours(&line));
        assert!(!is_ours("@reboot /usr/bin/backup # mine"));
        assert!(!is_ours("# tilr service"));
    }

    #[test]
    fn the_plist_names_the_socket_and_keeps_tilrd_alive_on_failure() {
        let text = plist(
            Path::new("/Users/u/.local/bin/tilrd"),
            Path::new("/tmp/s & t/tilrd.sock"),
            Path::new("/Users/u/Library/Logs/tilrd.log"),
        );
        assert!(text.contains(MARK));
        assert!(text.contains("<string>sh.tilr.tilrd</string>"));
        assert!(text.contains(
            "<string>exec \"/Users/u/.local/bin/tilrd\" --socket \"/tmp/s &amp; t/tilrd.sock\"</string>"
        ));
        assert!(text.contains("<key>SuccessfulExit</key>\n\t\t<false/>"));
    }

    #[test]
    fn paths_a_command_line_cannot_carry_are_refused() {
        assert!(check_quotable(Path::new("/home/u/bin/tilrd")).is_ok());
        assert!(check_quotable(Path::new("/home/o'neil/tilrd")).is_err());
        assert!(check_quotable(Path::new("/home/$x/tilrd")).is_err());
    }
}
