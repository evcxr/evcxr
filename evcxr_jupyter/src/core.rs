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
use anyhow::bail;
use anyhow::Result;
use colored::*;
use crossbeam_channel::Select;
use evcxr::CommandContext;
use json::JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time;
use std::time::Duration;

// Note, to avoid potential deadlocks, each thread should lock at most one mutex at a time.
#[derive(Clone)]
pub(crate) struct Server {
    iopub: Arc<Mutex<Connection>>,
    stdin: Arc<Mutex<Connection>>,
    latest_execution_request: Arc<Mutex<Option<JupyterMessage>>>,
    shutdown_requested_receiver: Arc<Mutex<crossbeam_channel::Receiver<()>>>,
    shutdown_requested_sender: Arc<Mutex<crossbeam_channel::Sender<()>>>,
}

impl Server {
    pub(crate) fn start(config: &control_file::Control) -> Result<Server> {
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

        let (shutdown_requested_sender, shutdown_requested_receiver) =
            crossbeam_channel::unbounded();

        let server = Server {
            iopub,
            latest_execution_request: Arc::new(Mutex::new(None)),
            stdin: Arc::new(Mutex::new(stdin_socket)),
            shutdown_requested_receiver: Arc::new(Mutex::new(shutdown_requested_receiver)),
            shutdown_requested_sender: Arc::new(Mutex::new(shutdown_requested_sender)),
        };

        let (execution_sender, execution_receiver) = crossbeam_channel::unbounded();
        let (execution_response_sender, execution_response_receiver) =
            crossbeam_channel::unbounded();

        thread::spawn(move || Self::handle_hb(&heartbeat));
        server.start_thread(move |server: Server| server.handle_control(control_socket));
        let (mut context, outputs) = CommandContext::new()?;
        context.execute(":load_config")?;
        let context = Arc::new(Mutex::new(context));
        server.start_thread({
            let context = Arc::clone(&context);
            move |server: Server| {
                server.handle_shell(
                    shell_socket,
                    &execution_sender,
                    &execution_response_receiver,
                    context,
                )
            }
        });
        server.start_thread(move |server: Server| {
            server.handle_execution_requests(
                context,
                &execution_receiver,
                &execution_response_sender,
            )
        });
        server.clone().start_output_pass_through_thread(vec![
            ("stdout", outputs.stdout),
            ("stderr", outputs.stderr),
        ]);
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
        F: FnOnce(Server) -> Result<()> + std::marker::Send + 'static,
    {
        let server_clone = self.clone();
        thread::spawn(|| {
            if let Err(error) = body(server_clone) {
                eprintln!("{:?}", error);
            }
        });
    }

    fn handle_hb(connection: &Connection) -> Result<()> {
        let mut message = zmq::Message::new();
        let ping: &[u8] = b"ping";
        loop {
            connection.socket.recv(&mut message, 0)?;
            connection.socket.send(ping, 0)?;
        }
    }

    fn handle_execution_requests(
        self,
        context: Arc<Mutex<CommandContext>>,
        receiver: &crossbeam_channel::Receiver<JupyterMessage>,
        execution_reply_sender: &crossbeam_channel::Sender<JupyterMessage>,
    ) -> Result<()> {
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
                .send(&self.iopub.lock().unwrap())?;
            let mut callbacks = evcxr::EvalCallbacks {
                input_reader: &|prompt, is_password| {
                    self.request_input(&message, prompt, is_password)
                        .unwrap_or_default()
                },
            };

            #[allow(unknown_lints, clippy::significant_drop_in_scrutinee)]
            match context
                .lock()
                .unwrap()
                .execute_with_callbacks(src, &mut callbacks)
            {
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
                                data.insert(k, json::parse(&v).unwrap_or_else(|_| json::from(v)));
                            } else {
                                data.insert(k, json::from(v));
                            }
                        }
                        message
                            .new_message("execute_result")
                            .with_content(object! {
                                "execution_count" => execution_count,
                                "data" => data,
                                "metadata" => object!(),
                            })
                            .send(&self.iopub.lock().unwrap())?;
                    }
                    if let Some(duration) = output.timing {
                        // TODO replace by duration.as_millis() when stable
                        let ms = duration.as_secs() * 1000 + u64::from(duration.subsec_millis());
                        let mut data: HashMap<String, JsonValue> = HashMap::new();
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
                                "metadata" => object!(),
                            })
                            .send(&self.iopub.lock().unwrap())?;
                    }
                    execution_reply_sender.send(message.new_reply().with_content(object! {
                        "status" => "ok",
                        "execution_count" => execution_count,
                    }))?;
                }
                Err(errors) => {
                    self.emit_errors(&errors, &message)?;
                    execution_reply_sender.send(message.new_reply().with_content(object! {
                        "status" => "error",
                        "execution_count" => execution_count
                    }))?;
                }
            };
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
        let stdin = self.stdin.lock().unwrap();
        let stdin_request = current_request
            .new_reply()
            .with_message_type("input_request")
            .with_content(object! {
                "prompt" => prompt,
                "password" => password,
            });
        stdin_request.send(&stdin).ok()?;

        let input_response = JupyterMessage::read(&stdin).ok()?;
        input_response.get_content()["value"]
            .as_str()
            .map(|value| value.to_owned())
    }

    fn handle_shell(
        self,
        connection: Connection,
        execution_channel: &crossbeam_channel::Sender<JupyterMessage>,
        execution_reply_receiver: &crossbeam_channel::Receiver<JupyterMessage>,
        context: Arc<Mutex<CommandContext>>,
    ) -> Result<()> {
        loop {
            let message = JupyterMessage::read(&connection)?;
            self.handle_shell_message(
                message,
                &connection,
                execution_channel,
                execution_reply_receiver,
                &context,
            )?;
        }
    }

    fn handle_shell_message(
        &self,
        message: JupyterMessage,
        connection: &Connection,
        execution_channel: &crossbeam_channel::Sender<JupyterMessage>,
        execution_reply_receiver: &crossbeam_channel::Receiver<JupyterMessage>,
        context: &Arc<Mutex<CommandContext>>,
    ) -> Result<()> {
        // Processing of every message should be enclosed between "busy" and "idle"
        // see https://jupyter-client.readthedocs.io/en/latest/messaging.html#messages-on-the-shell-router-dealer-channel
        // Jupiter Lab doesn't use the kernel until it received "idle" for kernel_info_request
        message
            .new_message("status")
            .with_content(object! {"execution_state" => "busy"})
            .send(&self.iopub.lock().unwrap())?;
        let idle = message
            .new_message("status")
            .with_content(object! {"execution_state" => "idle"});
        if message.message_type() == "kernel_info_request" {
            message
                .new_reply()
                .with_content(kernel_info())
                .send(connection)?;
        } else if message.message_type() == "is_complete_request" {
            message
                .new_reply()
                .with_content(object! {"status" => "complete"})
                .send(connection)?;
        } else if message.message_type() == "execute_request" {
            execution_channel.send(message)?;
            execution_reply_receiver.recv()?.send(connection)?;
        } else if message.message_type() == "comm_open" {
            comm_open(message, context, Arc::clone(&self.iopub))?;
        } else if message.message_type() == "comm_msg"
            || message.message_type() == "comm_info_request"
        {
            // We don't handle this yet.
        } else if message.message_type() == "complete_request" {
            let reply = message.new_reply().with_content(
                match handle_completion_request(context, message) {
                    Ok(response_content) => response_content,
                    Err(error) => object! {
                        "status" => "error",
                        "ename" => error.to_string(),
                        "evalue" => "",
                    },
                },
            );
            reply.send(connection)?;
        } else {
            eprintln!(
                "Got unrecognized message type on shell channel: {}",
                message.message_type()
            );
        }
        idle.send(&self.iopub.lock().unwrap())?;
        Ok(())
    }

    fn handle_control(self, connection: Connection) -> Result<()> {
        loop {
            let message = JupyterMessage::read(&connection)?;
            match message.message_type() {
                "shutdown_request" => self.signal_shutdown(),
                "interrupt_request" => {
                    message.new_reply().send(&connection)?;
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
        channels: Vec<(&'static str, crossbeam_channel::Receiver<String>)>,
    ) {
        thread::spawn(move || {
            let mut select = Select::new();
            for (_, channel) in &channels {
                select.recv(channel);
            }
            loop {
                let index = select.ready();
                let (output_name, channel) = &channels[index];
                // Read from the channel that has output until it has been idle
                // for 1ms before we return to checking other channels. This
                // reduces the extent to which outputs interleave. e.g. a
                // multi-line print is performed to stderr, then another to
                // stdout - we can't guarantee the order in which they get sent,
                // but we'd like to try to make sure that we don't interleave
                // their lines if possible.
                while let Ok(line) = channel.recv_timeout(Duration::from_millis(1)) {
                    self.pass_output_line(output_name, line);
                }
            }
        });
    }

    fn pass_output_line(&self, output_name: &'static str, line: String) {
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
                .send(&self.iopub.lock().unwrap())
            {
                eprintln!("{}", error);
            }
        }
    }

    fn emit_errors(&self, errors: &evcxr::Error, parent_message: &JupyterMessage) -> Result<()> {
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
                        for help in error.help() {
                            traceback.push(format!("{}: {}", "help".bold(), help));
                        }
                        parent_message
                            .new_message("error")
                            .with_content(object! {
                                "ename" => "Error",
                                "evalue" => error.message(),
                                "traceback" => traceback,
                            })
                            .send(&self.iopub.lock().unwrap())?;
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
                            .send(&self.iopub.lock().unwrap())?;
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
                    .send(&self.iopub.lock().unwrap())?;
            }
        }
        Ok(())
    }
}

