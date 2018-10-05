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

use errors::Error;

#[derive(Clone)]
pub(crate) struct ExternalCrate {
    // The name, as it appears in the crate registry. This may be different than the key against
    // which this ExternalCrate is stored. If this name is "foo-bar", the key would be normalized as
    // "foo_bar".
    pub(crate) name: String,
    // Of the form "name = ..."
    pub(crate) config: String,
}

impl ExternalCrate {
    pub(crate) fn new(name: String, config: String) -> Result<ExternalCrate, Error> {
        Ok(ExternalCrate { name, config })
    }
}

#[cfg(test)]
mod tests {
    use super::ExternalCrate;
    use std::env;

    #[test]
    #[ignore]
    fn make_paths_absolute() {
        println!("{:?}", env::current_dir());
        let krate = ExternalCrate::new("foo".to_owned(), "{ path = \"my_crates/foo\" }".to_owned())
            .unwrap();
        assert_eq!(krate.name, "foo");
        assert_eq!(
            krate.config,
            format!(
                "{{ path = \"{}/my_crates/foo\" }}",
                env::current_dir().unwrap().to_string_lossy()
            )
        );
    }
}
