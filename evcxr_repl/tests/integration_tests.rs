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

use std::process;

// Checks that our binary can be executed. This used to be an important thing to
// check due to https://github.com/rust-lang/rust/issues/45601 which meant that
// we could easily end up with a binary that couldn't be executed (without
// LD_LIBRARY PATH or similar). That bug is now long fixed, but this test
// perhaps still has some value.
#[test]
fn test_binary_execution() {
    let output = process::Command::new(
        std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("evcxr"),
    )
    .env_remove("LD_LIBRARY_PATH")
    .output()
    .unwrap();
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert_eq!(stderr, "");
    if !stdout.contains("Welcome to evcxr") {
        panic!("Unexpected output:\n{:?}", stdout);
    }
}
