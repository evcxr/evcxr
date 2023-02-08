// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::connection::Connection;
use crate::connection::HmacSha256;
use anyhow::anyhow;
use anyhow::bail;
use anyhow::Result;
use bytes::Bytes;
use chrono::Utc;
use generic_array::GenericArray;
use json::JsonValue;
use json::{self};
use std::fmt;
use std::{self};
use uuid::Uuid;

struct RawMessage {
    zmq_identities: Vec<Bytes>,
    jparts: Vec<Bytes>,
}

impl RawMessage {
    pub(crate) async fn read<S: zeromq::SocketRecv>(
        connection: &mut Connection<S>,
    ) -> Result<RawMessage> {
        Self::from_multipart(connection.socket.recv().await?, connection)
    }

    pub(crate) fn from_multipart<S>(
        multipart: zeromq::ZmqMessage,
        connection: &Connection<S>,
    ) -> Result<RawMessage> {
        let delimiter_index = multipart
            .iter()
            .position(|part| &part[..] == DELIMITER)
            .ok_or_else(|| anyhow!("Missing delimeter"))?;
        let mut parts = multipart.into_vec();
        let jparts: Vec<_> = parts.drain(delimiter_index + 2..).collect();
        let hmac = parts.pop().unwrap();
        // Remove delimiter, so that what's left is just the identities.
        parts.pop();
        let zmq_identities = parts;

        let raw_message = RawMessage {
            zmq_identities,
            jparts,
        };

        if let Some(mac_template) = &connection.mac {
            let mut mac = mac_template.clone();
            raw_message.digest(&mut mac);
            use hmac::Mac;
            if let Err(error) = mac.verify(GenericArray::from_slice(&hex::decode(&hmac)?)) {
                bail!("{}", error);
            }
        }

        Ok(raw_message)
    }

    async fn send<S: zeromq::SocketSend>(self, connection: &mut Connection<S>) -> Result<()> {
        use hmac::Mac;
        let hmac = if let Some(mac_template) = &connection.mac {
            let mut mac = mac_template.clone();
            self.digest(&mut mac);
            hex::encode(mac.finalize().into_bytes().as_slice())
        } else {
            String::new()
        };
        let mut parts: Vec<bytes::Bytes> = Vec::new();
        for part in &self.zmq_identities {
            parts.push(part.to_vec().into());
        }
        parts.push(DELIMITER.into());
        parts.push(hmac.as_bytes().to_vec().into());
        for part in &self.jparts {
            parts.push(part.to_vec().into());
        }
        // ZmqMessage::try_from only fails if parts is empty, which it never
        // will be here.
        let message = zeromq::ZmqMessage::try_from(parts).unwrap();
        connection.socket.send(message).await?;
        Ok(())
    }

    fn digest(&self, mac: &mut HmacSha256) {
        use hmac::Mac;
        for part in &self.jparts {
            mac.update(part);
        }
    }
}

#[derive(Clone)]
pub(crate) struct JupyterMessage {
    zmq_identities: Vec<Bytes>,
    header: JsonValue,
    parent_header: JsonValue,
    metadata: JsonValue,
    content: JsonValue,
}

const DELIMITER: &[u8] = b"<IDS|MSG>";

impl JupyterMessage {
    pub(crate) async fn read<S: zeromq::SocketRecv>(
        connection: &mut Connection<S>,
    ) -> Result<JupyterMessage> {
        Self::from_raw_message(RawMessage::read(connection).await?)
    }

    fn from_raw_message(raw_message: RawMessage) -> Result<JupyterMessage> {
        fn message_to_json(message: &[u8]) -> Result<JsonValue> {
            Ok(json::parse(std::str::from_utf8(message)?)?)
        }

        if raw_message.jparts.len() < 4 {
            bail!("Insufficient message parts {}", raw_message.jparts.len());
        }

        Ok(JupyterMessage {
            zmq_identities: raw_message.zmq_identities,
            header: message_to_json(&raw_message.jparts[0])?,
            parent_header: message_to_json(&raw_message.jparts[1])?,
            metadata: message_to_json(&raw_message.jparts[2])?,
            content: message_to_json(&raw_message.jparts[3])?,
        })
    }

