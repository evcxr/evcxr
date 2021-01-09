// Copyright 2020 The Evcxr Authors.
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

use colored::*;
use evcxr::{CommandContext, CompilationError, Error};
use rustyline::{error::ReadlineError, At, Cmd, EditMode, Editor, KeyPress, Movement, Word};
use std::fs;
use std::io;
use std::sync::{mpsc, Arc, Mutex};
use structopt::StructOpt;

use evcxr_repl::EvcxrRustylineHelper;

const PROMPT: &str = ">> ";

struct Repl {
    command_context: Arc<Mutex<CommandContext>>,
    ide_mode: bool,
}

fn send_output<T: io::Write + Send + 'static>(
    channel: mpsc::Receiver<String>,
    mut output: T,
    color: Option<Color>,
) {
    std::thread::spawn(move || {
        while let Ok(line) = channel.recv() {
            let status = if let Some(color) = color {
                writeln!(output, "{}", line.color(color))
            } else {
                writeln!(output, "{}", line)
            };
            if status.is_err() {
                break;
            }
        }
    });
}

impl Repl {
    fn new(ide_mode: bool) -> Result<Repl, Error> {
        let (command_context, outputs) = CommandContext::new()?;
        send_output(outputs.stdout, io::stdout(), None);
        send_output(outputs.stderr, io::stderr(), Some(Color::BrightRed));
        let mut repl = Repl {
            command_context: Arc::new(Mutex::new(command_context)),
            ide_mode,
        };
        repl.execute(":load_config");
        Ok(repl)
    }

    fn execute(&mut self, to_run: &str) {
        let execution_result = self.command_context.lock().unwrap().execute(to_run);
        let success = match execution_result {
            Ok(output) => {
                if let Some(text) = output.get("text/plain") {
                    println!("{}", text);
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
                eprintln!("{}", format!("{}", err).bright_red());
                false
            }
        };

        if self.ide_mode {
            let success_marker = if success { "\u{0091}" } else { "\u{0092}" };
            print!("{}", success_marker);
        }
    }

    fn display_errors(&mut self, source: &str, errors: Vec<CompilationError>) {
        let source_lines: Vec<&str> = source.lines().collect();
        let mut last_span_lines: &Vec<String> = &vec![];
        for error in &errors {
            if error.is_from_user_code() {
                for spanned_message in error.spanned_messages() {
                    if let Some(span) = &spanned_message.span {
                        let start_column;
                        let end_column;
                        if source_lines.len() > 1 {
                            // for multi line source code, print the lines
                            if last_span_lines != &spanned_message.lines {
                                for line in &spanned_message.lines {
                                    println!("{}", line);
                                }
                            }
                            last_span_lines = &spanned_message.lines;
                            start_column = span.output_start_column;
                            end_column = span.output_end_column;
                        } else {
                            print!("{}", " ".repeat(PROMPT.len()));
                            start_column = span.start_column;
                            end_column = span.end_column;
                        }
                        print!("{}", " ".repeat(start_column - 1));

                        // considering spans can cover multiple lines,
                        // it could be that end_column is less than start_column.
                        let span_diff = if start_column < end_column {
                            end_column - start_column
                        } else {
                            start_column - end_column
                        };
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
                    println!("{} {}", "help:".bold(), help);
                }
                if let Some(extra_hint) = error.evcxr_extra_hint() {
                    println!("{}", extra_hint);
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

#[derive(StructOpt, Debug)]
#[structopt(name = "evcxr")]
struct Options {
    #[structopt(long)]
    disable_readline: bool,
    #[structopt(long)]
    ide_mode: bool,
    /// Optimization level (0, 1 or 2)
    #[structopt(long, default_value = "")]
    opt: String,
    #[structopt(
        long,
        parse(try_from_str = parse_edit_mode),
        possible_values = &["vi", "emacs"],
        default_value = "emacs"
     )]
    edit_mode: rustyline::EditMode,
}

fn main() {
    evcxr::runtime_hook();

    let options = Options::from_args();

    #[cfg(windows)]
    colored::control::set_virtual_terminal(true).ok();

    println!("Welcome to evcxr. For help, type :help");
    let mut repl = match Repl::new(options.ide_mode) {
        Ok(c) => c,
        Err(error) => {
            eprintln!("{}", error);
            return;
        }
    };

    repl.command_context
        .lock()
        .unwrap()
        .set_opt_level(&options.opt)
        .ok();
    let config = match options.edit_mode {
        EditMode::Vi => rustyline::Config::builder()
            .edit_mode(EditMode::Vi)
            .keyseq_timeout(0) // https://github.com/kkawakam/rustyline/issues/371
            .build(),
        _ => rustyline::Config::default(), // default edit_mode is emacs
    };
    let mut editor = Editor::<EvcxrRustylineHelper>::with_config(config);
    editor.bind_sequence(
        KeyPress::ControlLeft,
        Cmd::Move(Movement::BackwardWord(1, Word::Big)),
    );
    editor.bind_sequence(
        KeyPress::ControlRight,
        Cmd::Move(Movement::ForwardWord(1, At::AfterEnd, Word::Big)),
    );
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
    loop {
        let prompt = format!("{}", PROMPT.yellow());
        let readline = if options.disable_readline {
            readline_direct(&prompt)
        } else {
            editor.readline(PROMPT)
        };
        match readline {
            Ok(line) => {
                editor.add_history_entry(line.clone());
                repl.execute(&line);
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }
    if let Some(history_file) = &opt_history_file {
        editor.save_history(&history_file).ok();
    }
}

fn parse_edit_mode(src: &str) -> Result<EditMode, &str> {
    let mode = match src {
        "vi" => EditMode::Vi,
        "emacs" => EditMode::Emacs,
        _ => return Err("only 'vi' and 'emacs' are supported"),
    };
    Ok(mode)
}
