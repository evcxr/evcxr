use std::fmt::Display;
use std::path::PathBuf;

/// An artifact emitted by Rustc.
pub(super) struct Artifact {
    pub(super) path: PathBuf,
    pub(super) emit: String,
}

pub(super) fn read_artifacts(input: &str) -> Vec<Artifact> {
    let mut artifacts = Vec::new();
    for line in input.lines() {
        let Ok(entry) = json::parse(line) else {
            continue;
        };
        let Some(path) = entry["artifact"].as_str().map(PathBuf::from) else {
            continue;
        };
        let Some(emit) = entry["emit"].as_str() else {
            continue;
        };
        artifacts.push(Artifact {
            path,
            emit: emit.to_owned(),
        });
    }
    artifacts
}

impl Display for Artifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let obj = json::object! {
            artifact: self.path.display().to_string(),
            emit: self.emit.clone(),
        };
        obj.fmt(f)
    }
}
