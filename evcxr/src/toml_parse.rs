use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use toml::{Table, Value};

use crate::eval_context::Config;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigToml {
    #[serde(default = "EvcxrToml::new")]
    pub evcxr: EvcxrToml,
    #[serde(default = "Default::default")]
    pub dependencies: Table,
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct EvcxrToml {
    pub tmpdir: Option<String>,
    #[serde(default = "default_value::preserve_vars_on_panic")]
    pub preserve_vars_on_panic: bool,
    #[serde(default = "default_value::offline_mode")]
    pub offline_mode: bool,
    pub sccache: Option<String>,
    #[serde(default = "default_value::allow_static_linking")]
    pub allow_static_linking: bool,
    pub prelude: Option<String>,
    #[serde(default = "default_value::opt_level")]
    pub opt_level: String,
}

pub(crate) enum TmpDirVar {
    PathBuf(PathBuf),
    TmpDir(TempDir),
}

impl TmpDirVar {
    pub(crate) fn get_path(&self) -> Result<PathBuf> {
        let mut res = match self {
            TmpDirVar::PathBuf(p) => p.clone(),
            TmpDirVar::TmpDir(p) => PathBuf::from(p.path()),
        };
        if !res.is_absolute() {
            res = std::env::current_dir()?.join(res);
        }
        Ok(res)
    }

    pub(crate) fn get_opt_tmpdir(self) -> Option<TempDir> {
        match self {
            TmpDirVar::PathBuf(_) => None,
            TmpDirVar::TmpDir(p) => Some(p),
        }
    }
}

impl ConfigToml {
    fn new() -> Self {
        Self {
            evcxr: EvcxrToml::new(),
            dependencies: Default::default(),
            source_path: None,
        }
    }

    fn parse_from_path(path: &Path) -> Result<Self> {
        let toml_string = std::fs::read_to_string(path)?;
        let mut res: Self = toml::from_str(&toml_string)?;
        res.source_path = Some(path.to_owned());
        Ok(res)
    }

    fn find_parse_dir() -> Result<Option<PathBuf>> {
        let current_path = std::env::current_dir()?.join("evcxr.toml");
        if current_path.exists() {
            return Ok(Some(current_path));
        }
        let config_path = crate::config_dir();
        if let Some(config_path) = config_path {
            let config_path = config_path.join("evcxr.toml");
            if config_path.exists() {
                return Ok(Some(config_path));
            }
        }
        Ok(None)
    }

    pub(crate) fn find_then_parse() -> Result<Self> {
        match Self::find_parse_dir()? {
            Some(path) => Self::parse_from_path(&path),
            None => Ok(Self::new()),
        }
    }

    pub(crate) fn get_tmp_dir(&self) -> Result<TmpDirVar> {
        let tmp_dir = match &self.evcxr.tmpdir {
            Some(from_parse) => TmpDirVar::PathBuf(PathBuf::from(from_parse)),
            None => match std::env::var("EVCXR_TMPDIR") {
                Ok(from_env) => TmpDirVar::PathBuf(PathBuf::from(from_env)),
                _ => TmpDirVar::TmpDir(tempfile::tempdir()?),
            },
        };
        Ok(tmp_dir)
    }

    pub(crate) fn update_config(self, config: &mut Config) -> Result<()> {
        // config.tmpdir = self.get_tmp_dir()?.get_path()?;
        config.preserve_vars_on_panic = self.evcxr.preserve_vars_on_panic;
        config.offline_mode = self.evcxr.offline_mode;
        config.sccache = self.evcxr.sccache.map(PathBuf::from);
        config.allow_static_linking = self.evcxr.allow_static_linking;
        config.opt_level = self.evcxr.opt_level;
        Ok(())
    }

    pub(crate) fn get_dep_string(&self) -> Result<String> {
        let mut res = vec![];
        for (crate_name, crate_config) in self.dependencies.iter() {
            let res_config_v = match crate_config {
                Value::Table(table) => {
                    let res_config_v = toml::to_string(&table)?
                        .lines()
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(r#"{{{}}}"#, res_config_v)
                }
                other => {
                    format!("{}", other)
                }
            };
            let res_part = format!(":dep {crate_name} = {res_config_v}\n");
            res.push(res_part);
        }
        Ok(res.concat())
    }
}

impl EvcxrToml {
    fn new() -> Self {
        Self {
            preserve_vars_on_panic: default_value::preserve_vars_on_panic(),
            offline_mode: default_value::offline_mode(),
            allow_static_linking: default_value::allow_static_linking(),
            opt_level: default_value::opt_level(),
            ..Default::default()
        }
    }
}

mod default_value {
    pub(super) fn preserve_vars_on_panic() -> bool {
        true
    }
    pub(super) fn offline_mode() -> bool {
        false
    }
    pub(super) fn allow_static_linking() -> bool {
        true
    }
    pub(super) fn opt_level() -> String {
        String::from("2")
    }
}
