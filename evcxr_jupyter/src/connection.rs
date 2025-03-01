// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use anyhow::Result;
use hmac::Hmac;
use hmac::digest::KeyInit;
use sha2::Sha256;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;
use tokio_util::sync::CancellationToken;
use zeromq::ZmqMessage;
use zeromq::ZmqResult;

pub(crate) type HmacSha256 = Hmac<Sha256>;

pub(crate) enum RecvError {
    ShutdownRequested,
    Other(anyhow::Error),
}

/// Identifies a group of connections that need to shut down together.
#[derive(Clone)]
pub(crate) struct ConnectionGroup {
    token: CancellationToken,
    /// When all instances are dropped, the corresponding ConnectionShutdownRequester will unblock.
    _shutdown_complete: Sender<()>,
}

/// Used to request and wait for shutdown of the group.
pub(crate) struct ConnectionShutdownRequester {
    shutdown_complete: Receiver<()>,
}

pub(crate) struct Connection<S> {
    socket: Option<S>,
    /// Will be None if our key was empty (digest authentication disabled).
    pub(crate) mac: Option<HmacSha256>,
    group: Option<ConnectionGroup>,
}

impl ConnectionGroup {
    pub(crate) fn new() -> (ConnectionGroup, ConnectionShutdownRequester) {
        let (send, recv) = tokio::sync::mpsc::channel(1);
        (
            ConnectionGroup {
                token: CancellationToken::new(),
                _shutdown_complete: send,
            },
            ConnectionShutdownRequester {
                shutdown_complete: recv,
            },
        )
    }
}

impl<S> Connection<S> {
    /// Requests that all other connections in the same group shut down. Waits for them all to shut
    /// down. Note, the connection on which this is called isn't shut down, since it needs to send a
    /// response to the shutdown request before it does so.
    pub(crate) async fn shutdown_all_connections(
        &mut self,
        mut requester: ConnectionShutdownRequester,
    ) {
        let group = self
            .group
            .take()
            .expect("shutdown_all_connections called on connection without group");

        group.token.cancel();

        // Drop our group in order to pretend that this connection has shut down, otherwise the recv
        // below will block forever.
        drop(group);

        // Wait for all other connections to be dropped.
        let _ = requester.shutdown_complete.recv().await;
    }
}

impl<S: zeromq::Socket> Connection<S> {
    pub(crate) fn new(socket: S, key: &str, group: Option<ConnectionGroup>) -> Result<Self> {
        let mac = if key.is_empty() {
            None
        } else {
            Some(HmacSha256::new_from_slice(key.as_bytes()).expect("Shouldn't fail with HMAC"))
        };
        Ok(Connection {
            socket: Some(socket),
            mac,
            group,
        })
    }
}

impl<S: zeromq::Socket + zeromq::SocketRecv> Connection<S> {
    pub(crate) async fn recv(&mut self) -> Result<ZmqMessage, RecvError> {
        let Some(socket) = self.socket.as_mut() else {
            return Err(RecvError::ShutdownRequested);
        };
        let Some(group) = self.group.as_ref() else {
            // We're not in a group, so just do an ordinary read.
            return socket.recv().await.map_err(|e| RecvError::Other(e.into()));
        };
        tokio::select! {
            _ = group.token.cancelled() => {
                if let Some(socket) = self.socket.take() {
                    socket.close().await;
                }
                Err(RecvError::ShutdownRequested)
            }
            recv_result = socket.recv() => {
                recv_result.map_err(|e| RecvError::Other(e.into()))
            }
        }
    }
}

impl<S: zeromq::SocketSend> Connection<S> {
    pub(crate) async fn send(&mut self, message: ZmqMessage) -> ZmqResult<()> {
        let Some(socket) = self.socket.as_mut() else {
            // For now, we just silently ignore send requests on a socket after we've started
            // shutdown.
            return Ok(());
        };
        socket.send(message).await
    }
}

impl From<anyhow::Error> for RecvError {
    fn from(value: anyhow::Error) -> Self {
        RecvError::Other(value)
    }
}
