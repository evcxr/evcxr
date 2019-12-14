// Copyright 2018 Google Inc.
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

// We currently use the json crate. Could probably rewrite to use serde-json. At
// the time this was originally written we couldn't due to
// https://github.com/rust-lang/rust/issues/45601 - but that's now long fixed
// and we've dropped support for old version for rustc prior to the fix.

use failure::Error;
use json;
use std::fs;

#[derive(Debug, Clone)]
pub(crate) struct Control {
    pub(crate) control_port: u16,
    pub(crate) shell_port: u16,
    pub(crate) stdin_port: u16,
    pub(crate) hb_port: u16,
    pub(crate) iopub_port: u16,
    pub(crate) transport: String,
    pub(crate) signature_scheme: String,
    pub(crate) ip: String,
    pub(crate) key: String,
}

macro_rules! parse_to_var {
    ($control_json:expr, $name:ident, $convert:ident) => {
        let $name = $control_json[stringify!($name)]
            .$convert()
            .ok_or_else(|| format_err!("Missing JSON field {}", stringify!($name)))?;
    };
}

impl Control {
    pub(crate) fn parse_file(file_name: &str) -> Result<Control, Error> {
        let control_file_contents = fs::read_to_string(file_name)?;
        let control_json = json::parse(&control_file_contents)?;
        parse_to_var!(control_json, control_port, as_u16);
        parse_to_var!(control_json, shell_port, as_u16);
        parse_to_var!(control_json, stdin_port, as_u16);
        parse_to_var!(control_json, hb_port, as_u16);
        parse_to_var!(control_json, iopub_port, as_u16);
        parse_to_var!(control_json, transport, as_str);
        parse_to_var!(control_json, signature_scheme, as_str);
        parse_to_var!(control_json, ip, as_str);
        parse_to_var!(control_json, key, as_str);
        Ok(Control {
            control_port,
            shell_port,
            stdin_port,
            hb_port,
            iopub_port,
            transport: transport.to_owned(),
            signature_scheme: signature_scheme.to_owned(),
            key: key.to_owned(),
            ip: ip.to_owned(),
        })
    }
}
