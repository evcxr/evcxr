// Copyright 2022 The Evcxr Authors.
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

use evcxr::Error;
use evcxr::EvalContext;

fn main() -> Result<(), Error> {
    // You must call ```evcxr::runtime_hook()``` at the top of main, otherwise
    // the library becomes a fork-bomb.
    evcxr::runtime_hook();

    let (mut context, outputs) = EvalContext::new()?;
    context.eval("let mut s = String::new();")?;
    context.eval(r#"s.push_str("Hello, ");"#)?;
    context.eval(r#"s.push_str("World!");"#)?;
    context.eval(r#"println!("{}", s);"#)?;

    // For this trivial example, we just receive a single line of output from
    // the code that was run. In a more complex case, we'd likely want to have
    // separate threads waiting for output on both stdout and stderr.
    if let Ok(line) = outputs.stdout.recv() {
        println!("{line}");
    }

    Ok(())
}
