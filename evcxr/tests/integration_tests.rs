// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use evcxr::CommandContext;
use evcxr::Error;
use evcxr::EvalContext;
use evcxr::EvalContextOutputs;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::Mutex;

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

macro_rules! eval {
    ($ctxt:expr, $($t:tt)*) => {eval_and_unwrap(&mut $ctxt, stringify!($($t)*))}
}

fn new_command_context_and_outputs() -> (CommandContext, EvalContextOutputs) {
    let (eval_context, outputs) = EvalContext::new_for_testing();
    let command_context = CommandContext::with_eval_context(eval_context);
    (command_context, outputs)
}

fn send_output<T: io::Write + Send + 'static>(
    channel: crossbeam_channel::Receiver<String>,
    mut output: T,
) {
    std::thread::spawn(move || {
        while let Ok(line) = channel.recv() {
            if writeln!(output, "{}", line).is_err() {
                break;
            }
        }
    });
}
fn context_pool() -> &'static Mutex<Vec<CommandContext>> {
    static CONTEXT_POOL: OnceCell<Mutex<Vec<CommandContext>>> = OnceCell::new();
    CONTEXT_POOL.get_or_init(|| Mutex::new(vec![]))
}

struct ContextHolder {
    // Only `None` while being dropped.
    ctx: Option<CommandContext>,
}

impl Drop for ContextHolder {
    fn drop(&mut self) {
        if is_context_pool_enabled() {
            let mut pool = context_pool().lock().unwrap();
            let mut ctx = self.ctx.take().unwrap();
            ctx.reset_config();
            ctx.execute(":clear").unwrap();
            pool.push(ctx)
        }
    }
}

impl Deref for ContextHolder {
    type Target = CommandContext;

    fn deref(&self) -> &Self::Target {
        self.ctx.as_ref().unwrap()
    }
}

impl DerefMut for ContextHolder {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx.as_mut().unwrap()
    }
}

fn is_context_pool_enabled() -> bool {
    std::env::var("EVCXR_DISABLE_CTX_POOL")
        .map(|var| var != "1")
        .unwrap_or(true)
}

/// Returns a ContextHolder, which will dereference to a CommandContext. When
/// the ContextHolder is dropped, the held CommandContext will be cleared then
/// returned to a global pool. This reuse speeds up running lots of tests by at
/// least 25%. This is probably mostly due to avoiding the need to reload the
/// standard library in rust-analyzer, as that is quite expensive. If you think
/// a test is causing subsequent tests to misbehave, you can disable the pool by
/// setting `EVCXR_DISABLE_CTX_POOL=1`. This can be helpful for debugging,
/// however the interference problem should be fixed as the ":clear" command,
/// combined with resetting configuration should really be sufficient to ensure
/// that subsequent tests will pass.
fn new_context() -> ContextHolder {
    let ctx = context_pool().lock().unwrap().pop().unwrap_or_else(|| {
        let (context, outputs) = new_command_context_and_outputs();
        send_output(outputs.stderr, io::stderr());
        context
    });
    ContextHolder { ctx: Some(ctx) }
}

fn defined_item_names(eval_context: &CommandContext) -> Vec<&str> {
    let mut defined_names = eval_context.defined_item_names().collect::<Vec<_>>();
    defined_names.sort();
    defined_names
}

fn variable_names_and_types(ctx: &CommandContext) -> Vec<(&str, &str)> {
    let mut var_names = ctx.variables_and_types().collect::<Vec<_>>();
    var_names.sort();
    var_names
}

