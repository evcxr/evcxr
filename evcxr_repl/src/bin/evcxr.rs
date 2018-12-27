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

extern crate colored;
extern crate dirs;
extern crate evcxr;
extern crate regex;
extern crate rustyline;

use colored::*;
use evcxr::{CommandContext, CompilationError, Error};
use rustyline::{error::ReadlineError, Editor};
use std::fs;
use std::io;
use std::sync::mpsc;

const PROMPT: &str = ">> ";

struct Repl {
    command_context: CommandContext,
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

impl Repl {
    fn new() -> Result<Repl, Error> {
        let (command_context, outputs) = CommandContext::new()?;
        send_output(outputs.stdout, io::stdout());
        send_output(outputs.stderr, io::stderr());
        Ok(Repl { command_context })
    }

    fn execute(&mut self, to_run: &str) {
        match self.command_context.execute(to_run) {
            Ok(output) => {
                if let Some(text) = output.get("text/plain") {
                    println!("{}", text);
                }
                if let Some(duration) = output.timing {
                    // TODO replace by duration.as_millis() when stable
                    let ms = duration.as_secs() * 1000 + u64::from(duration.subsec_millis());
                    println!("{}", format!("Took {}ms", ms).blue());
                }
            }
            Err(evcxr::Error::CompilationErrors(errors)) => {
                self.display_errors(errors);
            }
            Err(evcxr::Error::ChildProcessTerminated(err))
            | Err(evcxr::Error::JustMessage(err)) => eprintln!("{}", err.bright_red()),
        }
    }

    fn display_errors(&mut self, errors: Vec<CompilationError>) {
        for error in errors {
            if error.is_from_user_code() {
                for spanned_message in error.spanned_messages() {
                    if let Some(span) = &spanned_message.span {
                        for _ in 1..span.start_column + PROMPT.len() {
                            print!(" ");
                        }
                        let mut carrots = String::new();
                        for _ in span.start_column..span.end_column {
                            carrots.push('^');
                        }
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
                     Ideally this should't happen. Type :last_error_json to see details.\n{}",
                    error.rendered()
                );
            }
        }
    }
}

fn main() {
    evcxr::runtime_hook();
    println!("Welcome to evcxr. For help, type :help");
    let mut repl = match Repl::new() {
        Ok(c) => c,
        Err(error) => {
            eprintln!("{}", error);
            return;
        }
    };

    let mut editor = Editor::<()>::new();
    let mut opt_history_file = None;
    let config_dir = dirs::config_dir().map(|h| h.join("evcxr"));
    if let Some(config_dir) = &config_dir {
        fs::create_dir_all(config_dir).ok();
        let history_file = config_dir.join("history.txt");
        editor.load_history(&history_file).ok();
        opt_history_file = Some(history_file);
    }
    loop {
        let readline = editor.readline(&PROMPT.yellow());
        match readline {
            Ok(line) => {
                editor.add_history_entry(&line);
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
