//! Use cases: launch profiles (`pane-profiles`): `profiles_dir`, file modes, names,
//! strict parsing, the request's override, `pane.profiles`. Real server, the owner
//! on the Unix socket; a pane's program writes what it saw into a FIFO the test
//! reads as the bytes come (`Sink`), so every wait is on an event.

mod common;

use std::os::unix::fs::PermissionsExt;

use common::Server;
use common::bus_panes::{raw, shell};
use common::bus_profiles::{
    ProfileServer, Sink, create, default_profiles_dir, list_panes, profiles, secure_dir,
    start_with, write_in, write_profile, write_to,
};
use common::bus_streams::within;
use prost::Message;
use tilr_bus::CallOptions;
use tilr_bus::pane::PaneInfo;

fn start() -> ProfileServer {
    start_with("")
}

fn as_strs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

#[tokio::test]
async fn a_pane_starts_with_the_profile_from_the_config_directory() {
    let server = start();
    write_profile(&server, "demo", "command = [\"/bin/sh\"]");
    let bus = server.owner().await;
    let mut created = bus.subscribe("pane.created").await.unwrap();
    let pane = create(&bus, "demo", &[], "").await.unwrap();
    assert_eq!(pane.profile, "demo");
    assert_eq!(pane.command, ["/bin/sh"]);
    assert!(!pane.sandboxed);
    // The event names the profile too.
    let event = within(created.next())
        .await
        .expect("the subscription ended");
    let info = PaneInfo::decode(event.payload).unwrap();
    assert_eq!(info.pane_id, pane.pane_id);
    assert_eq!(info.profile, "demo");
    assert!(!info.sandboxed);
    let listed = list_panes(&bus).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].profile, "demo");
    // A pane without a profile names none.
    let plain = create(&bus, "", &as_strs(&shell()), "").await.unwrap();
    assert_eq!(plain.profile, "");
    // tilrd provides `pane.profiles` among its methods.
    let methods: tilr_bus::bus::MethodsReply = bus
        .call_typed(
            "bus.methods",
            &tilr_bus::bus::MethodsRequest {},
            CallOptions::default(),
        )
        .await
        .unwrap();
    let info = methods
        .methods
        .iter()
        .find(|m| m.method == "pane.profiles")
        .expect("pane.profiles is listed");
    assert_eq!(info.providers, ["tilrd"]);
}

#[tokio::test]
async fn profiles_dir_names_another_directory() {
    let other = secure_dir();
    write_in(&other, "demo", "command = [\"/bin/sh\"]", 0o600);
    let server = start_with(&format!("profiles_dir = \"{}\"\n", other.display()));
    assert!(!default_profiles_dir(&server).exists());
    let bus = server.owner().await;
    let pane = create(&bus, "demo", &[], "").await.unwrap();
    assert_eq!(pane.profile, "demo");
    assert_eq!(pane.command, ["/bin/sh"]);
    let listed = profiles(&bus).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "demo");
}

#[tokio::test]
async fn a_profile_others_can_write_is_refused() {
    let server = start();
    let file = write_in(
        &default_profiles_dir(&server),
        "demo",
        "command = [\"/bin/sh\"]",
        0o666,
    );
    let bus = server.owner().await;
    let err = create(&bus, "demo", &[], "").await.unwrap_err();
    assert_eq!(err.code, "bad_request");
    assert!(
        err.message.starts_with("create pane: profile demo: "),
        "{err}"
    );
    assert!(err.message.contains(&file.display().to_string()), "{err}");
    assert!(err.message.contains("is writable by others"), "{err}");
    assert!(list_panes(&bus).await.is_empty());
    // pane.profiles gives the same reason.
    let listed = profiles(&bus).await;
    assert!(
        listed[0].error.contains("is writable by others"),
        "{listed:?}"
    );
}

#[tokio::test]
async fn a_profile_name_is_never_a_path() {
    let server = start();
    // A file the names would reach if they were paths.
    write_profile(&server, "demo", "command = [\"/bin/sh\"]");
    std::fs::write(server.config.join("server.toml"), "").unwrap();
    let bus = server.owner().await;
    for name in ["../server", "/etc/passwd", ".hidden", "a/b"] {
        let err = create(&bus, name, &[], "").await.unwrap_err();
        assert_eq!(err.code, "bad_request", "{name}");
        assert_eq!(
            err.message,
            format!("create pane: bad profile name: {name}")
        );
    }
    assert!(list_panes(&bus).await.is_empty());
}

#[tokio::test]
async fn an_unknown_profile_is_an_error() {
    let server = start();
    let bus = server.owner().await;
    let err = create(&bus, "nope", &[], "").await.unwrap_err();
    assert_eq!(err.code, "bad_request");
    assert_eq!(err.message, "create pane: no such profile: nope");
    // The connection stays open and a pane without a profile can be created next.
    let pane = create(&bus, "", &as_strs(&shell()), "").await.unwrap();
    assert_eq!(pane.profile, "");
}

