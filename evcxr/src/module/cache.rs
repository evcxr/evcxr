use super::artifacts::Artifact;
use super::artifacts::read_artifacts;
use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use std::borrow::Cow;
use std::fmt::Display;
use std::fmt::Write;
use std::hash::Hasher;
use std::path::PathBuf;
use std::process::Command;
use std::time::SystemTime;

pub(crate) const TARGET_DIR_ENV: &str = "EVCXR_TARGET_DIR";

pub(crate) enum CacheResult {
    /// We got a cache hit. The cache result has been written to appropriate output location.
    Hit,
    /// We got a cache miss. If compilation succeeds
    Miss(CacheMiss),
    /// The current compiler invocation can't be cached.
    NonCache,
}

pub(crate) struct CacheMiss {
    cache_subdirectory: PathBuf,
    output_directory: PathBuf,
    meta: String,
}

/// The values of environment variables relevant to caching.
pub(crate) struct CacheEnv {
    target_dir: String,
}

/// Checks arguments to `command`, produces a cache key then checks to see if it already exists. If
/// it does, copies the cached output to where the actual output is expected. Note, we don't check
/// the rustc version, since we should have the path to the standard library on the command. Using a
/// different version of rustc would require also using a different standard library.
pub(crate) fn access_cache(command: &Command) -> Result<CacheResult> {
    if std::env::var(super::CACHE_ENABLED_ENV).is_err() {
        return Ok(CacheResult::NonCache);
    }
    let cache_dir = cache_directory()?;
    let cache_env = CacheEnv::from_env()?;
    let key = cache_key(command, &cache_env)?;
    let Some(rust_command_line) = RustCommandLine::parse(command) else {
        return Ok(CacheResult::NonCache);
    };
    if rust_command_line.is_incremental {
        // Cargo disables incremental compilation for crates that that it downloads, which are the
        // ones that we want to cache the compilation of. So when incremental compilation is
        // enabled, that means its a local crate, so we disable caching.
        return Ok(CacheResult::NonCache);
    }
    let output_directory = rust_command_line.output_directory;
    let cache_subdirectory = cache_dir.join(key.to_string());
    if let Ok(stderr) = std::fs::read_to_string(cache_subdirectory.join("stderr")) {
        let mut artifacts_out = Vec::new();
        for mut artifact in read_artifacts(&stderr) {
            let Some(filename) = artifact.path.file_name() else {
                continue;
            };
            let cache_path = cache_subdirectory.join(filename);
            let output_path = output_directory.join(filename);
            std::fs::copy(&cache_path, &output_path).with_context(|| {
                format!(
                    "Failed to copy from cache `{}` -> `{}`",
                    cache_path.display(),
                    output_path.display()
                )
            })?;
            artifact.path = output_path;
            artifacts_out.push(artifact);
        }

        // Update the hit counter file for this entry. The main purpose of this is to track how
        // recently this cache entry was used so that we can clean up the least recently used
        // entries after the build finishes.
        let hits_file = cache_subdirectory.join("hits");
        let mut hits: u32 = std::fs::read_to_string(&hits_file)?.parse()?;
        hits += 1;
        std::fs::write(&hits_file, format!("{hits}"))?;

        // We only emit the artifacts once we've copied all the cache files over, otherwise things
        // break for some reason that isn't obvious.
        for a in artifacts_out {
            eprintln!("{a}");
        }
        return Ok(CacheResult::Hit);
    }
    Ok(CacheResult::Miss(CacheMiss {
        output_directory,
        cache_subdirectory,
        meta: cache_key_inputs(command, &cache_env)?,
    }))
}

fn cache_directory() -> Result<PathBuf> {
    Ok(dirs::cache_dir()
        .ok_or_else(|| anyhow!("Failed to get determine directory"))?
        .join("evcxr"))
}

struct RustCommandLine {
    output_directory: PathBuf,
    is_incremental: bool,
}

impl RustCommandLine {
    fn parse(command: &Command) -> Option<Self> {
        let mut args = command.get_args();
        let mut out_dir = None;
        let mut is_incremental = false;
        while let Some(arg) = args.next() {
            if arg == "--out-dir" {
                out_dir = args.next().map(PathBuf::from);
            }
            if arg == "-C" {
                let Some(next) = args.next() else {
                    break;
                };
                if next.to_string_lossy().starts_with("incremental=") {
                    is_incremental = true;
                }
            }
        }
        if let Some(output_directory) = out_dir {
            return Some(Self {
                output_directory,
                is_incremental,
            });
        }
        None
    }
}

impl CacheEnv {
    fn from_env() -> Result<Self> {
        fn get_env(var_name: &str) -> Result<String> {
            std::env::var(var_name)
                .with_context(|| format!("Failed to get environment variable `{var_name}`"))
        }

        Ok(CacheEnv {
            target_dir: get_env(TARGET_DIR_ENV)?,
        })
    }
}