fn variable_names(ctx: &CommandContext) -> Vec<&str> {
    let mut var_names = ctx
        .variables_and_types()
        .map(|(var_name, _)| var_name)
        .collect::<Vec<_>>();
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
    assert_eq!(eval!(e, a), text_plain("42"));
    // Try to change a mutable variable and check that the error we get is what we expect.
    match e.execute("b = 2;") {
        Err(Error::CompilationErrors(errors)) => {
            if errors.len() != 1 {
                println!("{:#?}", errors);
            }
            assert_eq!(errors.len(), 1);
            if errors[0].code() != Some("E0594") && errors[0].code() != Some("E0384") {
                panic!("Unexpected error {:?}", errors[0].code());
            }
        }
        _ => unreachable!(),
    }

    // Make sure that we can correctly determine variable types when using the
    // question mark operator.
    eval_and_unwrap(
        &mut e,
        r#"
        pub mod foo {
            pub mod bar {
                pub struct Baz {}
                impl Baz {
                    pub fn r42(&self) -> i32 {42}
                }
            }
        }
        fn create_baz() -> Result<Option<foo::bar::Baz>, i32> {
            Ok(Some(foo::bar::Baz {}))
        }
    "#,
    );
    eval_and_unwrap(&mut e, "let v1 = create_baz()?;");
    eval_and_unwrap(&mut e, "let v2 = create_baz()?;");
    assert_eq!(
        eval_and_unwrap(&mut e, "v1.unwrap().r42() + v2.unwrap().r42()"),
        text_plain("84")
    );
}

#[test]
fn missing_semicolon_on_let_stmt() {
    let mut e = new_context();
    eval_and_unwrap(&mut e, "mod foo {pub mod bar { pub struct Baz {} }}");
    match e.execute("let v1 = foo::bar::Baz {}") {
        Err(Error::CompilationErrors(e)) => {
            assert!(e.first().unwrap().message().contains(';'));
        }
        x => {
            panic!("Unexpected result: {:?}", x);
        }
    }
}

#[test]
fn printing() {
    let (mut e, outputs) = new_command_context_and_outputs();

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
        let r2: Rc<RefCell<String>> = Rc::clone(&r);
    );
    eval!(e,
        r.borrow_mut().push_str("f");
        let s = "oo";
    );
    eval!(e,
        r.borrow_mut().push_str(s);
        assert!(*r.borrow() == "foo");
    );
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
        assert_eq!(foo(), 42);
    );
    assert_eq!(defined_item_names(&e), vec!["bar", "foo"]);
}

// This test has recently started failing on windows. It fails when deleting the .pdb file with an
// "access denied" error. No idea why. Perhaps in this scenario the file is still locked for some
// reason. This is a somewhat obscure test and Windows is a somewhat obscure platform, so I'll just
// disable this for now.
#[cfg(not(windows))]
#[test]
fn function_panics_with_variable_preserving() {
    // Don't allow stderr to be printed here. We don't really want to see the
    // panic stack trace when running tests.
    let (mut e, _) = new_command_context_and_outputs();
    eval_and_unwrap(
        &mut e,
        r#"
        :preserve_vars_on_panic 1
        let a = vec![1, 2, 3];
        let b = 42;
    "#,
    );
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
    let (mut e, _) = new_command_context_and_outputs();
    eval_and_unwrap(
        &mut e,
        r#"
        :preserve_vars_on_panic 0
        let a = vec![1, 2, 3];
        let b = 42;
    "#,
    );
    let result = e.execute(stringify!(panic!("Intentional panic {}", b);));
    if let Err(Error::SubprocessTerminated(message)) = result {
        assert!(message.contains("Subprocess terminated"));
    } else {
        panic!("Unexpected result: {:?}", result);
    }
    assert_eq!(variable_names_and_types(&e), vec![]);
    // Make sure that a compilation error doesn't bring the variables back from
    // the dead.
    assert!(e.execute("This will not compile").is_err());
    assert_eq!(variable_names_and_types(&e), vec![]);
}

