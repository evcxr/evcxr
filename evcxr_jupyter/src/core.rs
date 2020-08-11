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

use crate::connection::Connection;
use crate::control_file;
use crate::jupyter_message::JupyterMessage;
use colored::*;
use evcxr;
use evcxr::CommandContext;
use failure::Error;
use json;
use json::JsonValue;
use std;
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time;
use zmq;

// Note, to avoid potential deadlocks, each thread should lock at most one mutex at a time.
#[derive(Clone)]
pub(crate) struct Server {
    iopub: Arc<Mutex<Connection>>,
    stdin: Arc<Mutex<Connection>>,
    latest_execution_request: Arc<Mutex<Option<JupyterMessage>>>,
    shutdown_requested_receiver: Arc<Mutex<mpsc::Receiver<()>>>,
    shutdown_requested_sender: Arc<Mutex<mpsc::Sender<()>>>,
}

impl Server {
    pub(crate) fn start(config: &control_file::Control) -> Result<Server, Error> {
        use zmq::SocketType;

        let zmq_context = zmq::Context::new();
        let heartbeat = bind_socket(config, config.hb_port, zmq_context.socket(SocketType::REP)?)?;
        let shell_socket = bind_socket(
            config,
            config.shell_port,
            zmq_context.socket(SocketType::ROUTER)?,
        )?;
        let control_socket = bind_socket(
            config,
            config.control_port,
            zmq_context.socket(SocketType::ROUTER)?,
        )?;
        let stdin_socket = bind_socket(
            config,
            config.stdin_port,
            zmq_context.socket(SocketType::ROUTER)?,
        )?;
        let iopub = Arc::new(Mutex::new(bind_socket(
            config,
            config.iopub_port,
            zmq_context.socket(SocketType::PUB)?,
        )?));

        let (shutdown_requested_sender, shutdown_requested_receiver) = mpsc::channel();

        let server = Server {
            iopub,
            latest_execution_request: Arc::new(Mutex::new(None)),
            stdin: Arc::new(Mutex::new(stdin_socket)),
            shutdown_requested_receiver: Arc::new(Mutex::new(shutdown_requested_receiver)),
            shutdown_requested_sender: Arc::new(Mutex::new(shutdown_requested_sender)),
        };

        let (execution_sender, execution_receiver) = mpsc::channel();
        let (execution_response_sender, execution_response_receiver) = mpsc::channel();

        thread::spawn(move || Self::handle_hb(&heartbeat));
        server.start_thread(move |server: Server| server.handle_control(control_socket));
        server.start_thread(move |server: Server| {
            server.handle_shell(
                shell_socket,
                &execution_sender,
                &execution_response_receiver,
            )
        });
        let (mut context, outputs) = CommandContext::new()?;
        context.execute(":load_config")?;
        server.start_thread(move |server: Server| {
            server.handle_execution_requests(
                context,
                &execution_receiver,
                &execution_response_sender,
            )
        });
        server
            .clone()
            .start_output_pass_through_thread("stdout", outputs.stdout);
        server
            .clone()
            .start_output_pass_through_thread("stderr", outputs.stderr);
        Ok(server)
    }

    pub(crate) fn wait_for_shutdown(&self) {
        self.shutdown_requested_receiver
            .lock()
            .unwrap()
            .recv()
            .unwrap();
    }

    fn signal_shutdown(&self) {
        self.shutdown_requested_sender
            .lock()
            .unwrap()
            .send(())
            .unwrap();
    }

    fn start_thread<F>(&self, body: F)
    where
        F: FnOnce(Server) -> Result<(), Error> + std::marker::Send + 'static,
    {
        let server_clone = self.clone();
        thread::spawn(|| {
            if let Err(error) = body(server_clone) {
                eprintln!("{:?}", error);
            }
        });
    }

    fn handle_hb(connection: &Connection) -> Result<(), Error> {
        let mut message = zmq::Message::new();
        let ping: &[u8] = b"ping";
        loop {
            connection.socket.recv(&mut message, 0)?;
            connection.socket.send(ping, 0)?;
        }
    }

