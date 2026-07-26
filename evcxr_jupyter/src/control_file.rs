// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use anyhow::Result;
use anyhow::anyhow;
use std::fs;

#[derive(Debug, Clone)]
pub(crate) struct Control {
    pub(crate) control_port: u16,
    pub(crate) shell_port: u16,
    pub(crate) stdin_port: u16,
    pub(crate) hb_port: u16,
    pub(crate) iopub_port: u16,
    pub(crate) transport: String,
    pub(crate) ip: String,
    pub(crate) key: String,
}

macro_rules! parse_to_var {
    ($control_json:expr, $name:ident, as_u16) => {
        let $name = $control_json[stringify!($name)]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| anyhow!("Missing or invalid JSON field {}", stringify!($name)))?;
    };
    ($control_json:expr, $name:ident, $convert:ident) => {
        let $name = $control_json[stringify!($name)]
            .$convert()
            .ok_or_else(|| anyhow!("Missing JSON field {}", stringify!($name)))?;
    };
}

impl Control {
    pub(crate) fn parse_file(file_name: &str) -> Result<Control> {
        let control_file_contents = fs::read_to_string(file_name)?;
        let control_json: serde_json::Value = serde_json::from_str(&control_file_contents)?;
        parse_to_var!(control_json, control_port, as_u16);
        parse_to_var!(control_json, shell_port, as_u16);
        parse_to_var!(control_json, stdin_port, as_u16);
        parse_to_var!(control_json, hb_port, as_u16);
        parse_to_var!(control_json, iopub_port, as_u16);
        parse_to_var!(control_json, transport, as_str);
        parse_to_var!(control_json, ip, as_str);
        parse_to_var!(control_json, key, as_str);
        Ok(Control {
            control_port,
            shell_port,
            stdin_port,
            hb_port,
            iopub_port,
            transport: transport.to_owned(),
            key: key.to_owned(),
            ip: ip.to_owned(),
        })
    }
}