fn cache_key_inputs(command: &Command, env: &CacheEnv) -> Result<String> {
    let mut out = String::new();
    cache_key_inputs_do(command, env, |input| {
        out.push_str(input);
        out.push('\n');
    })?;
    Ok(out)
}

fn cache_key_inputs_do(
    command: &Command,
    env: &CacheEnv,
    mut callback: impl FnMut(&str),
) -> Result<()> {
    for arg in command.get_args() {
        let mut arg = Cow::Borrowed(
            arg.to_str()
                .ok_or_else(|| anyhow!("Non-UTF8 argument to rustc. Caching not supported."))?,
        );
        if arg.contains(&env.target_dir) {
            arg = Cow::Owned(arg.replace(&env.target_dir, "<target_dir>"));
        }
        (callback)(&arg);
    }
    Ok(())
}

fn cache_key(command: &Command, env: &CacheEnv) -> Result<u64> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cache_key_inputs_do(command, env, |input| {
        std::hash::Hash::hash(input, &mut hasher)
    })?;
    Ok(hasher.finish())
}

impl CacheMiss {
    pub(super) fn update_cache(&self, artifacts: &[Artifact]) -> Result<()> {
        std::fs::create_dir_all(&self.cache_subdirectory).with_context(|| {
            format!(
                "Failed to create cache directory `{}`",
                self.cache_subdirectory.display()
            )
        })?;
        for artifact in artifacts {
            let Some(filename) = artifact.path.file_name() else {
                continue;
            };
            let output_path = self.output_directory.join(filename);
            let cache_path = self.cache_subdirectory.join(filename);
            std::fs::copy(&output_path, &cache_path).with_context(|| {
                format!(
                    "Failed to copy output to cache `{}` -> `{}`",
                    output_path.display(),
                    cache_path.display()
                )
            })?;
        }

        std::fs::write(self.cache_subdirectory.join("meta"), &self.meta)?;
        std::fs::write(self.cache_subdirectory.join("hits"), "0")?;
        let mut stderr = String::new();
        for a in artifacts {
            writeln!(&mut stderr, "{a}").unwrap();
        }
        std::fs::write(self.cache_subdirectory.join("stderr"), stderr)?;
        Ok(())
    }
}

/// Reduces cache usage to <= `cache_bytes`. Returns the number of bytes freed.
pub(crate) fn cleanup(cache_bytes: u64) -> Result<u64> {
    let mut freed = 0;
    let mut entries = read_cache_entries()?;
    let total_size: u64 = entries.iter().map(|e| e.size).sum();
    if total_size <= cache_bytes {
        return Ok(freed);
    }
    entries.sort_by_key(|e| e.last_access);
    entries.reverse();
    let mut to_free = (total_size - cache_bytes) as i64;
    while to_free > 0 {
        let Some(entry) = entries.pop() else {
            break;
        };
        std::fs::remove_dir_all(entry.subdirectory)?;
        to_free -= entry.size as i64;
        freed += entry.size;
    }

    Ok(freed)
}

#[derive(Default)]
pub(crate) struct CacheStats {
    num_entries: u64,
    disk_used: u64,
    num_hits: u64,
}

impl CacheStats {
    pub(crate) fn get() -> Result<Self> {
        let mut result = CacheStats::default();
        for entry in read_cache_entries()? {
            result.num_entries += 1;
            result.disk_used += entry.size;
            if let Some(hits) = std::fs::read_to_string(entry.subdirectory.join("hits"))
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
            {
                result.num_hits += hits;
            }
        }
        Ok(result)
    }
}

impl Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Entries: {}", self.num_entries)?;
        writeln!(f, "Disk used: {} MiB", self.disk_used / 1024 / 1024)?;
        writeln!(f, "Hits: {}", self.num_hits)?;
        Ok(())
    }
}

struct CacheEntry {
    last_access: SystemTime,
    size: u64,
    subdirectory: PathBuf,
}

fn read_cache_entries() -> Result<Vec<CacheEntry>> {
    let mut cache_entries = Vec::new();
    let directory = cache_directory()?;
    if !directory.exists() {
        return Ok(vec![]);
    }
    for entry in directory.read_dir()? {
        let entry = entry?;
        let Ok(metadata) = std::fs::metadata(entry.path().join("hits")) else {
            continue;
        };
        let Ok(last_access) = metadata.modified() else {
            continue;
        };
        let mut size = 0;
        for item in entry.path().read_dir()? {
            let item = item?;
            let Ok(metadata) = item.metadata() else {
                continue;
            };
            size += metadata.len();
        }
        cache_entries.push(CacheEntry {
            last_access,
            size,
            subdirectory: entry.path().to_owned(),
        });
    }
    Ok(cache_entries)
}

#[cfg(test)]
mod tests {
    use super::cache_key;
    use std::path::Path;
    use std::process::Command;