    fn handle_execution_requests(
        self,
        mut context: CommandContext,
        receiver: &mpsc::Receiver<JupyterMessage>,
        execution_reply_sender: &mpsc::Sender<JupyterMessage>,
    ) -> Result<(), Error> {
        let mut execution_count = 1;
        loop {
            let message = receiver.recv()?;

            // If we want this clone to be cheaper, we probably only need the header, not the
            // whole message.
            *self.latest_execution_request.lock().unwrap() = Some(message.clone());
            let src = message.code();
            execution_count += 1;
            message
                .new_message("execute_input")
                .with_content(object! {
                    "execution_count" => execution_count,
                    "code" => src
                })
                .send(&mut *self.iopub.lock().unwrap())?;
            let mut has_error = false;
            let mut callbacks = evcxr::EvalCallbacks {
                input_reader: &|prompt, is_password| {
                    self.request_input(&message, prompt, is_password)
                        .unwrap_or_default()
                },
                ..evcxr::EvalCallbacks::default()
            };
            for code in split_code_and_command(src) {
                // stop execution after the first error
                has_error = has_error
                    || match context.execute_with_callbacks(&code, &mut callbacks) {
                        Ok(output) => {
                            if !output.is_empty() {
                                // Increase the odds that stdout will have been finished being sent. A
                                // less hacky alternative would be to add a print statement, then block
                                // waiting for it.
                                thread::sleep(time::Duration::from_millis(1));
                                let mut data = HashMap::new();
                                // At the time of writing the json crate appears to have a generic From
                                // implementation for a Vec<T> where T implements Into<JsonValue>. It also
                                // has conversion from HashMap<String, JsonValue>, but it doesn't have
                                // conversion from HashMap<String, T>. Perhaps send a PR? For now, we
                                // convert the values manually.
                                for (k, v) in output.content_by_mime_type {
                                    if k.contains("json") {
                                        data.insert(
                                            k,
                                            json::parse(&v).unwrap_or_else(|_| json::from(v)),
                                        );
                                    } else {
                                        data.insert(k, json::from(v));
                                    }
                                }
                                message
                                    .new_message("execute_result")
                                    .with_content(object! {
                                        "execution_count" => execution_count,
                                        "data" => data,
                                        "metadata" => HashMap::new(),
                                    })
                                    .send(&mut *self.iopub.lock().unwrap())?;
                            }
                            if let Some(duration) = output.timing {
                                // TODO replace by duration.as_millis() when stable
                                let ms =
                                    duration.as_secs() * 1000 + u64::from(duration.subsec_millis());
                                let mut data = HashMap::new();
                                data.insert(
                                    "text/html".into(),
                                    json::from(format!(
                                        "<span style=\"color: rgba(0,0,0,0.4);\">Took {}ms</span>",
                                        ms
                                    )),
                                );
                                message
                                    .new_message("execute_result")
                                    .with_content(object! {
                                        "execution_count" => execution_count,
                                        "data" => data,
                                        "metadata" => HashMap::new(),
                                    })
                                    .send(&mut *self.iopub.lock().unwrap())?;
                            }
                            false
                        }
                        Err(errors) => {
                            self.emit_errors(&errors, &message)?;
                            true
                        }
                    };
            }
            let reply = if has_error {
                message.new_reply().with_content(object! {
                    "status" => "error",
                    "execution_count" => execution_count,
                })
            } else {
                message.new_reply().with_content(object! {
                    "status" => "ok",
                    "execution_count" => execution_count
                })
            };
            execution_reply_sender.send(reply)?;
        }
    }

    fn request_input(
        &self,
        current_request: &JupyterMessage,
        prompt: &str,
        password: bool,
    ) -> Option<String> {
        if current_request.get_content()["allow_stdin"].as_bool() != Some(true) {
            return None;
        }
        let mut stdin = self.stdin.lock().unwrap();
        let stdin_request = current_request
            .new_reply()
            .with_message_type("input_request")
            .with_content(object! {
                "prompt" => prompt,
                "password" => password,
            });
        stdin_request.send(&mut *stdin).ok()?;

        let input_response = JupyterMessage::read(&mut *stdin).ok()?;
        input_response.get_content()["value"]
            .as_str()
            .map(|value| value.to_owned())
    }

