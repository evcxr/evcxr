// Copyright 2022 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

// use evcxr::CommandContext;
// use evcxr::Error;
use evcxr::EvalContext;
// use evcxr::EvalContextOutputs;
// use std::collections::HashMap;


fn main() {
    let (mut e, _) = EvalContext::new_for_testing();
    // e.eval("let zz = vec![vec![1f64; 10]; 10];").unwrap();
    e.eval(r#"let a = undefined_fn();"#).unwrap();
    // e.eval("zz[1:100]").unwrap();
    // let zz = undef_fn(1, 2);
}