    const SAMPLE_ARGS: &[&str] = &[
        "--crate-name",
        "regex",
        "--edition=2021",
        "/home/david/.cargo/registry/src/index.crates.io-6f17d22bba15001f/regex-1.10.2/src/lib.rs",
        "--error-format=json",
        "--json=diagnostic-rendered-ansi,artifacts,future-incompat",
        "--crate-type",
        "lib",
        "--crate-type",
        "dylib",
        "--emit=dep-info,metadata,link",
        "-C",
        "opt-level=2",
        "-C",
        "embed-bitcode=no",
        "-C",
        "codegen-units=16",
        "-C",
        "debug-assertions=on",
        "--cfg",
        "feature=\"default\"",
        "--cfg",
        "feature=\"perf\"",
        "--cfg",
        "feature=\"perf-backtrack\"",
        "--cfg",
        "feature=\"perf-cache\"",
        "--cfg",
        "feature=\"perf-dfa\"",
        "--cfg",
        "feature=\"perf-inline\"",
        "--cfg",
        "feature=\"perf-literal\"",
        "--cfg",
        "feature=\"perf-onepass\"",
        "--cfg",
        "feature=\"std\"",
        "--cfg",
        "feature=\"unicode\"",
        "--cfg",
        "feature=\"unicode-age\"",
        "--cfg",
        "feature=\"unicode-perl\"",
        "--cfg",
        "feature=\"unicode-script\"",
        "--cfg",
        "feature=\"unicode-segment\"",
        "-C",
        "metadata=7a60aee6f9c9be14",
        "-C",
        "extra-filename=-7a60aee6f9c9be14",
        "-C",
        "rpath",
        "--out-dir",
        "/tmp/.tmpAGPAC4/target/x86_64-unknown-linux-gnu/debug/deps",
        "--target",
        "x86_64-unknown-linux-gnu",
        "-C",
        "linker=/usr/bin/clang-15",
        "-C",
        "strip=debuginfo",
        "-L",
        "dependency=/tmp/.tmpAGPAC4/target/x86_64-unknown-linux-gnu/debug/deps",
        "-L",
        "dependency=/tmp/.tmpAGPAC4/target/debug/deps",
        "--extern",
        "aho_corasick=/tmp/.tmpAGPAC4/target/x86_64-unknown-linux-gnu/debug/deps/libaho_corasick-4147e32b287a933f.so",
        "--extern",
        "memchr=/tmp/.tmpAGPAC4/target/x86_64-unknown-linux-gnu/debug/deps/libmemchr-3ac30149df7a21d7.so",
        "--extern",
        "regex_automata=/tmp/.tmpAGPAC4/target/x86_64-unknown-linux-gnu/debug/deps/libregex_automata-b68475a948dc5cb3.so",
        "--extern",
        "regex_syntax=/tmp/.tmpAGPAC4/target/x86_64-unknown-linux-gnu/debug/deps/libregex_syntax-e85cd106ae7f15b9.so",
        "--cap-lints",
        "allow",
        "-Cprefer-dynamic",
        "--extern",
        "core=/home/david/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-unknown-linux-gnu/lib/libstd-6498d8891e016dca.so",
        "-C",
        "prefer-dynamic",
    ];

    fn replace(command: &Command, replacements: &[(&str, &str)]) -> Command {
        let mut new_command = Command::new(command.get_program());
        for arg in command.get_args() {
            let mut new_arg = arg.to_str().unwrap().to_owned();
            for (from, to) in replacements {
                new_arg = new_arg.replace(from, to);
            }
            new_command.arg(new_arg);
        }
        new_command
    }

    #[test]
    fn test_hash() {
        let mut command = Command::new(
            "/home/david/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rustc",
        );
        command.args(SAMPLE_ARGS);
        let mut cache_env = crate::module::cache::CacheEnv {
            target_dir: "/tmp/.tmpAGPAC4".to_owned(),
        };
        let base = cache_key(&command, &cache_env).unwrap();
        // Changing a crate name should affect the hash.
        assert_ne!(
            cache_key(&replace(&command, &[("memchr=", "mem_chr=")]), &cache_env).unwrap(),
            base
        );
        // Changing the temporary directory should not affect the hash.
        "/tmp/.tmp12345".clone_into(&mut cache_env.target_dir);
        command = replace(&command, &[("tmpAGPAC4", "tmp12345")]);
        assert_eq!(cache_key(&command, &cache_env).unwrap(), base);

        // Check the actual value of the hash. We want this to remain stable between separate runs
        // and even separate builds.
        assert_eq!(base, 12456302748188275193);
    }

    #[test]
    fn test_rustc_command_line() {
        let mut command = Command::new(
            "/home/david/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rustc",
        );
        command.args(SAMPLE_ARGS);
        let rustc_command_line = super::RustCommandLine::parse(&command).unwrap();
        assert_eq!(
            rustc_command_line.output_directory,
            Path::new("/tmp/.tmpAGPAC4/target/x86_64-unknown-linux-gnu/debug/deps")
        );
        assert!(!rustc_command_line.is_incremental);
    }
}