fn comm_open(
    message: JupyterMessage,
    context: &Arc<Mutex<CommandContext>>,
    iopub: Arc<Mutex<Connection>>,
) -> Result<()> {
    if message.target_name() == "evcxr-cargo-check" {
        let context = Arc::clone(context);
        std::thread::spawn(move || {
            if let Some(code) = message.data()["code"].as_str() {
                let data = cargo_check(code, &context);
                let response_content = object! {
                    "comm_id" => message.comm_id(),
                    "data" => data,
                };
                message
                    .new_message("comm_msg")
                    .without_parent_header()
                    .with_content(response_content)
                    .send(&iopub.lock().unwrap())
                    .unwrap();
            }
            message
                .comm_close_message()
                .send(&iopub.lock().unwrap())
                .unwrap();
        });
        Ok(())
    } else {
        // Unrecognised comm target, just close the comm.
        message.comm_close_message().send(&iopub.lock().unwrap())
    }
}

fn cargo_check(code: &str, context: &Mutex<CommandContext>) -> JsonValue {
    let problems = context.lock().unwrap().check(code).unwrap_or_default();
    let problems_json: Vec<JsonValue> = problems
        .iter()
        .filter_map(|problem| {
            if let Some(primary_spanned_message) = problem.primary_spanned_message() {
                if let Some(span) = primary_spanned_message.span {
                    use std::fmt::Write;
                    let mut message = primary_spanned_message.label.clone();
                    if !message.is_empty() {
                        message.push('\n');
                    }
                    message.push_str(&problem.message());
                    for help in problem.help() {
                        write!(message, "\nhelp: {}", help).unwrap();
                    }
                    return Some(object! {
                        "message" => message,
                        "severity" => problem.level(),
                        "start_line" => span.start_line,
                        "start_column" => span.start_column,
                        "end_column" => span.end_column,
                        "end_line" => span.end_line,
                    });
                }
            }
            None
        })
        .collect();
    object! {
        "problems" => problems_json,
    }
}

