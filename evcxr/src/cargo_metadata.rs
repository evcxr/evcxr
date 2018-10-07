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
use json::{self, JsonValue};
use std;
use std::collections::HashMap;
use std::path::Path;

/// Returns the library names for the direct dependencies of the crate rooted at
/// the specified path.
pub(crate) fn get_library_names(crate_dir: &Path) -> Result<Vec<String>, Error> {
    let output = std::process::Command::new("cargo")
        .arg("metadata")
        .current_dir(crate_dir)
        .output()?;
    library_names_from_metadata(crate_dir, std::str::from_utf8(&output.stdout)?)
}

fn library_names_from_metadata(crate_dir: &Path, metadata: &str) -> Result<Vec<String>, Error> {
    let metadata = json::parse(metadata)?;
    let mut direct_dependencies = Vec::new();
    let mut crate_to_library_names = HashMap::new();
    if let JsonValue::Array(packages) = &metadata["packages"] {
        for package in packages {
            if let Some(package_name) = package["name"].as_str() {
                if let Some(manifest_path) = package["manifest_path"].as_str() {
                    if Path::new(manifest_path).parent() == Some(crate_dir) {
                        if let JsonValue::Array(dependencies) = &package["dependencies"] {
                            for dependency in dependencies {
                                if let Some(dependency_name) = dependency["name"].as_str() {
                                    direct_dependencies.push(dependency_name);
                                }
                            }
                        }
                    }
                }
                if let JsonValue::Array(targets) = &package["targets"] {
                    for target in targets {
                        if let JsonValue::Array(kinds) = &target["kind"] {
                            if kinds.iter().any(|kind| kind == "lib") {
                                if let Some(target_name) = target["name"].as_str() {
                                    crate_to_library_names
                                        .insert(package_name, target_name.to_owned());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let mut library_names = Vec::new();
    for dep_name in direct_dependencies {
        if let Some(lib_name) = crate_to_library_names.get(dep_name) {
            library_names.push(lib_name.clone());
        }
    }
    Ok(library_names)
}

#[cfg(test)]
mod tests {
    use super::library_names_from_metadata;
    use std::path::Path;

    #[test]
    fn test_library_names_from_metadata() {
        assert_eq!(
            library_names_from_metadata(
                Path::new("/home/foo/project"),
                include_str!("testdata/sample_metadata.json")
            ).unwrap(),
            vec!["direct1", "direct2"]
        );
    }
}
