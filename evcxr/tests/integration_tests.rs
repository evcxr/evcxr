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

use evcxr;

use evcxr::{CommandContext, Error, EvalContext, EvalContextOutputs};
use std::collections::HashMap;
use std::io;
use std::sync::mpsc;
use tempfile;

macro_rules! eval {
    ($ctxt:expr, $($t:tt)*) => {$ctxt.eval(stringify!($($t)*)).unwrap().content_by_mime_type}
}

fn new_context_and_outputs() -> (EvalContext, EvalContextOutputs) {
    let testing_runtime_path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testing_runtime");
    EvalContext::with_subprocess_command(std::process::Command::new(&testing_runtime_path)).unwrap()
}

fn new_command_context_and_outputs() -> (CommandContext, EvalContextOutputs) {
    let (eval_context, outputs) = new_context_and_outputs();
    let command_context = CommandContext::with_eval_context(eval_context);
    (command_context, outputs)
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

fn variable_names_and_types(ctx: &EvalContext) -> Vec<(&str, &str)> {
    let mut var_names = ctx.variables_and_types().collect::<Vec<_>>();
    var_names.sort();
    var_names
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
fn function_panics_with_variable_preserving() {
    // Don't allow stderr to be printed here. We don't really want to see the
    // panic stack trace when running tests.
    let (mut e, _) = new_context_and_outputs();
    e.preserve_vars_on_panic = true;
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

#[test]
fn function_panics_without_variable_preserving() {
    // Don't allow stderr to be printed here. We don't really want to see the
    // panic stack trace when running tests.
    let (mut e, _) = new_context_and_outputs();
    e.preserve_vars_on_panic = false;
    eval!(e, let a = vec![1, 2, 3];);
    eval!(e, let b = 42;);
    let result = e.eval(stringify!(panic!("Intentional panic {}", b);));
    if let Err(Error::ChildProcessTerminated(message)) = result {
        assert!(message.contains("Child process terminated"));
    } else {
        panic!("Unexpected result: {:?}", result);
    }
    assert_eq!(variable_names_and_types(&e), vec![]);
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
        variable_names_and_types(&e),
        vec![("a", "std::option::Option<std::string::String>")]
    );
    assert_eq!(eval!(e, a.unwrap()), text_plain("\"foo\""));
    assert_eq!(variable_names_and_types(&e), vec![]);
}

struct TmpCrate {
    name: String,
    tempdir: tempfile::TempDir,
}

impl TmpCrate {
    fn new(name: &str, src: &str) -> Result<TmpCrate, io::Error> {
        let tempdir = tempfile::tempdir()?;
        let src_dir = tempdir.path().join("src");
        std::fs::create_dir_all(&src_dir)?;
        std::fs::write(
            tempdir.path().join("Cargo.toml"),
            format!(
                "\
                 [package]\n\
                 name = \"{}\"\n\
                 version = \"0.0.1\"\n\
                 edition = \"2018\"\n\
                 ",
                name
            ),
        )?;
        std::fs::write(src_dir.join("lib.rs"), src)?;
        Ok(TmpCrate {
            name: name.to_owned(),
            tempdir,
        })
    }

    fn dep_command(&self) -> String {
        format!(
            ":dep {} = {{ path = \"{}\" }}",
            self.name,
            self.tempdir.path().to_string_lossy().replace("\\", "\\\\")
        )
    }
}

#[test]
fn crate_deps() {
    let (mut e, _) = new_command_context_and_outputs();
    // Try loading a crate that doesn't exist. This it to make sure that we
    // don't keep this bad crate around for subsequent execution attempts.
    let r = e.execute(
        r#"
       :dep bad = { path = "this_path_does_not_exist"}
       40"#,
    );
    assert!(r.is_err());
    let crate1 = TmpCrate::new("crate1", "pub fn r20() -> i32 {20}").unwrap();
    let crate2 = TmpCrate::new("crate2", "pub fn r22() -> i32 {22}").unwrap();
    let to_run =
        crate1.dep_command() + "\n" + &crate2.dep_command() + "\ncrate1::r20() + crate2::r22()";
    let outputs = e.execute(&to_run).unwrap();
    assert_eq!(outputs.content_by_mime_type, text_plain("42"));
}

// This test still needs some work.
#[test]
#[ignore]
fn crate_name_with_hyphens() {
    let (mut e, _) = new_command_context_and_outputs();
    let crate1 = TmpCrate::new("crate-name-with-hyphens", "pub fn r42() -> i32 {42}").unwrap();
    let to_run = crate1.dep_command()
        + "\nextern crate crate_name_with_hyphens;\ncrate_name_with_hyphens::r42()";
    let outputs = e.execute(&to_run).unwrap();
    assert_eq!(outputs.content_by_mime_type, text_plain("42"));
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

#[test]
fn redefine_type_with_existing_var() {
    let mut e = new_context();
    eval!(e,
        struct Foo {x: i32}
        let f1 = Foo {x: 42};
        let f2 = Foo {x: 42};
    );
    assert_eq!(
        variable_names_and_types(&e),
        vec![("f1", "Foo"), ("f2", "Foo")]
    );
    eval!(e,
        struct Foo { x: i32, y: i32 }
        let f3 = Foo {x: 42, y: 43};
    );
    // `f1` and `f2` should have been dropped because the type Foo was
    // redefined.
    assert_eq!(variable_names_and_types(&e), vec![("f3", "Foo")]);
    // Make sure that we actually evaluated the above by checking that f3 is
    // accessible.
    eval!(e,
        assert_eq!(f3.x, 42);
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
        #[cfg(not(windows))]
        {
            assert_eq!(message, "Child process terminated with status: signal: 6");
        }
        #[cfg(windows)]
        {
            assert_eq!(
                message,
                "Child process terminated with status: exit code: 0xc0000409"
            );
        }
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
fn int_array() {
    let mut e = new_context();
    eval!(e, let v = [42; 5];);
    eval!(e, assert_eq!(v[4], 42));
}

// Make sure that a type name containing a reserved word (e.g. async) doesn't
// cause a compilation error.
#[test]
fn reserved_words() {
    let mut e = new_context();
    eval!(e,
        mod r#async { pub struct Foo {} }
        let v = r#async::Foo {};
    );
}

#[test]
fn unnamable_type_closure() {
    let mut e = new_context();
    let result = e.eval(stringify!(let v = || {42};));
    if let Err(Error::Message(message)) = result {
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
    if let Err(Error::Message(message)) = result {
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
fn partially_inferred_variable_type() {
    let mut e = new_context();
    eval!(e, let v : Vec<_> = (1..10).collect(););
    eval!(e, assert_eq!(v.len(), 9););
}

// Makes sure that we properly handle switching from code that doesn't need a
// variable store to code that does.
#[test]
fn print_then_assign_variable() {
    let mut e = new_context();
    eval!(e, println!("Hello, world!"););
    eval!(e, let x = 42;);
}
