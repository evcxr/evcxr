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

use crate::errors::Error;
use regex::Regex;
use std::path::Path;

#[derive(Clone)]
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
    lazy_static! {
        static ref PATH_RE: Regex = Regex::new("^(.*)path *= *\"([^\"]+)\"(.*)$").unwrap();
    }
    if let Some(captures) = PATH_RE.captures(&config) {
        let path = Path::new(&captures[2]);
        if !path.is_absolute() {
            match path.canonicalize() {
                Ok(path) => {
                    return Ok(captures[1].to_owned()
                        + "path = \""
                        + &path.to_string_lossy()
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

impl ExternalCrate {
    pub(crate) fn new(name: String, config: String) -> Result<ExternalCrate, Error> {
        let config = make_paths_absolute(config)?;
        Ok(ExternalCrate { name, config })
    }
}

#[cfg(test)]
mod tests {
    use super::ExternalCrate;
    use std::path::Path;

    #[test]
    fn make_paths_absolute() {
        let krate =
            ExternalCrate::new("foo".to_owned(), "{ path = \"src/testdata\" }".to_owned()).unwrap();
        assert_eq!(krate.name, "foo");
        assert_eq!(
            krate.config,
            format!(
                "{{ path = \"{}\" }}",
                Path::new("src/testdata")
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
            )
        );
    }
}
