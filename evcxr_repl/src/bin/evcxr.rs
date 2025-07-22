// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use anyhow::Result;
use ariadne::sources;
use clap::Parser;
use clap::ValueEnum;
use evcxr::CommandContext;
use evcxr::CompilationError;
use evcxr::Error;
use evcxr::Theme;
use evcxr_repl::BgInitMutex;
use evcxr_repl::EvcxrRustylineHelper;
use rustyline::At;
use rustyline::Cmd;
use rustyline::Editor;
use rustyline::ExternalPrinter;
use rustyline::KeyCode;
use rustyline::KeyEvent;
use rustyline::Modifiers;
use rustyline::Movement;
use rustyline::Word;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use std::fs;
use std::io;
use std::sync::Arc;
use yansi::Color;
use yansi::Paint as _;

const PROMPT: &str = ">> ";

struct Repl {
    command_context: Arc<BgInitMutex<Result<CommandContext, Error>>>,
    ide_mode: bool,
}

fn send_output<T: io::Write + Send + 'static>(
    channel: crossbeam_channel::Receiver<String>,
    mut printer: Option<impl ExternalPrinter + Send + 'static>,
    mut fallback_output: T,
    color: Option<Color>,
) {
    std::thread::spawn(move || {
        while let Ok(line) = channel.recv() {
            let to_print = if let Some(color) = color {
                format!("{}\n", line.paint(color))
            } else {
                format!("{line}\n")
            };
            if let Some(printer) = printer.as_mut() {
                if printer.print(to_print).is_err() {
                    break;
                }
            } else if write!(fallback_output, "{to_print}").is_err() {
                break;
            }
        }
    });
}

impl Repl {
    fn new(
        ide_mode: bool,
        opt: String,
        editor: &mut Editor<EvcxrRustylineHelper, DefaultHistory>,
    ) -> Repl {
        let stdout_printer = editor.create_external_printer().ok();
        let stderr_printer = editor.create_external_printer().ok();
        let stderr_colour = Some(Color::BrightRed);
        let initialize = move || -> Result<CommandContext, Error> {
            let (mut command_context, outputs) = CommandContext::new()?;

            send_output(outputs.stdout, stdout_printer, io::stdout(), None);
            send_output(outputs.stderr, stderr_printer, io::stderr(), stderr_colour);
            command_context.execute(":load_config --quiet")?;
            if !opt.is_empty() {
                // Ignore failure
                command_context.set_opt_level(&opt).ok();
            }
            setup_ctrlc_handler(&command_context);
            Ok(command_context)
        };
        let command_context = Arc::new(BgInitMutex::new(initialize));
        Repl {
            command_context,
            ide_mode,
        }
    }
    fn execute(&mut self, to_run: &str) -> Result<(), Error> {
        let execution_result = match &mut *self.command_context.lock() {
            Ok(context) => context.execute(to_run),
            Err(error) => return Err(error.clone()),
        };
        let success = match execution_result {
            Ok(output) => {
                if let Some(text) = output.get("text/plain") {
                    println!("{text}");
                }
                if let Some(duration) = output.timing {
                    println!("{}", format!("Took {}ms", duration.as_millis()).blue());

                    for phase in output.phases {
                        println!(
                            "{}",
                            format!("  {}: {}ms", phase.name, phase.duration.as_millis()).blue()
                        );
                    }
                }
                true
            }
            Err(evcxr::Error::CompilationErrors(errors)) => {
                self.display_errors(to_run, errors);
                false
            }
            Err(err) => {
                eprintln!("{}", format!("{err}").bright_red());
                false
            }
        };

        if self.ide_mode {
            let success_marker = if success { "\u{0091}" } else { "\u{0092}" };
            print!("{success_marker}");
        }
        Ok(())
    }

