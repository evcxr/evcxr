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
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
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
        let obj = serde_json::json!({
            "artifact": self.path.display().to_string(),
            "emit": self.emit,
        });
        obj.fmt(f)
    }
}
