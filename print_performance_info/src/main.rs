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

use evcxr::EvalContext;
use evcxr::{self};
use std::io;

fn send_output<T: io::Write + Send + 'static>(
    channel: crossbeam_channel::Receiver<String>,
    mut output: T,
) {
    std::thread::spawn(move || {
        while let Ok(line) = channel.recv() {
            if writeln!(output, "{line}").is_err() {
                break;
            }
        }
    });
}

fn has_nightly_compiler() -> bool {
    use std::process;
    match process::Command::new("cargo")
        .arg("+nightly")
        .arg("help")
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null())
        .status()
    {
        Ok(exit_status) => exit_status.success(),
        Err(_) => false,
    }
}

fn main() -> Result<(), evcxr::Error> {
    use std::time::Instant;
    evcxr::runtime_hook();
    if !has_nightly_compiler() {
        println!("print_performance_info: Nightly compiler is required.");
        // Exit with Ok status. We run this from Travis CI and don't want to
        // fail when it gets run on non-nightly configurations.
        return Ok(());
    }
    let (mut ctx, outputs) = EvalContext::new()?;
    send_output(outputs.stderr, io::stdout());
    ctx.set_time_passes(true);
    let mut state = ctx.state();
    state.set_toolchain("nightly");
    ctx.eval_with_state("println!(\"41\");", state)?;
    let start = Instant::now();
    let output = ctx.eval_with_state("println!(\"42\");", ctx.state())?;
    println!("Total eval time: {}ms", start.elapsed().as_millis());
    for phase in output.phases {
        println!("  {}: {}ms", phase.name, phase.duration.as_millis());
    }
    Ok(())
}