    fn display_errors(&mut self, source: &str, errors: Vec<CompilationError>) {
        let mut last_span_lines: &Vec<String> = &vec![];
        for error in &errors {
            if error.is_from_user_code() {
                if let Some(report) =
                    error.build_report("command".to_string(), source.to_string(), Theme::Dark)
                {
                    report
                        .print(sources([("command".to_string(), source.to_string())]))
                        .unwrap();
                    continue;
                }
                for spanned_message in error.spanned_messages() {
                    if let Some(span) = &spanned_message.span {
                        let mut start_column = character_column_to_grapheme_number(
                            span.start_column - 1,
                            &spanned_message.lines[0],
                        );
                        let mut end_column = character_column_to_grapheme_number(
                            span.end_column - 1,
                            spanned_message.lines.last().unwrap(),
                        );
                        // Considering spans can cover multiple lines, it could be that end_column
                        // is less than start_column.
                        if end_column < start_column {
                            std::mem::swap(&mut start_column, &mut end_column);
                        }
                        if source.lines().count() > 1 {
                            // for multi line source code, print the lines
                            if last_span_lines != &spanned_message.lines {
                                for line in &spanned_message.lines {
                                    println!("{line}");
                                }
                            }
                            last_span_lines = &spanned_message.lines;
                        } else {
                            print!("{}", " ".repeat(PROMPT.len()));
                        }
                        print!("{}", " ".repeat(start_column));

                        // Guaranteed not to underflow since if they were out-of-order, we swapped
                        // them above.
                        let span_diff = end_column - start_column;
                        let carrots = "^".repeat(span_diff);
                        print!("{}", carrots.bright_red());
                        println!(" {}", spanned_message.label.bright_blue());
                    } else {
                        // Our error originates from both user-code and generated
                        // code.
                        println!("{}", spanned_message.label.bright_blue());
                    }
                }
                println!("{}", error.message().bright_red());
                for help in error.help() {
                    println!("{} {help}", "help:".bold());
                }
                if let Some(extra_hint) = error.evcxr_extra_hint() {
                    println!("{extra_hint}");
                }
            } else {
                println!(
                    "A compilation error was found in code we generated.\n\
                     Ideally this shouldn't happen. Type :last_error_json to see details.\n{}",
                    error.rendered()
                );
            }
        }
    }
}

/// Returns a 0-based grapheme index corresponding to the supplied 0-based character column.
fn character_column_to_grapheme_number(character_column: usize, line: &str) -> usize {
    let mut characters_remaining = character_column;
    let mut grapheme_index = 0;
    for (_byte_offset, chars) in
        unicode_segmentation::UnicodeSegmentation::grapheme_indices(line, true)
    {
        let num_chars = chars.chars().count();
        if characters_remaining < num_chars {
            break;
        }
        characters_remaining -= num_chars;
        grapheme_index += 1;
    }
    grapheme_index
}

fn readline_direct(prompt: &str) -> rustyline::Result<String> {
    use std::io::Write;

    // Write prompt and flush it to stdout
    let mut stdout = io::stdout();
    stdout.write_all(prompt.as_bytes())?;
    stdout.flush()?;

    let mut line = String::new();
    if io::stdin().read_line(&mut line)? > 0 {
        line = line.replace('\u{2028}', "\n");
        Ok(line)
    } else {
        Err(rustyline::error::ReadlineError::Eof)
    }
}

#[derive(Parser, Debug)]
#[clap(version, about)]
struct Options {
    #[clap(long)]
    disable_readline: bool,

    #[clap(long)]
    ide_mode: bool,

    /// Optimization level (0, 1 or 2)
    #[clap(long, default_value = "")]
    opt: String,

    #[clap(long, default_value = "emacs")]
    edit_mode: EditMode,

    /// Extra arguments; ignored, but show up in std::env::args() which is passed to the subprocess
    /// (see child_process.rs).
    _extra_args: Vec<String>,
}

#[derive(ValueEnum, Default, Clone, Copy, Debug)]
enum EditMode {
    #[default]
    Emacs,
    Vi,
}