#[tokio::test]
async fn a_misspelt_key_makes_the_profile_invalid() {
    let server = start();
    let file = write_profile(&server, "demo", "comand = [\"/bin/sh\"]");
    let bus = server.owner().await;
    let err = create(&bus, "demo", &[], "").await.unwrap_err();
    assert_eq!(err.code, "bad_request");
    assert!(
        err.message.starts_with("create pane: profile demo: "),
        "{err}"
    );
    assert!(err.message.contains(&file.display().to_string()), "{err}");
    assert!(err.message.contains("comand"), "{err}");
    assert!(list_panes(&bus).await.is_empty());
}

#[tokio::test]
async fn request_fields_override_the_profile() {
    let server = start();
    let work = Server::scratch_dir();
    let other = Server::scratch_dir();
    let script = "printf '%s|%s|%s' \"$GREETING\" \"$TERM\" \"$(pwd -P)\"";
    write_profile(
        &server,
        "demo",
        &format!(
            "command = [\"/bin/sh\", \"-c\", \"exit 3\"]\ncwd = \"{}\"\n[env]\nGREETING = \"hi\"\nTERM = \"dumb\"\n",
            work.display()
        ),
    );
    let bus = server.owner().await;
    // The request's command and an existing directory win; [env] stays; TERM is the
    // server's, not the profile's.
    let mut seen = Sink::new(server.dir.join("out"));
    let command = write_to(&seen, script);
    let pane = create(&bus, "demo", &as_strs(&command), other.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(pane.profile, "demo");
    assert_eq!(pane.command, command);
    seen.holds(format!(
        "hi|xterm-256color|{}",
        std::fs::canonicalize(&other).unwrap().display()
    ))
    .await;
    // A directory that does not exist: the profile's.
    let mut seen = Sink::new(server.dir.join("out2"));
    let command = write_to(&seen, script);
    let pane = create(&bus, "demo", &as_strs(&command), "/no/such/directory")
        .await
        .unwrap();
    assert_eq!(
        std::fs::canonicalize(&pane.cwd).unwrap(),
        std::fs::canonicalize(&work).unwrap()
    );
    seen.holds(format!(
        "hi|xterm-256color|{}",
        std::fs::canonicalize(&work).unwrap().display()
    ))
    .await;
}

#[tokio::test]
async fn an_edited_profile_applies_to_the_next_pane_only() {
    let server = start();
    let bus = server.owner().await;
    // The first pane prints $X, waits for a line, and prints $X again.
    let mut first = Sink::new(server.dir.join("first"));
    let mut second = Sink::new(server.dir.join("second"));
    write_profile(&server, "demo", "[env]\nX = \"1\"\n");
    let c = write_to(&first, "printf '%s' \"$X\"; read line; printf '%s' \"$X\"");
    let running = create(&bus, "demo", &as_strs(&c), "").await.unwrap();
    first.holds("1").await;
    write_profile(&server, "demo", "[env]\nX = \"2\"\n");
    let c = write_to(&second, "printf '%s' \"$X\"");
    create(&bus, "demo", &as_strs(&c), "").await.unwrap();
    second.holds("2").await;
    // The pane that ran before the edit still has what the profile said then.
    raw(&bus, &running.pane_id, "\r").await.unwrap();
    first.holds("11").await;
}

#[tokio::test]
async fn a_sandboxed_profile_is_refused_where_it_cannot_be_enforced() {
    let server = start();
    let marker = server.dir.join("marker");
    write_profile(
        &server,
        "boxed",
        &format!(
            "command = [\"/bin/sh\", \"-c\", \"touch '{}'\"]\n[sandbox]\nread_write = [\"~/w\"]\n",
            marker.display()
        ),
    );
    let bus = server.owner().await;
    let mut created = bus.subscribe("pane.created").await.unwrap();
    let err = create(&bus, "boxed", &[], "").await.unwrap_err();
    assert_eq!(err.code, "bad_request");
    assert_eq!(
        err.message,
        "create pane: profile boxed asks for a sandbox, which this server cannot enforce"
    );
    assert!(list_panes(&bus).await.is_empty());
    // The next pane is the first one created: the refused one never started.
    let plain = create(&bus, "", &as_strs(&shell()), "").await.unwrap();
    let event = within(created.next())
        .await
        .expect("the subscription ended");
    let info = PaneInfo::decode(event.payload).unwrap();
    assert_eq!(info.pane_id, plain.pane_id);
    assert!(!marker.exists());
}

#[tokio::test]
async fn profiles_are_listed_with_their_errors() {
    let server = start();
    let bus = server.owner().await;
    // No directory: no profiles.
    assert!(profiles(&bus).await.is_empty());
    let dir = default_profiles_dir(&server);
    write_in(
        &dir,
        "a",
        "description = \"the first\"\ncommand = [\"/bin/sh\"]",
        0o600,
    );
    write_in(&dir, "b", "[sandbox]\nread_write = [\"/tmp\"]", 0o600);
    write_in(&dir, "c", "this is not toml", 0o600);
    // Invalid for another reason, and with a [sandbox] table.
    write_in(
        &dir,
        "d",
        "comand = []\n[sandbox]\nread_write = [\"/tmp\"]",
        0o600,
    );
    write_in(&dir, ".x", "command = [\"/bin/sh\"]", 0o600);
    std::fs::write(dir.join("notes.txt"), "").unwrap();
    let listed = profiles(&bus).await;
    let names: Vec<&str> = listed.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["a", "b", "c", "d"]);
    assert_eq!(listed[0].description, "the first");
    assert!(!listed[0].sandboxed && listed[0].error.is_empty());
    assert!(listed[1].sandboxed && listed[1].error.is_empty());
    assert!(!listed[2].error.is_empty() && !listed[2].sandboxed);
    assert!(
        listed[3].error.contains("comand") && listed[3].sandboxed,
        "{listed:?}"
    );
    // Valid and invalid alike, in JSON too.
    let json = bus
        .call(
            "pane.profiles",
            "{}",
            CallOptions::default().content_type(tilr_bus::CONTENT_TYPE_JSON),
        )
        .await
        .unwrap();
    assert_eq!(json.content_type, tilr_bus::CONTENT_TYPE_JSON);
    let value: serde_json::Value = serde_json::from_slice(&json.payload).unwrap();
    assert_eq!(value["profiles"].as_array().unwrap().len(), 4);
    assert_eq!(value["profiles"][1]["sandboxed"], true);
}