    fn handle_shell(
        self,
        mut connection: Connection,
        execution_channel: &mpsc::Sender<JupyterMessage>,
        execution_reply_receiver: &mpsc::Receiver<JupyterMessage>,
    ) -> Result<(), Error> {
        loop {
            let message = JupyterMessage::read(&mut connection)?;
            // Processing of every message should be enclosed between "busy" and "idle"
            // see https://jupyter-client.readthedocs.io/en/latest/messaging.html#messages-on-the-shell-router-dealer-channel
            // Jupiter Lab doesn't use the kernel until it received "idle" for kernel_info_request
            message
                .new_message("status")
                .with_content(object! {"execution_state" => "busy"})
                .send(&mut *self.iopub.lock().unwrap())?;
            let idle = message
                .new_message("status")
                .with_content(object! {"execution_state" => "idle"});
            if message.message_type() == "kernel_info_request" {
                message
                    .new_reply()
                    .with_content(kernel_info())
                    .send(&mut connection)?;
            } else if message.message_type() == "is_complete_request" {
                message
                    .new_reply()
                    .with_content(object! {"status" => "complete"})
                    .send(&mut connection)?;
            } else if message.message_type() == "execute_request" {
                execution_channel.send(message)?;
                execution_reply_receiver.recv()?.send(&mut connection)?;
            } else if message.message_type() == "comm_open" {
                message
                    .new_message("comm_close")
                    .with_content(message.get_content().clone())
                    .send(&mut connection)?;
            } else {
                eprintln!(
                    "Got unrecognized message type on shell channel: {}",
                    message.message_type()
                );
            }
            idle.send(&mut *self.iopub.lock().unwrap())?;
        }
    }

    fn handle_control(self, mut connection: Connection) -> Result<(), Error> {
        loop {
            let message = JupyterMessage::read(&mut connection)?;
            match message.message_type() {
                "shutdown_request" => self.signal_shutdown(),
                "interrupt_request" => {
                    message.new_reply().send(&mut connection)?;
                    eprintln!(
                        "Rust doesn't support interrupting execution. Perhaps restart kernel?"
                    );
                }
                _ => {
                    eprintln!(
                        "Got unrecognized message type on control channel: {}",
                        message.message_type()
                    );
                }
            }
        }
    }

    fn start_output_pass_through_thread(
        self,
        output_name: &'static str,
        channel: mpsc::Receiver<String>,
    ) {
        thread::spawn(move || {
            while let Ok(line) = channel.recv() {
                let mut message = None;
                if let Some(exec_request) = &*self.latest_execution_request.lock().unwrap() {
                    message = Some(exec_request.new_message("stream"));
                }
                if let Some(message) = message {
                    if let Err(error) = message
                        .with_content(object! {
                            "name" => output_name,
                            "text" => format!("{}\n", line),
                        })
                        .send(&mut *self.iopub.lock().unwrap())
                    {
                        eprintln!("{}", error);
                    }
                }
            }
        });
    }

    fn emit_errors(
        &self,
        errors: &evcxr::Error,
        parent_message: &JupyterMessage,
    ) -> Result<(), Error> {
        match errors {
            evcxr::Error::CompilationErrors(errors) => {
                for error in errors {
                    let message = format!("{}", error.message().bright_red());
                    if error.is_from_user_code() {
                        let mut traceback = Vec::new();
                        for spanned_message in error.spanned_messages() {
                            for line in &spanned_message.lines {
                                traceback.push(line.clone());
                            }
                            if let Some(span) = &spanned_message.span {
                                let mut carrots = String::new();
                                for _ in 1..span.start_column {
                                    carrots.push(' ');
                                }
                                for _ in span.start_column..span.end_column {
                                    carrots.push('^');
                                }
                                traceback.push(format!(
                                    "{} {}",
                                    carrots.bright_red(),
                                    spanned_message.label.bright_blue()
                                ));
                            } else {
                                traceback.push(spanned_message.label.clone());
                            }
                        }
                        traceback.push(error.message());
                        parent_message
                            .new_message("error")
                            .with_content(object! {
                                "ename" => "Error",
                                "evalue" => error.message(),
                                "traceback" => traceback,
                            })
                            .send(&mut *self.iopub.lock().unwrap())?;
                    } else {
                        parent_message
                            .new_message("error")
                            .with_content(object! {
                                "ename" => "Error",
                                "evalue" => error.message(),
                                "traceback" => array![
                                    message
                                ],
                            })
                            .send(&mut *self.iopub.lock().unwrap())?;
                    }
                }
            }
            error => {
                let displayed_error = format!("{}", error);
                parent_message
                    .new_message("error")
                    .with_content(object! {
                        "ename" => "Error",
                        "evalue" => displayed_error.clone(),
                        "traceback" => array![displayed_error],
                    })
                    .send(&mut *self.iopub.lock().unwrap())?;
            }
        }
        Ok(())
    }
}

