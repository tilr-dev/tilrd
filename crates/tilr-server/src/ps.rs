//! `tilr-server ps`: the panes of a running server, one per line.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use tilr_proto::{
    ClientMessage, Hello, ListPanes, PROTOCOL_VERSION, PaneInfo, ServerMessage,
    client_message::Msg, server_message,
};
use tokio_tungstenite::tungstenite::Message;

const TIMEOUT: Duration = Duration::from_secs(5);

pub fn run(url: &str) -> Result<()> {
    let panes = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            tokio::time::timeout(TIMEOUT, list_panes(url))
                .await
                .with_context(|| format!("{url} did not answer"))?
        })?;
    print!("{}", table(&panes));
    Ok(())
}

async fn list_panes(url: &str) -> Result<Vec<PaneInfo>> {
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .with_context(|| format!("connect to {url}"))?;
    for msg in [
        Msg::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "tilr-server ps".into(),
            features: Vec::new(),
            // The server on this machine is this user's: its token is at hand.
            token: crate::auth::stored_token().unwrap_or_default(),
        }),
        Msg::ListPanes(ListPanes { request_id: 1 }),
    ] {
        let packet = tilr_proto::encode(&ClientMessage::from(msg));
        ws.send(Message::Binary(packet)).await?;
    }
    while let Some(message) = ws.next().await {
        let Message::Binary(packet) = message? else {
            continue;
        };
        match tilr_proto::decode::<ServerMessage>(&packet)?.msg {
            Some(server_message::Msg::PaneList(list)) if list.request_id == 1 => {
                return Ok(list.panes);
            }
            Some(server_message::Msg::Error(error)) => bail!("{url}: {}", error.message),
            _ => {}
        }
    }
    bail!("{url} closed the connection")
}

/// One line per pane, oldest first. The pid is that of the pane process, as
/// `ps` shows it; the shell is its child.
fn table(panes: &[PaneInfo]) -> String {
    let mut out = format!(
        "{:<36}  {:>7}  {:>7}  {:>7}  {}\n",
        "PANE", "PID", "SIZE", "VIEWERS", "CWD  COMMAND"
    );
    for pane in panes {
        let size = format!("{}x{}", pane.cols, pane.rows);
        let command = match pane.command.as_slice() {
            [] => "(shell)".to_owned(),
            command => command.join(" "),
        };
        out.push_str(&format!(
            "{:<36}  {:>7}  {:>7}  {:>7}  {}  {}\n",
            pane.pane_id, pane.pid, size, pane.viewers, pane.cwd, command
        ));
    }
    out
}
