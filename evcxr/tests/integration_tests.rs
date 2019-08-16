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

extern crate evcxr;

use evcxr::{Error, EvalContext, EvalContextOutputs};
use std::collections::HashMap;
use std::io;
use std::sync::mpsc;

macro_rules! eval {
    ($ctxt:expr, $($t:tt)*) => {$ctxt.eval(stringify!($($t)*)).unwrap().content_by_mime_type}
}

fn new_context_and_outputs() -> (EvalContext, EvalContextOutputs) {
    if false {
        evcxr::runtime_hook();

        let mut command = std::process::Command::new(&std::env::current_exe().unwrap());
        command
            .arg("--test-threads")
            .arg("1")
            .arg("--nocapture")
            .arg("--quiet");
        EvalContext::with_subprocess_command(command).unwrap()
    } else {
        let testing_runtime_path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("testing_runtime");
        EvalContext::with_subprocess_command(std::process::Command::new(&testing_runtime_path))
            .unwrap()
    }
}

fn send_output<T: io::Write + Send + 'static>(channel: mpsc::Receiver<String>, mut output: T) {
    std::thread::spawn(move || {
        while let Ok(line) = channel.recv() {
            if writeln!(output, "{}", line).is_err() {
                break;
            }
        }
    });
}

fn new_context() -> EvalContext {
    let (context, outputs) = new_context_and_outputs();
    send_output(outputs.stderr, io::stderr());
    context
}

#[test]
fn single_statement() {
    let mut e = new_context();
    eval!(e, assert_eq!(40i32 + 2, 42));
}

#[test]
fn save_and_restore_variables() {
    let mut e = new_context();

    eval!(e, let mut a = 34; let b = 8;);
    eval!(e, a = a + b;);
    eval!(e, assert_eq!(a, 42););
    assert_eq!(eval!(e, a), text_plain("42"));
    // Try to change a mutable variable and check that the error we get is what we expect.
    match e.eval("b = 2;") {
        Err(Error::CompilationErrors(errors)) => {
            if errors.len() != 1 {
                println!("{:#?}", errors);
            }
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0].code(), Some("E0384"));
        }
        _ => unreachable!(),
    }
}

#[test]
fn printing() {
    let (mut e, outputs) = new_context_and_outputs();

    eval!(e,
        println!("This is stdout");
        eprintln!("This is stderr");
        println!("Another stdout line");
        eprintln!("Another stderr line");
    );
    assert_eq!(outputs.stdout.recv(), Ok("This is stdout".to_owned()));
    assert_eq!(outputs.stderr.recv(), Ok("This is stderr".to_owned()));
    assert_eq!(outputs.stdout.recv(), Ok("Another stdout line".to_owned()));
    assert_eq!(outputs.stderr.recv(), Ok("Another stderr line".to_owned()));
}

#[test]
fn rc_refcell_etc() {
    let mut e = new_context();
    eval!(e,
        use std::cell::RefCell; use std::rc::Rc;
        let r: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    );
    eval!(e, let r2: Rc<RefCell<String>> = Rc::clone(&r););
    eval!(e, r.borrow_mut().push_str("f"););
    eval!(e, let s = "oo";);
    eval!(e, r.borrow_mut().push_str(s););
    eval!(e, assert!(*r.borrow() == "foo"););
}

#[test]
fn define_then_call_function() {
    let mut e = new_context();
    eval!(
        e,
        pub fn bar() -> i32 {
            42
        }
    );
    eval!(
        e,
        pub fn foo() -> i32 {
            bar()
        }
    );
    eval!(e, assert_eq!(foo(), 42););
    let mut defined_names = e.defined_item_names().collect::<Vec<_>>();
    defined_names.sort();
    assert_eq!(defined_names, vec!["bar", "foo"]);
}

#[test]
fn function_panics() {
    // Don't allow stderr to be printed here. We don't really want to see the
    // panic stack trace when running tests.
    let (mut e, _) = new_context_and_outputs();
    eval!(e, let a = vec![1, 2, 3];);
    eval!(e, let b = 42;);
    eval!(e, panic!("Intentional panic {}", b););
    // The variable a isn't referenced by the code that panics, while the variable b implements
    // Copy, so neither should be lost.
    assert_eq!(
        eval!(e, format!("{:?}, {}", a, b)),
        text_plain("\"[1, 2, 3], 42\"")
    );
}

// Also tests multiple item definitions in the one compilation unit.
#[test]
fn tls_implementing_drop() {
    let mut e = new_context();
    eval!(e, pub struct Foo {}
    impl Drop for Foo {
        fn drop(&mut self) {
            println!("Dropping Foo");
        }
    }
    pub fn init_foo() {
        thread_local! {
            pub static FOO: Foo = Foo {};
        }
        FOO.with(|f| ())
    });
    eval!(e, init_foo(););
}

fn text_plain(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert("text/plain".to_owned(), content.to_owned());
    map
}

#[test]
fn moved_value() {
    let mut e = new_context();
    eval!(e, let a = Some("foo".to_owned()););
    assert_eq!(
        e.variables_and_types().collect::<Vec<_>>(),
        vec![("a", "std::option::Option<std::string::String>")]
    );
    assert_eq!(eval!(e, a.unwrap()), text_plain("\"foo\""));
    assert_eq!(e.variables_and_types().collect::<Vec<_>>(), vec![]);
}

