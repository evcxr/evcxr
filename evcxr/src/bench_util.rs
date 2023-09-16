use std::io::BufRead;
use std::io::Read;

use crate::errors::bail;
use crate::errors::Error;
use crate::eval_context::Config;
use crate::module::tee_error_line;
use crate::module::{create_dir, write_file};
use tl::{Bytes, Node, NodeHandle};

pub(crate) struct BenchmarkObj(pub(crate) Config);

impl BenchmarkObj {
    fn crate_all_files(&self, input: &str) -> Result<(), Error> {
        self.create_cargo_toml()?;
        self.create_benches_dir()?;
        self.create_lib(input)?;
        Ok(())
    }

    fn create_cargo_toml(&self) -> Result<(), Error> {
        let top_dir = self.0.crate_dir().join("bench");
        write_file(top_dir.as_path(), "Cargo.toml", BENCHMARK_TOML)
    }

    fn create_benches_dir(&self) -> Result<(), Error> {
        let top_dir = self.0.crate_dir().join("bench").join("benches");
        create_dir(&top_dir)?;
        write_file(top_dir.as_path(), "my_benchmark.rs", MY_BENCHMARK_CONTENTS)?;
        Ok(())
    }

    fn create_lib(&self, input: &str) -> Result<(), Error> {
        let top_dir = self.0.crate_dir().join("bench").join("src");
        create_dir(&top_dir)?;
        write_file(top_dir.as_path(), "lib.rs", input)
    }

    fn run_with_input(&self, input: &str) -> Result<std::process::Output, Error> {
        self.crate_all_files(input)?;
        let mut command = self.0.cargo_command("bench");
        command.current_dir(self.0.crate_dir().join("bench"));
        let mb_child = command
            .stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn();
        let mut child = match mb_child {
            Ok(out) => out,
            Err(err) => bail!("Error running 'cargo rustc': {}", err),
        };
        // Collect stdout in a parallel thread
        let mut stdout = child.stdout.take().unwrap();
        let output_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            stdout.read_to_end(&mut buf)?;
            Ok::<_, Error>(buf)
        });

        // Collect stderr synchronously
        let stderr = std::io::BufReader::new(child.stderr.take().unwrap());
        let mut all_errors = Vec::new();
        for mb_line in stderr.split(10) {
            let mut line = mb_line?;
            tee_error_line(&line);
            all_errors.append(&mut line);
            all_errors.push(10);
        }

        let status = child.wait()?;
        let all_output = output_thread.join().expect("Panic in child thread")?;

        let cargo_output = std::process::Output {
            status,
            stdout: all_output,
            stderr: all_errors,
        };
        if cargo_output.status.success() {
            Ok(cargo_output)
        } else {
            let zz = String::from_utf8_lossy(&cargo_output.stderr);
            bail!(Error::Message(zz.to_string()));
        }
    }

    fn get_result(&self) -> Result<String, Error> {
        let top_path = self
            .0
            .crate_dir()
            .join("bench")
            .join("target/criterion/benchmark_fn/report");
        let index = std::fs::read_to_string(top_path.join("index.html"))?;
        let mut dom = tl::parse(&index, tl::ParserOptions::default())
            .map_err(|_| Error::Message("dom parse wrong".to_owned()))?;
        let anchors: Vec<NodeHandle> = dom
            .query_selector("a[href]")
            .ok_or("bench error")?
            .collect();
        for anchor in anchors.iter() {
            let parser_mut = dom.parser_mut();
            let anchor = anchor.get_mut(parser_mut).unwrap();
            let svg_name = anchor
                .as_tag()
                .ok_or("bench error")?
                .attributes()
                .get("href")
                .ok_or("bench error")?
                .ok_or("bench error")?
                .as_utf8_str()
                .to_string();
            if svg_name.contains("http") || EXCLUE_SVG.contains(&svg_name.as_str()) {
                continue;
            }
            let svg_content = std::fs::read_to_string(top_path.join(&svg_name))?;
            let svg_byte =
                Bytes::try_from(svg_content).map_err(|_| "svg content wrong".to_owned())?;
            *anchor = Node::Raw(svg_byte);
        }
        Ok(dom.outer_html().to_string())
    }

    pub(crate) fn run_then_get(&self, input: &str) -> Result<(String, String), Error> {
        let command_output = self.run_with_input(input)?;
        let command_output_string = String::from_utf8_lossy(&command_output.stdout).to_string();
        let report_output_string = self.get_result()?;
        Ok((command_output_string, report_output_string))
    }
}

const BENCHMARK_TOML: &str = r#"
[package]
name = "bench"
version = "0.1.0"
edition = "2021"

# See more keys and their definitions at https://doc.rust-lang.org/cargo/reference/manifest.html

[dependencies]
jemalloc-ctl = "0.5.4"
jemallocator = "0.5.4"


[dev-dependencies]
criterion = "0.5.1"

[[bench]]
name = "my_benchmark"
harness = false
"#;

const MY_BENCHMARK_CONTENTS: &str = r#"
use criterion::{criterion_group, criterion_main, Criterion};
use bench::benchmark_fn;

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("benchmark_fn", |b| b.iter(benchmark_fn));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
"#;

const EXCLUE_SVG: [&str; 9] = [
    "typical.svg",
    "mean.svg",
    "SD.svg",
    "median.svg",
    "MAD.svg",
    "slope.svg",
    "change/mean.svg",
    "change/median.svg",
    "change/t-test.svg",
];
