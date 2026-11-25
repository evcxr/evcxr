// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::connection::Connection;
use crate::connection::HmacSha256;
use crate::connection::RecvError;
use anyhow::Result;
use anyhow::anyhow;
use anyhow::bail;
use bytes::Bytes;
use chrono::Utc;
use omq_tokio::Message;
use serde_json::Value;
use std::fmt;
use std::{self};
use uuid::Uuid;

struct RawMessage {
    zmq_identities: Vec<Bytes>,
    jparts: Vec<Bytes>,
}

impl RawMessage {
    pub(crate) async fn read(connection: &mut Connection) -> Result<RawMessage, RecvError> {
        Ok(Self::from_multipart(connection.recv().await?, connection)?)
    }

    pub(crate) fn from_multipart(
        multipart: Message,
        connection: &Connection,
    ) -> Result<RawMessage> {
        let delimiter_index = multipart
            .iter()
            .position(|part| &part[..] == DELIMITER)
            .ok_or_else(|| anyhow!("Missing delimeter"))?;
        let mut parts: Vec<_> = multipart.iter().collect();
        if parts.len() < delimiter_index + 2 {
            bail!("Missing HMAC frame");
        }
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
            if let Err(error) = mac.verify_slice(&hex::decode(&hmac)?) {
                bail!("{}", error);
            }
        }

        Ok(raw_message)
    }

    async fn send(self, connection: &mut Connection) -> Result<()> {
        use hmac::Mac;
        let hmac = if let Some(mac_template) = &connection.mac {
            let mut mac = mac_template.clone();
            self.digest(&mut mac);
            hex::encode(mac.finalize().into_bytes().as_slice())
        } else {
            String::new()
        };
        let parts = self
            .zmq_identities
            .into_iter()
            .chain([Bytes::from_static(DELIMITER), Bytes::from(hmac)])
            .chain(self.jparts);
        connection.send(Message::multipart(parts)).await?;
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
    header: Value,
    parent_header: Value,
    metadata: Value,
    content: Value,
}

const DELIMITER: &[u8] = b"<IDS|MSG>";

impl JupyterMessage {
    pub(crate) async fn read(connection: &mut Connection) -> Result<JupyterMessage, RecvError> {
        Ok(Self::from_raw_message(RawMessage::read(connection).await?)?)
    }

    fn from_raw_message(raw_message: RawMessage) -> Result<JupyterMessage> {
        fn message_to_json(message: &[u8]) -> Result<Value> {
            Ok(serde_json::from_slice(message)?)
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
        self.content["cursor_pos"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or_default()
    }

    pub(crate) fn target_name(&self) -> &str {
        self.content["target_name"].as_str().unwrap_or("")
    }

    pub(crate) fn data(&self) -> &Value {
        &self.content["data"]
    }

    pub(crate) fn comm_id(&self) -> &str {
        self.content["comm_id"].as_str().unwrap_or("")
    }

    // Creates a new child message of this message. ZMQ identities are not transferred.
    pub(crate) fn new_message(&self, msg_type: &str) -> JupyterMessage {
        let mut header = self.header.clone();
        header["msg_type"] = Value::String(msg_type.to_owned());
        header["username"] = Value::String("kernel".to_owned());
        header["msg_id"] = Value::String(Uuid::new_v4().to_string());
        header["date"] = Value::String(Utc::now().to_rfc3339());

        JupyterMessage {
            zmq_identities: Vec::new(),
            header,
            parent_header: self.header.clone(),
            metadata: object! {},
            content: object! {},
        }
    }

    // Creates a reply to this message. This is a child with the message type determined
    // automatically by replacing "request" with "reply". ZMQ identities are transferred.
    pub(crate) fn new_reply(&self) -> JupyterMessage {
        let mut reply = self.new_message(&self.message_type().replace("_request", "_reply"));
        reply.zmq_identities.clone_from(&self.zmq_identities);
        reply
    }

    #[must_use = "Need to send this message for it to have any effect"]
    pub(crate) fn comm_close_message(&self) -> JupyterMessage {
        self.new_message("comm_close").with_content(object! {
            "comm_id" => self.comm_id()
        })
    }

    pub(crate) fn get_content(&self) -> &Value {
        &self.content
    }

    pub(crate) fn with_content(mut self, content: Value) -> JupyterMessage {
        self.content = content;
        self
    }

    pub(crate) fn with_message_type(mut self, msg_type: &str) -> JupyterMessage {
        self.header["msg_type"] = Value::String(msg_type.to_owned());
        self
    }

    pub(crate) fn without_parent_header(mut self) -> JupyterMessage {
        self.parent_header = object! {};
        self
    }

    pub(crate) async fn send(&self, connection: &mut Connection) -> Result<()> {
        let raw_message = RawMessage {
            zmq_identities: self.zmq_identities.clone(),
            jparts: vec![
                serde_json::to_vec(&self.header)?.into(),
                serde_json::to_vec(&self.parent_header)?.into(),
                serde_json::to_vec(&self.metadata)?.into(),
                serde_json::to_vec(&self.content)?.into(),
            ],
        };
        raw_message.send(connection).await
    }
}

impl fmt::Debug for JupyterMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "\nHEADER {}",
            serde_json::to_string_pretty(&self.header).unwrap()
        )?;
        writeln!(
            f,
            "PARENT_HEADER {}",
            serde_json::to_string_pretty(&self.parent_header).unwrap()
        )?;
        writeln!(
            f,
            "METADATA {}",
            serde_json::to_string_pretty(&self.metadata).unwrap()
        )?;
        writeln!(
            f,
            "CONTENT {}\n",
            serde_json::to_string_pretty(&self.content).unwrap()
        )?;
        Ok(())
    }
}
