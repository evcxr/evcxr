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

use crate::errors::Error;
use crate::runtime;
use std;
use std::io::BufReader;
use std::process;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

pub(crate) struct ChildProcess {
    process: std::process::Child,
    stdout: std::io::Lines<BufReader<std::process::ChildStdout>>,
    // Only none while in drop.
    stdin: Option<std::process::ChildStdin>,
    command: Arc<Mutex<process::Command>>,
    stderr_sender: Arc<Mutex<mpsc::Sender<String>>>,
}

impl ChildProcess {
    pub(crate) fn new(
        mut command: std::process::Command,
        stderr_sender: mpsc::Sender<String>,
    ) -> Result<ChildProcess, Error> {
        command
            .env(runtime::EVCXR_IS_RUNTIME_VAR, "1")
            .env("RUST_BACKTRACE", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        ChildProcess::new_internal(
            Arc::new(Mutex::new(command)),
            Arc::new(Mutex::new(stderr_sender)),
        )
    }
    fn new_internal(
        command: Arc<Mutex<std::process::Command>>,
        stderr_sender: Arc<Mutex<mpsc::Sender<String>>>,
    ) -> Result<ChildProcess, Error> {
        let process = command.lock().unwrap().spawn();
        let mut process = match process {
            Ok(c) => c,
            Err(error) => bail!("Failed to run '{:?}': {:?}", command, error),
        };

        let stdout = std::io::BufRead::lines(BufReader::new(process.stdout.take().unwrap()));

        // Handle stderr by patching it through to a channel in our output struct.
        let mut child_stderr =
            std::io::BufRead::lines(BufReader::new(process.stderr.take().unwrap()));
        std::thread::spawn({
            let stderr_sender = Arc::clone(&stderr_sender);
            move || {
                let stderr_sender = stderr_sender.lock().unwrap();
                while let Some(Ok(line)) = child_stderr.next() {
                    // Ignore errors, since it just means that the user of the library has dropped the receive end.
                    let _ = stderr_sender.send(line);
                }
            }
        });

        let stdin = process.stdin.take();
        Ok(ChildProcess {
            process,
            stdout,
            stdin,
            command,
            stderr_sender,
        })
    }

    /// Terminates this process if it hasn't already, then restarts
    pub(crate) fn restart(&mut self) -> Result<ChildProcess, Error> {
        // If the process hasn't already terminated for some reason, kill it.
        if let Ok(None) = self.process.try_wait() {
            let _ = self.process.kill();
            let _ = self.process.wait();
        }
        ChildProcess::new_internal(Arc::clone(&self.command), Arc::clone(&self.stderr_sender))
    }

    pub(crate) fn send(&mut self, command: &str) -> Result<(), Error> {
        use std::io::Write;
        if writeln!(self.stdin.as_mut().unwrap(), "{}", command).is_err() {
            return Err(self.get_termination_error());
        }
        self.stdin.as_mut().unwrap().flush()?;
        Ok(())
    }

    pub(crate) fn recv_line(&mut self) -> Result<String, Error> {
        Ok(self
            .stdout
            .next()
            .ok_or_else(|| self.get_termination_error())??)
    }

    fn get_termination_error(&mut self) -> Error {
        // Wait until the stderr handling thread has released its lock on
        // stderr_sender, which it will do when there's nothing more to read
        // from stderr.
        let _ = self.stderr_sender.lock().unwrap();
        let mut content = String::new();
        while let Some(Ok(line)) = self.stdout.next() {
            content.push_str(&line);
            content.push('\n');
        }
        Error::ChildProcessTerminated(match self.process.wait() {
            Ok(exit_status) => format!(
                "{}Child process terminated with status: {}",
                content, exit_status
            ),
            Err(wait_error) => format!("Child process didn't start: {}", wait_error),
        })
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        // Drop child_stdin before we wait. Our child process uses stdin being
        // closed to know that it's time to terminate.
        self.stdin.take();
        // Wait for our child process to terminate. Otherwise we'll be left with
        // zombie processes.
        let _ = self.process.wait();
    }
}
