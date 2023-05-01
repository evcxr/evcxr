// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

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
            .join("evcxr_jupyter"),
    )
    .arg("--help")
    .env_remove("LD_LIBRARY_PATH")
    .output()
    .unwrap();
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert_eq!(stderr, "");
    if !stdout.contains("To install, run") {
        panic!("Unexpected output:\n{:?}", stdout);
    }
}