#[test]
fn struct_type_inference() {
    let mut e = new_context();
    eval!(
        e,
        #[derive(Debug)]
        pub struct Point {
            pub x: i32,
            pub y: i32,
        }
    );
    eval!(e, let p1 = Point {x: 3, y: 8};);
    // While we're here, also test that printing an expression doesn't move the (non-copy)
    // value. Hey, these tests take time to run, we've got to economise :)
    eval!(e, p1);
    // Destructure our point.
    eval!(e, let Point {x, y: y2} = p1;);
    eval!(e, assert_eq!(x, 3););
    eval!(e, assert_eq!(y2, 8););
    let mut defined_names = e.defined_item_names().collect::<Vec<_>>();
    defined_names.sort();
    assert_eq!(defined_names, vec!["Point"]);
}

#[test]
fn non_concrete_types() {
    let mut e = new_context();
    eval!(e, let a = (42, 3.14););
    assert_eq!(
        e.variables_and_types().collect::<Vec<_>>(),
        vec![("a", "(i32, f32)")]
    );
}

#[test]
fn statement_and_expression() {
    let mut e = new_context();
    assert_eq!(
        eval!(e, let a = "foo".to_owned() + "bar"; a),
        text_plain("\"foobar\"")
    );
}

#[test]
fn continue_execution_after_bad_use_statement() {
    let mut e = new_context();
    // First make sure we get the error we expect.
    match e.eval("use foobar;") {
        Err(Error::CompilationErrors(errors)) => {
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0].code(), Some("E0432"));
        }
        x => panic!("Unexpected result: {:?}", x),
    }
    // Now make sure we can still execute code.
    assert_eq!(eval!(e, "f".to_string() + "oo"), text_plain("\"foo\""));
}

#[test]
fn error_from_macro_expansion() {
    let mut e = new_context();
    // The the following line we're missing & before format!. The compiler reports the error as
    // coming from "<format macros>" with expansion information leading to the user code. Make sure
    // we ignore the span from non-user code and use the expansion info correctly.
    match e.eval("let mut s = String::new(); s.push_str(format!(\"\"));") {
        Err(Error::CompilationErrors(errors)) => {
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0].code(), Some("E0308")); // mismatched types
            let mut lines = std::collections::HashSet::new();
            for spanned_message in errors[0].spanned_messages() {
                for line in &spanned_message.lines {
                    lines.insert(line.as_str());
                }
            }
            // There's only one line on which we should be reporting errors...
            assert_eq!(
                lines.into_iter().collect::<Vec<_>>(),
                vec!["s.push_str(format!(\"\"));"]
            );
        }
        x => panic!("Unexpected result: {:?}", x),
    }
}

#[test]
fn multiple_identical_use_statements() {
    let mut e = new_context();
    eval!(e, use std::collections::HashMap;);
    eval!(e, use std::collections::HashMap;);
}

// Defines a type, creates variable of that type (both from the same and from a
// different compilation unit), then redefines the type and makes sure we can
// still call an old method on the variables.
#[test]
fn redefine_type_reference_old_var() {
    let mut e = new_context();
    eval!(e,
        pub struct Foo {}
        impl Foo {
            pub fn get_value(&self) -> i32 { 42 }
        }
        let f = Foo {};
    );
    eval!(e, let f2 = Foo {};);
    eval!(e,
        pub struct Foo { pub value: i32 }
        impl Foo {
            pub fn get_value(&self) -> i32 { self.value }
        }
    );
    eval!(
        e,
        assert_eq!(f.get_value(), 42);
        assert_eq!(f2.get_value(), 42);
    );
}

#[test]
fn abort_and_restart() {
    let mut e = new_context();
    eval!(
        e,
        pub fn foo() -> i32 {
            42
        }
        // Define a variable in order to check that we don't think it still
        // exists after a restart.
        let a = 42i32;
    );
    let result = e.eval(stringify!(std::process::abort();));
    if let Err(Error::ChildProcessTerminated(message)) = result {
        assert_eq!(message, "Child process terminated with status: signal: 6");
    } else {
        panic!("Unexpected result: {:?}", result);
    }
    eval!(e, assert_eq!(foo(), 42));
    e.clear().unwrap();
    eval!(e, assert_eq!(40 + 2, 42););
}

#[test]
fn variable_assignment_compile_fail_then_use_statement() {
    let mut e = new_context();
    assert!(e.eval(stringify!(let v = foo();)).is_err());
    eval!(e, use std::collections::HashMap;);
    assert_eq!(eval!(e, 42), text_plain("42"));
}

#[test]
fn unnamable_type_closure() {
    let mut e = new_context();
    let result = e.eval(stringify!(let v = || {42};));
    if let Err(Error::JustMessage(message)) = result {
        if !(message.starts_with("Sorry, the type")
            && message.contains("cannot currently be persisted"))
        {
            panic!("Unexpected error: {:?}", message);
        }
    } else {
        panic!("Unexpected result: {:?}", result);
    }
}

#[test]
fn unnamable_type_impl_trait() {
    let mut e = new_context();
    let result = e.eval(stringify!(
        pub trait Bar {}
        impl Bar for i32 {}
        pub fn foo() -> impl Bar {42}
        let v = foo();
    ));
    if let Err(Error::JustMessage(message)) = result {
        if !(message.starts_with("Sorry, the type")
            && message.contains("cannot currently be persisted"))
        {
            panic!("Unexpected error: {:?}", message);
        }
    } else {
        panic!("Unexpected result: {:?}", result);
    }
}
