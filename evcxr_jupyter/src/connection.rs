// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use anyhow::Result;
use hmac::digest::KeyInit;
use hmac::Hmac;
use sha2::Sha256;

pub(crate) type HmacSha256 = Hmac<Sha256>;

pub(crate) struct Connection<S> {
    pub(crate) socket: S,
    /// Will be None if our key was empty (digest authentication disabled).
    pub(crate) mac: Option<HmacSha256>,
}

impl<S: zeromq::Socket> Connection<S> {
    pub(crate) fn new(socket: S, key: &str) -> Result<Self> {
        let mac = if key.is_empty() {
            None
        } else {
            Some(HmacSha256::new_from_slice(key.as_bytes()).expect("Shouldn't fail with HMAC"))
        };
        Ok(Connection { socket, mac })
    }
}
