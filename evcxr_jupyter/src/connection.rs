// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use anyhow::Result;
use hmac::digest::KeyInit;
use hmac::Hmac;
use sha2::Sha256;

pub(crate) type HmacSha256 = Hmac<Sha256>;

pub(crate) struct Connection {
    pub(crate) socket: zmq::Socket,
    /// Will be None if our key was empty (digest authentication disabled).
    pub(crate) mac: Option<HmacSha256>,
}

impl Connection {
    pub(crate) fn new(socket: zmq::Socket, key: &str) -> Result<Connection> {
        let mac = if key.is_empty() {
            None
        } else {
            Some(HmacSha256::new_from_slice(key.as_bytes()).expect("Shouldn't fail with HMAC"))
        };
        Ok(Connection { socket, mac })
    }
}