fn main() -> Result<()> {
    evcxr::runtime_hook();

    let options = Options::parse();

    println!("Welcome to evcxr. For help, type :help");
    // Print this now, because we silence `:load_config` (writing to stdout
    // interferes with rustyline somewhat).
    if let Some(cfg) = evcxr::config_dir() {
        let init = cfg.join("init.evcxr");
        if init.exists() {
            println!("Startup commands will be loaded from {}", init.display());
        }
        let prelude = cfg.join("prelude.rs");
        if prelude.exists() {
            println!("Prelude will be loaded from {}", prelude.display());
        }
    }
    let mut config_builder = match options.edit_mode {
        EditMode::Vi => {
            rustyline::Config::builder()
                .edit_mode(rustyline::EditMode::Vi)
                .keyseq_timeout(Some(0)) // https://github.com/kkawakam/rustyline/issues/371
        }
        _ => rustyline::Config::builder(), // default edit_mode is emacs
    };
    if std::env::var("EVCXR_COMPLETION_TYPE").as_deref() != Ok("circular") {
        config_builder = config_builder.completion_type(rustyline::CompletionType::List);
    }
    let config = config_builder.build();
    let mut editor = Editor::<EvcxrRustylineHelper, DefaultHistory>::with_config(config)?;
    editor.bind_sequence(
        KeyEvent(KeyCode::Left, Modifiers::CTRL),
        Cmd::Move(Movement::BackwardWord(1, Word::Big)),
    );
    editor.bind_sequence(
        KeyEvent(KeyCode::Right, Modifiers::CTRL),
        Cmd::Move(Movement::ForwardWord(1, At::AfterEnd, Word::Big)),
    );
    let mut repl = Repl::new(options.ide_mode, options.opt.clone(), &mut editor);
    editor.set_helper(Some(EvcxrRustylineHelper::new(Arc::clone(
        &repl.command_context,
    ))));
    let mut opt_history_file = None;
    let config_dir = evcxr::config_dir();
    if let Some(config_dir) = &config_dir {
        fs::create_dir_all(config_dir).ok();
        let history_file = config_dir.join("history.txt");
        editor.load_history(&history_file).ok();
        opt_history_file = Some(history_file);
    }
    let mut interrupted = false;
    loop {
        let prompt = format!("{}", PROMPT.yellow());
        let readline = if options.disable_readline {
            readline_direct(&prompt)
        } else {
            editor.readline(PROMPT)
        };
        match readline {
            Ok(line) => {
                interrupted = false;
                let _ = editor.add_history_entry(line.clone());
                repl.execute(&line)?;
            }
            Err(ReadlineError::Interrupted) => {
                // If the user presses ctrl-c once, then perhaps they meant to
                // interrupt a long-running task, but it finished just before
                // they pressed it. However if they press it twice in a row,
                // then they probably want to exit.
                if interrupted {
                    break;
                }
                interrupted = true;
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                eprintln!("Error: {err:?}");
                break;
            }
        }
    }
    if let Some(history_file) = &opt_history_file {
        editor.save_history(&history_file).ok();
    }
    Ok(())
}

fn setup_ctrlc_handler(command_context: &CommandContext) {
    let subprocess = command_context.process_handle();
    // If we can't register a ctrl-c handler for some reason, then we just don't
    // support catching ctrl-c. The user probably wouldn't want to see an error
    // printed every time, so we ignore it.
    let _ = ctrlc::set_handler(move || {
        let _ = subprocess.lock().unwrap().kill();
    });
}

#[cfg(feature = "mimalloc")]
#[global_allocator]
static MIMALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_character_column_to_grapheme_number() {
        assert_eq!(character_column_to_grapheme_number(0, ""), 0);
        assert_eq!(character_column_to_grapheme_number(0, "aaa"), 0);
        assert_eq!(character_column_to_grapheme_number(1, "aaa"), 1);
        assert_eq!(character_column_to_grapheme_number(2, "aaa"), 2);
        assert_eq!(character_column_to_grapheme_number(3, "aaa"), 3);
        assert_eq!(character_column_to_grapheme_number(0, "äää"), 0);
        assert_eq!(character_column_to_grapheme_number(1, "äää"), 0);
        assert_eq!(character_column_to_grapheme_number(2, "äää"), 1);
        assert_eq!(character_column_to_grapheme_number(3, "äää"), 1);
        assert_eq!(character_column_to_grapheme_number(4, "äää"), 2);
        assert_eq!(character_column_to_grapheme_number(5, "äää"), 2);
        assert_eq!(character_column_to_grapheme_number(6, "äää"), 3);
        assert_eq!(character_column_to_grapheme_number(7, "äää"), 3);
    }
}
