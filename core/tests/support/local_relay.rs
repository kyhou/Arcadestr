// Real loopback WebSocket relay for transport/outcome regressions. No public relays.
use futures_util::{SinkExt, StreamExt};
use nostr_sdk::Event;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[derive(Clone)]
pub enum Reply {
    Events(Vec<Event>),
    EventsThenSilent(Vec<Event>),
    Disconnect,
    Closed,
    Silent,
}

pub struct LocalRelay {
    pub url: String,
    task: JoinHandle<()>,
}

impl Drop for LocalRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl LocalRelay {
    pub async fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local relay");
        let url = format!("ws://{}", listener.local_addr().expect("relay address"));
        let task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept client");
            let mut ws = accept_async(socket).await.expect("WebSocket handshake");
            while let Some(Ok(message)) = ws.next().await {
                let Message::Text(text) = message else {
                    continue;
                };
                let request: Value = serde_json::from_str(&text).expect("client JSON");
                if request[0] != "REQ" {
                    continue;
                }
                let id = request[1].clone();
                let messages = match &reply {
                    Reply::Events(events) => {
                        let mut messages = events
                            .iter()
                            .map(|event| json!(["EVENT", id, event]))
                            .collect::<Vec<_>>();
                        messages.push(json!(["EOSE", id]));
                        messages
                    }
                    Reply::Closed => {
                        vec![json!(["CLOSED", id, "error: test relay rejected query"])]
                    }
                    Reply::EventsThenSilent(events) => events
                        .iter()
                        .map(|event| json!(["EVENT", id, event]))
                        .collect(),
                    Reply::Disconnect => return,
                    Reply::Silent => Vec::new(),
                };
                for response in messages {
                    if ws
                        .send(Message::Text(response.to_string().into()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        });
        Self { url, task }
    }
}