/// `pane.create {profile: name}` on `server` is refused with a reason that is `reason`,
/// and `pane.profiles` gives the same; no pane starts.
async fn refused(server: &ProfileServer, name: &str, reason: &str) {
    let bus = server.owner().await;
    let err = within(create(&bus, name, &[], "")).await.unwrap_err();
    assert_eq!(err.code, "bad_request");
    assert_eq!(
        err.message,
        format!("create pane: profile {name}: {reason}")
    );
    assert!(list_panes(&bus).await.is_empty());
    let listed = within(profiles(&bus)).await;
    let row = listed.iter().find(|p| p.name == name).expect("listed");
    assert_eq!(row.error, reason);
}

#[tokio::test]
async fn a_symlinked_profile_in_a_group_writable_directory_is_refused() {
    let server = start();
    let elsewhere = secure_dir();
    let target = write_in(&elsewhere, "real", "command = [\"/bin/sh\"]", 0o600);
    let dir = default_profiles_dir(&server);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&target, dir.join("demo.toml")).unwrap();
    // The directory of the link's target counts, not the one the link is in.
    std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o770)).unwrap();
    let real = std::fs::canonicalize(&elsewhere).unwrap();
    refused(
        &server,
        "demo",
        &format!("{} is writable by others", real.display()),
    )
    .await;
}

#[tokio::test]
async fn a_group_writable_directory_above_the_profiles_is_refused() {
    let parent = secure_dir();
    let dir = parent.join("profiles");
    write_in(&dir, "demo", "command = [\"/bin/sh\"]", 0o600);
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o770)).unwrap();
    let server = start_with(&format!("profiles_dir = \"{}\"\n", dir.display()));
    let real = std::fs::canonicalize(&parent).unwrap();
    refused(
        &server,
        "demo",
        &format!("{} is writable by others", real.display()),
    )
    .await;
}

#[tokio::test]
async fn a_profile_larger_than_the_bound_is_refused() {
    let server = start();
    // A comment line of 64 KiB and one byte more.
    let text = format!("{}\n", "#".repeat(64 * 1024));
    let file = write_profile(&server, "big", &text);
    refused(
        &server,
        "big",
        &format!("{} is larger than 65536 bytes", file.display()),
    )
    .await;
}

#[tokio::test]
async fn a_fifo_in_place_of_a_profile_is_refused_without_hanging() {
    let server = start();
    let dir = default_profiles_dir(&server);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let fifo = dir.join("pipe.toml");
    let made = std::process::Command::new("mkfifo")
        .arg("-m")
        .arg("600")
        .arg(&fifo)
        .status()
        .expect("mkfifo");
    assert!(made.success());
    // Nobody writes into it: a blocking open would wait for a writer forever.
    refused(
        &server,
        "pipe",
        &format!("{} is not a regular file", fifo.display()),
    )
    .await;
}
