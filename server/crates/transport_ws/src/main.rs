use std::env;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use runtime::{
    run_data_plane, TransportAcceptor, TransportPacket, TransportReceiver, TransportReliability,
    TransportSender, TransportSocket,
};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{accept_async, WebSocketStream};

struct WsTransportAcceptor {
    listener: TcpListener,
}

impl WsTransportAcceptor {
    async fn bind(bind: &str) -> Result<Self, String> {
        let listener = TcpListener::bind(bind)
            .await
            .map_err(|error| format!("failed to bind websocket server on {bind}: {error}"))?;
        Ok(Self { listener })
    }
}

fn is_probe_handshake_error(error: &str) -> bool {
    error.contains("Handshake not finished") || error.contains("No \"Connection: upgrade\" header")
}

fn is_benign_send_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("sending after closing is not allowed")
        || normalized.contains("already closed")
        || normalized.contains("connection closed")
}

struct WsTransportSender {
    sink: SplitSink<WebSocketStream<TcpStream>, WsMessage>,
}

struct WsTransportReceiver {
    source: SplitStream<WebSocketStream<TcpStream>>,
}

#[async_trait]
impl TransportSender for WsTransportSender {
    async fn send(&mut self, packet: TransportPacket) -> Result<(), String> {
        let send_result = match String::from_utf8(packet.bytes.clone()) {
            Ok(text) => self.sink.send(WsMessage::Text(text)).await,
            Err(_) => self.sink.send(WsMessage::Binary(packet.bytes)).await,
        };

        match send_result {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = error.to_string();
                if is_benign_send_error(&message) {
                    Ok(())
                } else {
                    Err(format!("send failed: {message}"))
                }
            }
        }
    }
}

#[async_trait]
impl TransportReceiver for WsTransportReceiver {
    async fn receive(&mut self) -> Result<Option<TransportPacket>, String> {
        loop {
            let Some(frame) = self.source.next().await else {
                return Ok(None);
            };
            let frame = frame.map_err(|error| format!("read failed: {error}"))?;

            match frame {
                WsMessage::Text(text) => {
                    return Ok(Some(TransportPacket::new(
                        text.into_bytes(),
                        TransportReliability::Reliable,
                        None,
                    )))
                }
                WsMessage::Binary(binary) => {
                    return Ok(Some(TransportPacket::new(
                        binary,
                        TransportReliability::Reliable,
                        None,
                    )))
                }
                WsMessage::Close(_) => return Ok(None),
                _ => continue,
            }
        }
    }
}

#[async_trait]
impl TransportAcceptor for WsTransportAcceptor {
    async fn accept(&self) -> Result<(TransportSocket, String), String> {
        let (stream, peer_addr) = self
            .listener
            .accept()
            .await
            .map_err(|error| format!("tcp accept failed: {error}"))?;

        let websocket = accept_async(stream).await.map_err(|error| {
            let message = format!("websocket handshake failed: {error}");
            if is_probe_handshake_error(&message) {
                format!("transient:{message}")
            } else {
                message
            }
        })?;
        let (sink, source) = websocket.split();

        Ok((
            TransportSocket {
                sender: Box::new(WsTransportSender { sink }),
                receiver: Box::new(WsTransportReceiver { source }),
            },
            peer_addr.to_string(),
        ))
    }
}

#[tokio::main]
async fn main() {
    let bind = env::var("NEXIS_BIND").unwrap_or_else(|_| "0.0.0.0:4000".to_owned());
    let acceptor = match WsTransportAcceptor::bind(&bind).await {
        Ok(acceptor) => Arc::new(acceptor),
        Err(error) => {
            eprintln!("{error}");
            return;
        }
    };
    println!("nexis data-plane listening on ws://{bind}");

    if let Err(error) = run_data_plane(acceptor).await {
        eprintln!("data-plane runtime exited: {error}");
    }
}
