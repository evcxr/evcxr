// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::errors::bail;
use crate::errors::Error;
use once_cell::sync::OnceCell;
use regex::Regex;
use std::path::Path;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct ExternalCrate {
    // The name, as it appears in the crate registry. This may be different than the key against
    // which this ExternalCrate is stored. If this name is "foo-bar", the key would be normalized as
    // "foo_bar".
    pub(crate) name: String,
    // Of the form "name = ..."
    pub(crate) config: String,
}

fn make_paths_absolute(config: String) -> Result<String, Error> {
    // Perhaps not the nicest way to do this. Using a toml parser would possibly
    // be nicer. At the time this was written that wasn't an option due to a
    // compiler bug that prevented us from using any crate that used custom
    // derive. That bug is long fixed though, so switching this to use a toml
    // parser would be an option.
    static PATH_RE: OnceCell<Regex> = OnceCell::new();
    let path_re = PATH_RE.get_or_init(|| Regex::new("^(.*)path *= *\"([^\"]+)\"(.*)$").unwrap());
    if let Some(captures) = path_re.captures(&config) {
        let path = Path::new(&captures[2]);
        if !path.is_absolute() {
            match path.canonicalize() {
                Ok(path) => {
                    return Ok(captures[1].to_owned()
                        + "path = \""
                        + &escape_toml_string(&path.to_string_lossy())
                        + "\""
                        + &captures[3]);
                }
                Err(err) => {
                    bail!("{}: {:?}", err, path);
                }
            }
        }
    }
    Ok(config)
}

/// Escapes a TOML string, see https://toml.io/en/v1.0.0#string
fn escape_toml_string(string: &str) -> String {
    let mut escaped = String::new();

    for char in string.chars() {
        match char {
            '"' | '\\' => {
                escaped.push('\\');
                escaped.push(char);
            }
            // Control characters with special escape sequences
            '\u{0008}' => escaped.push_str("\\b"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\u{000C}' => escaped.push_str("\\f"),
            '\r' => escaped.push_str("\\r"),
            // Control characters using \uXXXX escape sequence
            '\0'..='\u{001F}' | '\u{007F}' => {
                escaped.push_str(&format!("\\u{:04X}", char as u32));
            }
            _ => escaped.push(char),
        }
    }

    escaped
}

impl ExternalCrate {
    pub(crate) fn new(name: String, config: String) -> Result<ExternalCrate, Error> {
        let config = make_paths_absolute(config)?;
        Ok(ExternalCrate { name, config })
    }
}

#[cfg(test)]
mod tests {
    use super::escape_toml_string;
    use super::ExternalCrate;
    use std::path::Path;

    #[test]
    fn test_escape_toml_string() {
        let string = "test \" \\ \\u1234 \u{10FFFF}";
        assert_eq!(
            escape_toml_string(string),
            "test \\\" \\\\ \\\\u1234 \u{10FFFF}"
        );

        let string = "\u{0000} \u{0008} \t \n \u{000C} \r \u{001F} \u{007F}";
        assert_eq!(
            escape_toml_string(string),
            r#"\u0000 \b \t \n \f \r \u001F \u007F"#
        );
    }

    #[test]
    fn make_paths_absolute() {
        let krate =
            ExternalCrate::new("foo".to_owned(), "{ path = \"src/testdata\" }".to_owned()).unwrap();
        assert_eq!(krate.name, "foo");

        let expected_path_string = &escape_toml_string(
            &Path::new("src/testdata")
                .canonicalize()
                .unwrap()
                .to_string_lossy(),
        );
        assert_eq!(
            krate.config,
            format!("{{ path = \"{expected_path_string}\" }}")
        );
    }
}
