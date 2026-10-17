//! `tilrd ps`: the panes of a running server, one per line.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use tilr_proto::{
    ClientMessage, Hello, ListPanes, PROTOCOL_VERSION, PaneInfo, ServerMessage,
    client_message::Msg, server_message,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

const TIMEOUT: Duration = Duration::from_secs(5);

/// `url`: the server's TCP address; `None`: its Unix socket.
pub fn run(url: Option<&str>, server_version: bool) -> Result<()> {
    let socket = tilr_proto::local::socket_path();
    let what = url.map_or_else(|| socket.display().to_string(), str::to_owned);
    let (panes, version) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let listed = async {
                match url {
                    Some(url) => {
                        let (ws, _) = tokio_tungstenite::connect_async(url)
                            .await
                            .with_context(|| format!("connect to {url}"))?;
                        list_panes(ws, &what).await
                    }
                    None => {
                        let stream = tokio::net::UnixStream::connect(&socket)
                            .await
                            .with_context(|| format!("connect to {what}"))?;
                        let (ws, _) =
                            tokio_tungstenite::client_async(tilr_proto::local::URL, stream)
                                .await
                                .with_context(|| format!("connect to {what}"))?;
                        list_panes(ws, &what).await
                    }
                }
            };
            tokio::time::timeout(TIMEOUT, listed)
                .await
                .with_context(|| format!("{what} did not answer"))?
        })?;
    if server_version {
        println!("{version}");
    } else {
        print!("{}", table(&panes));
    }
    Ok(())
}

async fn list_panes<S>(mut ws: WebSocketStream<S>, url: &str) -> Result<(Vec<PaneInfo>, String)>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    for msg in [
        Msg::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "tilrd ps".into(),
            features: Vec::new(),
            // The server on this machine is this user's: its token is at hand
            // (and not asked for on the Unix socket).
            token: crate::auth::stored_token().unwrap_or_default(),
            // A one-off look: no client instance to come back as.
            client_id: String::new(),
        }),
        Msg::ListPanes(ListPanes { request_id: 1 }),
    ] {
        let packet = tilr_proto::encode(&ClientMessage::from(msg));
        ws.send(Message::Binary(packet)).await?;
    }
    let mut version = None;
    while let Some(message) = ws.next().await {
        let Message::Binary(packet) = message? else {
            continue;
        };
        match tilr_proto::decode::<ServerMessage>(&packet)?.msg {
            Some(server_message::Msg::HelloAck(hello)) => version = Some(hello.server_version),
            Some(server_message::Msg::PaneList(list)) if list.request_id == 1 => {
                return Ok((
                    list.panes,
                    version.context("server sent panes before HelloAck")?,
                ));
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