fn bind_socket(
    config: &control_file::Control,
    port: u16,
    socket: zmq::Socket,
) -> Result<Connection, Error> {
    let endpoint = format!("{}://{}:{}", config.transport, config.ip, port);
    socket.bind(&endpoint)?;
    Ok(Connection::new(socket, &config.key)?)
}

/// See [Kernel info documentation](https://jupyter-client.readthedocs.io/en/stable/messaging.html#kernel-info)
fn kernel_info() -> JsonValue {
    object! {
        "protocol_version" => "5.3",
        "implementation" => env!("CARGO_PKG_NAME"),
        "implementation_version" => env!("CARGO_PKG_VERSION"),
        "language_info" => object!{
            "name" => "Rust",
            "version" => "",
            "mimetype" => "text/rust",
            "file_extension" => ".rs",
            // Pygments lexer, for highlighting Only needed if it differs from the 'name' field.
            // see http://pygments.org/docs/lexers/#lexers-for-the-rust-language
            "pygment_lexer" => "rust",
            // Codemirror mode, for for highlighting in the notebook. Only needed if it differs from the 'name' field.
            // codemirror use text/x-rustsrc as mimetypes
            // see https://codemirror.net/mode/rust/
            "codemirror_mode" => "rust",
        },
        "banner" => format!("EvCxR {} - Evaluation Context for Rust", env!("CARGO_PKG_VERSION")),
        "help_links" => array![
            object!{"text" => "Rust std docs",
                    "url" => "https://doc.rust-lang.org/stable/std/"}
        ],
        "status" => "ok"
    }
}

//TODO optimize by avoiding creation of new String
fn split_code_and_command(src: &str) -> Vec<String> {
    src.lines().fold(vec![], |mut acc, l| {
        if l.starts_with(':') {
            acc.push(l.to_owned());
        } else if let Some(last) = acc.pop() {
            if !last.starts_with(':') {
                acc.push(last + "\n" + l);
            } else {
                acc.push(last);
                acc.push(l.to_owned());
            }
        } else {
            acc.push(l.to_owned());
        }
        acc
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMAND_0: &str = r#":dep foo= "0.3.3""#;
    const CODE_0: &str = r#"println!(":dep code 0");"#;
    const CODE_1: &str = r#"
        println!('hello');
        eprintln!('world');
    "#;

    #[test]
    fn split_code_and_command_test_single_command() {
        let expected = vec![COMMAND_0.to_owned()];
        let actual = split_code_and_command(&expected.join("\n"));
        assert_eq!(actual, expected);
    }

    #[test]
    fn split_code_and_command_test_single_code() {
        let expected = vec![CODE_1.to_owned()];
        let actual = split_code_and_command(&expected.join("\n"));
        assert_eq!(actual, expected);
    }

    #[test]
    fn split_code_and_command_test_multi_command() {
        let expected = vec![COMMAND_0.to_owned(), COMMAND_0.to_owned()];
        let actual = split_code_and_command(&expected.join("\n"));
        assert_eq!(actual, expected);
    }

    #[test]
    fn split_code_and_command_test_mixed_command_code() {
        let expected = vec![
            COMMAND_0.to_owned(),
            COMMAND_0.to_owned(),
            CODE_0.to_owned(),
            COMMAND_0.to_owned(),
            CODE_1.to_owned(),
        ];
        let actual = split_code_and_command(&expected.join("\n"));
        assert_eq!(actual, expected);
    }
}
