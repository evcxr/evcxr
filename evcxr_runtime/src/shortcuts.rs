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

use mime::Mime;
use mime_type;
use std::fs;
use std::io;
use std::path::Path;

// This module provides a set of helpers / shortcut functions to display content

/// Display the content of a local file.
///
/// ```rust
/// extern crate evcxr_runtime;
/// extern crate mime;
/// use evcxr_runtime;
/// use mime;
///
/// evcxr_runtime::display_file("hello.html", mime::TEXT_HTML, true);
/// evcxr_runtime::display_file("hello.svg", mime::IMAGE_SVG, true);
/// evcxr_runtime::display_file("hello.png", mime::IMAGE_PNG, false);
/// ```
pub fn display_file<P: AsRef<Path>>(path: P, mime: Mime, as_text: bool) -> Result<(), io::Error> {
    let buffer = fs::read(path)?;
    let cmt = mime_type(mime.as_ref());
    if as_text {
        let text = String::from_utf8_lossy(&buffer);
        cmt.text(text);
    } else {
        cmt.bytes(&buffer);
    }
    Ok(())
}
