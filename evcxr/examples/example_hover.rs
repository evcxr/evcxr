// Copyright 2022 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use evcxr::CommandContext;
use evcxr::Error;
use evcxr::EvalContext;
use evcxr::EvalContextOutputs;
use std::collections::HashMap;

#[track_caller]
fn eval_and_unwrap(ctxt: &mut CommandContext, code: &str) -> HashMap<String, String> {
    match ctxt.execute(code) {
        Ok(output) => output.content_by_mime_type,
        Err(err) => {
            println!(
                "======== last src ========\n{}==========================",
                ctxt.last_source().unwrap()
            );
            match err {
                Error::CompilationErrors(errors) => {
                    for error in errors {
                        println!("{}", error.rendered());
                    }
                }
                other => println!("{}", other),
            }

            panic!("Unexpected compilation error. See above for details");
        }
    }
}

fn new_command_context_and_outputs() -> (CommandContext, EvalContextOutputs) {
    let (eval_context, outputs) = EvalContext::new_for_testing();
    let command_context = CommandContext::with_eval_context(eval_context);
    (command_context, outputs)
}

fn main() -> Result<(), Error> {
    let (mut e, _) = new_command_context_and_outputs();
    eval_and_unwrap(&mut e, r#"let a: usize = 1;"#);
    eval_and_unwrap(&mut e, r#"
    ///this is my struct
    struct MyStruct(usize);
    "#);
    println!("{}", eval_and_unwrap(&mut e, r#":doc MyStruct"#)["text/html"]);
    println!("{}", eval_and_unwrap(&mut e, r#":doc let"#)["text/html"]);
    println!("{}", eval_and_unwrap(&mut e, r#":doc std::fs::File::create"#)["text/html"]);
    Ok(())
}