fn bind_socket(
    config: &control_file::Control,
    port: u16,
    socket: zmq::Socket,
) -> Result<Connection> {
    let endpoint = format!("{}://{}:{}", config.transport, config.ip, port);
    socket.bind(&endpoint)?;
    Connection::new(socket, &config.key)
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

fn handle_completion_request(
    context: &Mutex<CommandContext>,
    message: JupyterMessage,
) -> Result<JsonValue> {
    let code = message.code();
    let completions = context.lock().unwrap().completions(
        code,
        grapheme_offset_to_byte_offset(code, message.cursor_pos()),
    )?;
    let matches: Vec<String> = completions
        .completions
        .into_iter()
        .map(|completion| completion.code)
        .collect();
    Ok(object! {
        "status" => "ok",
        "matches" => matches,
        "cursor_start" => byte_offset_to_grapheme_offset(code, completions.start_offset)?,
        "cursor_end" => byte_offset_to_grapheme_offset(code, completions.end_offset)?,
        "metadata" => object!{},
    })
}

/// Returns the byte offset for the start of the specified grapheme. Any grapheme beyond the last
/// grapheme will return the end position of the input.
fn grapheme_offset_to_byte_offset(code: &str, grapheme_offset: usize) -> usize {
    unicode_segmentation::UnicodeSegmentation::grapheme_indices(code, true)
        .nth(grapheme_offset)
        .map(|(byte_offset, _)| byte_offset)
        .unwrap_or_else(|| code.len())
}

/// Returns the grapheme offset of the grapheme that starts at
fn byte_offset_to_grapheme_offset(code: &str, target_byte_offset: usize) -> Result<usize> {
    let mut grapheme_offset = 0;
    for (byte_offset, _) in unicode_segmentation::UnicodeSegmentation::grapheme_indices(code, true)
    {
        if byte_offset == target_byte_offset {
            break;
        }
        if byte_offset > target_byte_offset {
            bail!(
                "Byte offset {} is not on a grapheme boundary in '{}'",
                target_byte_offset,
                code
            );
        }
        grapheme_offset += 1;
    }
    Ok(grapheme_offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grapheme_offsets() {
        let src = "a̐éx";
        assert_eq!(grapheme_offset_to_byte_offset(src, 0), 0);
        assert_eq!(grapheme_offset_to_byte_offset(src, 1), 3);
        assert_eq!(grapheme_offset_to_byte_offset(src, 2), 6);
        assert_eq!(grapheme_offset_to_byte_offset(src, 3), 7);

        assert_eq!(byte_offset_to_grapheme_offset(src, 0).unwrap(), 0);
        assert_eq!(byte_offset_to_grapheme_offset(src, 3).unwrap(), 1);
        assert_eq!(byte_offset_to_grapheme_offset(src, 6).unwrap(), 2);
        assert_eq!(byte_offset_to_grapheme_offset(src, 7).unwrap(), 3);
    }
}
