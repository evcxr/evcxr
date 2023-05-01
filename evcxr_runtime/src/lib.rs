// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

#[cfg(feature = "bytes")]
extern crate base64;

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

    /// Emits the supplied content, which should be of the mime type already
    /// specified. The content is a binary format (e.g. image/png), the content
    /// will be base64 encoded.
    /// ```
    /// let buffer: Vec<u8> = vec![];
    /// evcxr_runtime::mime_type("image/png").bytes(&buffer);
    /// ```
    #[cfg(feature = "bytes")]
    pub fn bytes(self, buffer: &[u8]) {
        self.text(base64::encode(buffer))
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