// Also tests multiple item definitions in the one compilation unit.
#[test]
fn tls_implementing_drop() {
    let mut e = new_context();
    eval!(e,
        pub struct Foo {}
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
        }
    );
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
    assert_eq!(variable_names_and_types(&e), vec![("a", "Option<String>")]);
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

    fn dep_command(&self, extra_options: &str) -> String {
        let path = self.tempdir.path().to_string_lossy().replace('\\', "\\\\");
        if extra_options.is_empty() {
            format!(":dep {}", path)
        } else {
            format!(
                ":dep {} = {{ path = \"{}\", {} }}",
                self.name, path, extra_options
            )
        }
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
    let error = e
        .execute(&crate1.dep_command(r#"features = ["no_such_feature"]"#))
        .unwrap_err();
    assert!(error.to_string().contains("no_such_feature"));
    let crate2 = TmpCrate::new("crate2", "pub fn r22() -> i32 {22}").unwrap();
    let to_run =
        crate1.dep_command("") + "\n" + &crate2.dep_command("") + "\ncrate1::r20() + crate2::r22()";
    let outputs = e.execute(&to_run).unwrap();
    assert_eq!(outputs.content_by_mime_type, text_plain("42"));
}

#[test]
fn crate_name_with_hyphens() {
    let (mut e, _) = new_command_context_and_outputs();
    let crate1 = TmpCrate::new("crate-name-with-hyphens", "pub fn r42() -> i32 {42}").unwrap();
    let to_run =
        crate1.dep_command("") + "\nuse crate_name_with_hyphens;\ncrate_name_with_hyphens::r42()";
    let outputs = e.execute(&to_run).unwrap();
    assert_eq!(outputs.content_by_mime_type, text_plain("42"));
}

// A collection of bits of code that are invalid. Our bar here is that we don't
// crash and each thing we try to evaluate results in an error. The actual
// errors will be produced by the rust compiler and we don't want to tie our
// tests needlessly to the specific error messages, so we don't check what the
// errors are.
#[test]
fn invalid_code() {
    let mut e = new_context();
    assert!(e.execute("use crate;").is_err());
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
    // value. Hey, these tests take time to run, we've got to economize :)
    eval!(e, p1);
    // Destructure our point.
    eval!(e, let Point {x, y: y2} = p1;);
    eval!(e,
        assert_eq!(x, 3);
        assert_eq!(y2, 8);
    );
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
        vec![("a", "(i32, f64)")]
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
    match e.execute("use foobar;") {
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
    match e.execute("let mut s = String::new(); s.push_str(format!(\"\"));") {
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
                vec!["let mut s = String::new(); s.push_str(format!(\"\"));"]
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
    let result = e.execute(stringify!(std::process::abort();));
    if let Err(Error::SubprocessTerminated(message)) = result {
        #[cfg(not(windows))]
        {
            if !message.starts_with("Subprocess terminated with status: signal: 6") {
                panic!("Unexpected abort message: '{message}'");
            }
        }
        #[cfg(windows)]
        {
            assert_eq!(
                message,
                "Subprocess terminated with status: exit code: 0xc0000409"
            );
        }
    } else {
        panic!("Unexpected result: {:?}", result);
    }
    eval!(e, assert_eq!(foo(), 42));
    assert!(e.defined_item_names().next().is_some());
    eval_and_unwrap(&mut e, ":clear");
    eval!(e, assert_eq!(40 + 2, 42););
    assert_eq!(e.defined_item_names().next(), None);
}

#[test]
fn variable_assignment_compile_fail_then_use_statement() {
    let mut e = new_context();
    assert!(e.execute(stringify!(let v = foo();)).is_err());
    eval!(e, use std::collections::HashMap;);
    assert_eq!(eval!(e, 42), text_plain("42"));
}

#[test]
fn int_array() {
    let mut e = new_context();
    eval!(e, let v = [42; 5];);
    eval!(e, assert_eq!(v[4], 42));
}

#[test]
fn const_generics_with_explicit_type() {
    let mut e = new_context();
    eval!(e,
        struct Foo<const I: usize> {
            data: [u8; I],
        }
        let foo: Foo<3> = Foo::<3> { data: [41, 42, 43] };
    );
    eval!(e, assert_eq!(foo.data[1], 42));
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
    let result = e.execute(stringify!(let v = || {42};));
    if let Err(Error::Message(message)) = result {
        if !(message.starts_with("The variable") && message.contains("cannot be persisted")) {
            panic!("Unexpected error: {:?}", message);
        }
    } else {
        panic!("Unexpected result: {:?}", result);
    }
}

#[test]
fn unnamable_type_impl_trait() {
    let mut e = new_context();
    let result = e.execute(stringify!(
        pub trait Bar {}
        impl Bar for i32 {}
        pub fn foo() -> impl Bar {42}
        let v = foo();
    ));
    if let Err(Error::Message(message)) = result {
        if !(message.starts_with("The variable `v` has type")
            && message.contains("cannot be persisted"))
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

#[test]
fn display_type() {
    let mut e = new_context();
    assert_eq!(
        e.execute(":types")
            .unwrap()
            .get("text/plain")
            .unwrap()
            .trim(),
        "Types: true"
    );
    assert_eq!(eval!(e, 42), text_plain(": i32 = 42"));
    assert_eq!(
        eval!(e, Some("hello".to_string())),
        text_plain(": Option<String> = Some(\"hello\")")
    );
    assert_eq!(
        e.execute(":types")
            .unwrap()
            .get("text/plain")
            .unwrap()
            .trim(),
        "Types: false"
    );
    assert_eq!(eval!(e, 42), text_plain("42"));
}

#[test]
fn shorten_type_name() {
    // This is a way to test the evcxr_shorten_type() function, evaluated in the child
    let mut e = new_context();
    e.execute(":fmt {}").unwrap();
    // We need to enable types, so evcxr_shorten_type() will be defined.
    e.execute(":types").unwrap();
    assert_eq!(
        eval!(e, evcxr_shorten_type("alloc::string::String")),
        text_plain(": String = String")
    );
    assert_eq!(
        eval!(
            e,
            evcxr_shorten_type("core::option::Option<alloc::string::String>")
        ),
        text_plain(": String = Option<String>")
    );
    assert_eq!(
        eval!(e, evcxr_shorten_type("i32")),
        text_plain(": String = i32")
    );
}

#[test]
fn question_mark_operator() {
    let mut e = new_context();
    // Make sure question mark works without variables.
    eval!(e, std::fs::read_to_string("/does/not/exist")?;);
    assert!(e.execute(":efmt x").is_err());
    eval_and_unwrap(&mut e, ":efmt {:?}");
    eval!(e,
        let owned = "owned".to_string();
        let copy = 40;
        let mut owned_mut = "owned_mut".to_string();
        let mut copy_mut = 41;
    );
    eval!(e,
        use std::io::Result;
        owned_mut.push_str("42");
        copy_mut += 1;
        let copy = 42;
        let copy2 = 42;
        std::fs::read_to_string("/does/not/exist")?;
        owned_mut.push_str("------");
        copy_mut += 10;
    );
    assert_eq!(variable_names(&e), vec!["copy_mut", "owned", "owned_mut"]);
    eval!(e,
        assert_eq!(owned, "owned");
        assert_eq!(owned_mut, "owned_mut42");
        assert_eq!(copy_mut, 42);
    );
}

#[test]
fn format() {
    let mut e = new_context();
    assert_eq!(eval!(e, format!("{:2x}", 2)), text_plain("\" 2\""));
}

// The final statement, if it doesn't end in a semicolon will be printed. Make
// sure we don't try to print earlier statements just because they don't end in
// semicolons.
#[test]
fn non_semi_statements() {
    let mut e = new_context();
    assert_eq!(
        eval!(e,
            for a in 1..5 {}
            for b in 1..5 {}
            42
        ),
        text_plain("42")
    );
}

#[test]
fn partial_destructuring() {
    let mut e = new_context();
    eval!(e,
        let _ = 1;
        let (x, ..) = (42, 43, 44);
    );
    assert_eq!(eval!(e, x), text_plain("42"));
}

#[test]
fn define_then_call_macro() {
    let mut e = new_context();
    eval!(
        e,
        macro_rules! foo {
            ($a:expr) => {
                40 + $a
            };
        }
    );
    assert_eq!(eval!(e, foo!(2)), text_plain("42"));
}

fn simple_completions(ctx: &mut CommandContext, code: &str) -> HashSet<String> {
    ctx.completions(code, code.len())
        .unwrap()
        .completions
        .into_iter()
        .map(|c| c.code)
        .collect()
}

#[test]
fn code_completion() {
    let mut ctx = new_context();
    // This first bit of code that we execute serves two purposes. Firstly, it's
    // used later in the test. Secondly, it ensures that our first attempt at
    // completion doesn't get confused by user code that has already been
    // evaluated.
    ctx.execute(
        r#"
        mod bar {
            pub struct Baz {}
            impl Baz {
                pub fn fff5() {}
            }
        }
        let var1 = 42;
        let var2 = String::new();"#,
    )
    .unwrap();
    let code = r#"
        fn foo() -> Vec<String> {
            vec![]
        }
        foo().res"#;
    let completions = ctx.completions(code, code.len()).unwrap();
    assert!(!completions.completions.is_empty());
    assert!(completions
        .completions
        .iter()
        .any(|c| c.code == "reserve(additional)"));
    for c in completions.completions {
        if !c.code.starts_with("res") {
            panic!("Unexpected completion: '{}'", c.code);
        }
    }
    assert_eq!(completions.start_offset, code.len() - "res".len());
    assert_eq!(completions.end_offset, code.len());

    // Check command completions.
    let completions = ctx.completions(":de", 3).unwrap();
    assert_eq!(completions.start_offset, 0);
    assert_eq!(completions.end_offset, 3);
    assert_eq!(
        completions
            .completions
            .iter()
            .map(|c| c.code.as_str())
            .collect::<Vec<_>>(),
        vec![":dep"]
    );

    // Check that we get zero completions when expected.
    let code = code.replace("res", "asdfasdf");
    assert_eq!(
        ctx.completions(&code, code.len()).unwrap().completions,
        vec![]
    );

    // Check that user-defined variables are included in the completions, but
    // evcxr internal variables are not.
    let completions = simple_completions(&mut ctx, "let _ = v");
    assert!(completions.contains("var1"));
    assert!(completions.contains("var2"));
    assert!(!completions.contains("vars_ok"));
    let completions = simple_completions(&mut ctx, "let _ = e");
    assert!(!completions.contains("evcxr_variable_store"));
    assert!(!completions.contains("evcxr_internal_runtime"));
    assert!(!completions.contains("evcxr_analysis_wrapper"));

    // We handle use-statements differently to other code, make sure that we can
    // still get completions from them.
    let code = "use bar::";
    let completions = ctx.completions(code, code.len()).unwrap();
    assert!(completions.completions.iter().any(|c| c.code == "Baz"));

    // Rust-analyzer yet doesn't handle use-statements inside a block, so if the
    // following use-statement ends up in a block, then `Baz` won't resolve and
    // we won't get any completions.
    let code = "use bar::Baz; Baz::";
    let completions = ctx.completions(code, code.len()).unwrap();
    assert!(completions.completions.iter().any(|c| c.code == "fff5()"));
}

#[test]
fn repeated_use_statements() {
    let mut e = new_context();
    eval_and_unwrap(
        &mut e,
        r#"
        mod foo {
            pub struct Bar {}
            impl Bar {
                pub fn result() -> i32 {
                    42
                }
                pub fn new() -> Bar { Bar {} }
            }
            pub struct Baz {}
            pub trait Foo {
                fn do_foo(&self) -> i32 {42}
            }
            impl Foo for Bar {}
            pub mod s1 {
                pub fn f1() {}
            }
            pub mod s2 {
                pub fn f2() {}
            }
        }
        use std::iter::Iterator as _;
        use foo::Foo as _;
        use foo::s1::*;
        use foo::s2::*;
        use foo::Bar;"#,
    );
    // Try a bad import. This should fail, but shouldn't affect subsequent eval
    // calls.
    assert!(e.execute("use this::will::fail;").is_err());
    assert_eq!(
        eval_and_unwrap(&mut e, "use foo::Bar as _; Bar::result()"),
        text_plain("42")
    );
    assert_eq!(
        eval_and_unwrap(
            &mut e,
            "use foo::{Bar, Baz}; f1(); f2(); Bar::new().do_foo()"
        ),
        text_plain("42")
    );
}

#[track_caller]
fn check(ctx: &mut CommandContext, code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for err in ctx.check(code).unwrap() {
        if let Some(spanned_message) = err.primary_spanned_message() {
            if let Some(span) = spanned_message.span {
                out.push(format!(
                    "{} {}:{}-{}:{}",
                    err.level(),
                    span.start_line,
                    span.start_column,
                    span.end_line,
                    span.end_column,
                ));
            }
        }
    }
    out
}

fn strs(input: &[String]) -> Vec<&str> {
    let mut result: Vec<&str> = input.iter().map(|s| s.as_str()).collect();
    result.sort();
    result
}

#[track_caller]
fn assert_no_errors(ctx: &mut CommandContext, code: &str) {
    assert_eq!(strs(&check(ctx, code)), Vec::<&str>::new());
}

#[test]
fn check_for_errors() {
    let mut ctx = new_context();

    assert_eq!(
        strs(&check(
            &mut ctx,
            r#"let a = 10;
let s2 = "さび  äää"; let s2: String = 42; fn foo() -> i32 {
    println!("さび  äää"); (
    )
}
"#
        )),
        vec!["error 2:41-2:43", "error 3:29-4:6"]
    );

    // An unused variable not within a function shouldn't produce a warning.
    assert_no_errors(&mut ctx, "let mut s = String::new();");

    // An unused variable within a function should produce a warning. Older versions of rustc have
    // the warning span on `mut s`, while newer ones have it just one `s`.
    let unused_var_warnings = check(&mut ctx, "fn foo() {let mut s = String::new();}");
    if strs(&unused_var_warnings) != vec!["warning 1:19-1:20"] {
        assert_eq!(strs(&unused_var_warnings), vec!["warning 1:15-1:20"]);
    }

    // Make sure we don't get errors about duplicate use statements after we've
    // executed some code.
    eval_and_unwrap(&mut ctx, "use std::fmt::Debug; 42");
    assert_no_errors(&mut ctx, "use std::fmt::Debug; 42");

    // Make sure we do get errors for a bad use statement. Currently this is
    // limited to simple use statements (with {}).
    assert_eq!(
        strs(&check(&mut ctx, "use std::foo::Bar;")),
        vec!["error 1:10-1:13"]
    );

    // Make sure that we can report errors resulting from macro expansions. The first span is the
    // whole macro, the second span, which is what appears in later versions of rustc, is just the
    // variable `s`.
    let allowed = ["error 1:28-1:44", "error 1:35-1:36"];
    let actual = check(
        &mut ctx,
        r#"let mut s = String::new(); write!(s, "foo").unwrap();"#,
    );
    let actual = &strs(&actual)[0];
    if !allowed.contains(actual) {
        panic!("Found '{actual}', but expected one of {allowed:?}");
    }

    // Check that errors adding crates are reported.
    assert_eq!(
        strs(&check(
            &mut ctx,
            "\
            :dep this_crate_does_not_exist = \"12.34\"\n\
            :dep foo = { path = \"/this/path/does/not/exist\" }\n\
            // This is a comment interleaved with commands\n\
            :an_invalid_command
            "
        )),
        vec!["error 1:6-1:41", "error 2:6-2:50", "error 4:1-4:20"]
    );

    // Make sure missing close parenthesis are reported as expected. Note, we still don't check the
    // message here, but the span of the message gives clues that we're getting an appropriate
    // error.
    assert_eq!(
        strs(&check(&mut ctx, "std::mem::drop(42); std::mem::drop(")),
        vec!["error 1:35-1:36"]
    );

    // Attempts to store a variable containing a non-static reference should report an error.
    assert_eq!(
        strs(&check(&mut ctx, "let s1 = String::new(); let v = &s1;")),
        vec!["error 1:29-1:30"]
    );

    // Dropped variables shouldn't report errors.
    assert_no_errors(&mut ctx, "let s1 = String::new(); std::mem::drop(s1);");
}
