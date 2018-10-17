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

pub trait Display {
    /// Implementation should emit a representation of itself in one or mime
    /// types  using the functions below.
    fn evcxr_display(&self);
}

/// Represents a mime type for some content that is yet to be emitted.
pub struct ContentMimeType {
    mime_type: String,
}

/// Prepares to output some content with the specified mime type.
/// ```
/// evcxr_runtime::mime_type("text/plain").text("Hello world");
/// ```
pub fn mime_type<S: Into<String>>(mime_type: S) -> ContentMimeType {
    ContentMimeType {
        mime_type: mime_type.into(),
    }
}

impl ContentMimeType {
    /// Emits the supplied content, which should be of the mime type already
    /// specified. If the type is a binary format (e.g. image/png), the content
    /// should have already been base64 encoded.
    /// ```
    /// evcxr_runtime::mime_type("text/html")
    ///     .text("<span style=\"color: red\">>Hello world</span>");
    /// ```
    pub fn text<S: AsRef<str>>(self, text: S) {
        println!(
            "EVCXR_BEGIN_CONTENT {}\n{}\nEVCXR_END_CONTENT",
            self.mime_type,
            text.as_ref()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::mime_type;

    #[test]
    fn test_emit_data() {
        mime_type("text/plain").text("Hello world");
    }

    #[test]
    fn test_mime_type_accept_string() {
        mime_type("text/plain".to_owned()).text("Hello world");
    }
}
