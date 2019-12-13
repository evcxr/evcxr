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

use colored::*;
use evcxr::{CommandContext, CompilationError, Error};
use rustyline::{error::ReadlineError, Editor};
use std::fs;
use std::io;
use std::sync::mpsc;
use structopt::StructOpt;

const PROMPT: &str = ">> ";

struct Repl {
    command_context: CommandContext,
    ide_mode: bool,
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
    fn new(ide_mode: bool) -> Result<Repl, Error> {
        let (command_context, outputs) = CommandContext::new()?;
        send_output(outputs.stdout, io::stdout());
        send_output(outputs.stderr, io::stderr());
        let mut repl = Repl {
            command_context,
            ide_mode,
        };
        repl.execute(":load_config");
        Ok(repl)
    }

    fn execute(&mut self, to_run: &str) {
        let success = match self.command_context.execute(to_run) {
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
                self.display_errors(errors);
                false
            }
            Err(err) => {
                eprintln!("{}", format!("{}", err).bright_red());
                false
            }
        };

        if self.ide_mode {
            let success_marker = if success { "\x01" } else { "\x02" };
            print!("{}", success_marker);
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
}

fn main() {
    evcxr::runtime_hook();

    let options = Options::from_args();

    println!("Welcome to evcxr. For help, type :help");
    let mut repl = match Repl::new(options.ide_mode) {
        Ok(c) => c,
        Err(error) => {
            eprintln!("{}", error);
            return;
        }
    };

    repl.command_context.set_opt_level(&options.opt).ok();

    let mut editor = Editor::<()>::new();
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
            editor.readline(&prompt)
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
