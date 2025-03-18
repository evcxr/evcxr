// Copyright 2022 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

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