    pub(crate) fn message_type(&self) -> &str {
        self.header["msg_type"].as_str().unwrap_or("")
    }

    pub(crate) fn code(&self) -> &str {
        self.content["code"].as_str().unwrap_or("")
    }

    pub(crate) fn cursor_pos(&self) -> usize {
        self.content["cursor_pos"].as_usize().unwrap_or_default()
    }

    pub(crate) fn target_name(&self) -> &str {
        self.content["target_name"].as_str().unwrap_or("")
    }

    pub(crate) fn data(&self) -> &JsonValue {
        &self.content["data"]
    }

    pub(crate) fn comm_id(&self) -> &str {
        self.content["comm_id"].as_str().unwrap_or("")
    }

    // Creates a new child message of this message. ZMQ identities are not transferred.
    pub(crate) fn new_message(&self, msg_type: &str) -> JupyterMessage {
        let mut header = self.header.clone();
        header["msg_type"] = JsonValue::String(msg_type.to_owned());
        header["username"] = JsonValue::String("kernel".to_owned());
        header["msg_id"] = JsonValue::String(Uuid::new_v4().to_string());
        header["date"] = JsonValue::String(Utc::now().to_rfc3339());

        JupyterMessage {
            zmq_identities: Vec::new(),
            header,
            parent_header: self.header.clone(),
            metadata: JsonValue::new_object(),
            content: JsonValue::new_object(),
        }
    }

    // Creates a reply to this message. This is a child with the message type determined
    // automatically by replacing "request" with "reply". ZMQ identities are transferred.
    pub(crate) fn new_reply(&self) -> JupyterMessage {
        let mut reply = self.new_message(&self.message_type().replace("_request", "_reply"));
        reply.zmq_identities = self.zmq_identities.clone();
        reply
    }

    #[must_use = "Need to send this message for it to have any effect"]
    pub(crate) fn comm_close_message(&self) -> JupyterMessage {
        self.new_message("comm_close").with_content(object! {
            "comm_id" => self.comm_id()
        })
    }

    pub(crate) fn get_content(&self) -> &JsonValue {
        &self.content
    }

    pub(crate) fn with_content(mut self, content: JsonValue) -> JupyterMessage {
        self.content = content;
        self
    }

    pub(crate) fn with_message_type(mut self, msg_type: &str) -> JupyterMessage {
        self.header["msg_type"] = JsonValue::String(msg_type.to_owned());
        self
    }

    pub(crate) fn without_parent_header(mut self) -> JupyterMessage {
        self.parent_header = object! {};
        self
    }

    pub(crate) async fn send<S: zeromq::SocketSend>(
        &self,
        connection: &mut Connection<S>,
    ) -> Result<()> {
        // If performance is a concern, we can probably avoid the clone and to_vec calls with a bit
        // of refactoring.
        let raw_message = RawMessage {
            zmq_identities: self.zmq_identities.clone(),
            jparts: vec![
                self.header.dump().as_bytes().to_vec().into(),
                self.parent_header.dump().as_bytes().to_vec().into(),
                self.metadata.dump().as_bytes().to_vec().into(),
                self.content.dump().as_bytes().to_vec().into(),
            ],
        };
        raw_message.send(connection).await
    }
}

impl fmt::Debug for JupyterMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "\nHEADER {}", self.header.pretty(2))?;
        writeln!(f, "PARENT_HEADER {}", self.parent_header.pretty(2))?;
        writeln!(f, "METADATA {}", self.metadata.pretty(2))?;
        writeln!(f, "CONTENT {}\n", self.content.pretty(2))?;
        Ok(())
    }
}
