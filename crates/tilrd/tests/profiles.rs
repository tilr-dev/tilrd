//! Use cases: launch profiles (`pane-profiles`): `profiles_dir`, file modes, names,
//! strict parsing, the request's override, `pane.profiles`. Real server, the owner
//! on the Unix socket; a pane's program writes what it saw to a file the test reads.
//!
//! A test that creates a pane is `#[ignore]`d until `bus-core-panes` provides
//! `pane.create`; its reason says so. Remove the attribute with that change's merge.

mod common;

use common::Server;
use common::bus::{BusOptions, BusServer};
use common::bus_profiles::{
    create, default_profiles_dir, list_panes, profiles, wait_for_file, write_in, write_profile,
    write_to,
};

fn start() -> BusServer {
    BusServer::start(BusOptions::default())
}

fn as_strs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

#[tokio::test]
#[ignore = "needs pane.create of bus-core-panes"]
async fn a_pane_starts_with_the_profile_from_the_config_directory() {
    let server = start();
    write_profile(&server, "demo", "command = [\"/bin/sh\"]");
    let bus = server.owner().await;
    let pane = create(&bus, "demo", &[], "").await.unwrap();
    assert_eq!(pane.profile, "demo");
    assert_eq!(pane.command, ["/bin/sh"]);
    assert!(!pane.sandboxed);
    assert_eq!(list_panes(&bus).await.len(), 1);
}

#[tokio::test]
#[ignore = "needs pane.create of bus-core-panes"]
async fn profiles_dir_names_another_directory() {
    let other = Server::scratch_dir();
    write_in(&other, "demo", "command = [\"/bin/sh\"]", 0o600);
    let server = BusServer::start(BusOptions {
        server_toml: Some(format!("profiles_dir = \"{}\"\n", other.display())),
        ..BusOptions::default()
    });
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
#[ignore = "needs pane.create of bus-core-panes"]
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
}

#[tokio::test]
#[ignore = "needs pane.create of bus-core-panes"]
async fn a_profile_name_is_never_a_path() {
    let server = start();
    // A file the names would reach if they were paths.
    write_profile(&server, "demo", "command = [\"/bin/sh\"]");
    std::fs::write(server.dir.join("config/server.toml"), "").unwrap();
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
#[ignore = "needs pane.create of bus-core-panes"]
async fn an_unknown_profile_is_an_error() {
    let server = start();
    let bus = server.owner().await;
    let err = create(&bus, "nope", &[], "").await.unwrap_err();
    assert_eq!(err.code, "bad_request");
    assert_eq!(err.message, "create pane: no such profile: nope");
    // The connection stays open and a pane without a profile can be created next.
    let pane = create(&bus, "", &[], "").await.unwrap();
    assert_eq!(pane.profile, "");
}

#[tokio::test]
#[ignore = "needs pane.create of bus-core-panes"]
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
#[ignore = "needs pane.create of bus-core-panes"]
async fn request_fields_override_the_profile() {
    let server = start();
    let work = Server::scratch_dir();
    let other = Server::scratch_dir();
    let out = server.dir.join("out");
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
    let command = write_to(&out, script);
    let pane = create(&bus, "demo", &as_strs(&command), other.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(pane.profile, "demo");
    assert_eq!(pane.command, command);
    let seen = wait_for_file(&out).await;
    assert_eq!(
        seen,
        format!(
            "hi|xterm-256color|{}",
            std::fs::canonicalize(&other).unwrap().display()
        )
    );
    // A directory that does not exist: the profile's.
    let out2 = server.dir.join("out2");
    let command = write_to(&out2, script);
    create(&bus, "demo", &as_strs(&command), "/no/such/directory")
        .await
        .unwrap();
    assert_eq!(
        wait_for_file(&out2).await,
        format!(
            "hi|xterm-256color|{}",
            std::fs::canonicalize(&work).unwrap().display()
        )
    );
}

#[tokio::test]
#[ignore = "needs pane.create of bus-core-panes"]
async fn an_edited_profile_applies_to_the_next_pane_only() {
    let server = start();
    let bus = server.owner().await;
    let script = "printf '%s' \"$X\"";
    let (first, second) = (server.dir.join("first"), server.dir.join("second"));
    let command = |out: &std::path::Path| write_to(out, script);
    write_profile(&server, "demo", "[env]\nX = \"1\"\n");
    let c = command(&first);
    create(&bus, "demo", &as_strs(&c), "").await.unwrap();
    assert_eq!(wait_for_file(&first).await, "1");
    write_profile(&server, "demo", "[env]\nX = \"2\"\n");
    let c = command(&second);
    create(&bus, "demo", &as_strs(&c), "").await.unwrap();
    assert_eq!(wait_for_file(&second).await, "2");
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "1");
}

#[tokio::test]
#[ignore = "needs pane.create of bus-core-panes"]
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
    let err = create(&bus, "boxed", &[], "").await.unwrap_err();
    assert_eq!(err.code, "bad_request");
    assert_eq!(
        err.message,
        "create pane: profile boxed asks for a sandbox, which this server cannot enforce"
    );
    assert!(list_panes(&bus).await.is_empty());
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
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
    write_in(&dir, ".x", "command = [\"/bin/sh\"]", 0o600);
    std::fs::write(dir.join("notes.txt"), "").unwrap();
    let listed = profiles(&bus).await;
    let names: Vec<&str> = listed.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["a", "b", "c"]);
    assert_eq!(listed[0].description, "the first");
    assert!(!listed[0].sandboxed && listed[0].error.is_empty());
    assert!(listed[1].sandboxed && listed[1].error.is_empty());
    assert!(!listed[2].error.is_empty());
    // Valid and invalid alike, in JSON too.
    let json = bus
        .call(
            "pane.profiles",
            "{}",
            tilr_bus::CallOptions::default().content_type(tilr_bus::CONTENT_TYPE_JSON),
        )
        .await
        .unwrap();
    assert_eq!(json.content_type, tilr_bus::CONTENT_TYPE_JSON);
    let value: serde_json::Value = serde_json::from_slice(&json.payload).unwrap();
    assert_eq!(value["profiles"].as_array().unwrap().len(), 3);
    assert_eq!(value["profiles"][1]["sandboxed"], true);
}
