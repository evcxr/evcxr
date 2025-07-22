use crate::eval_context::Config;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;
use tempfile::TempDir;
use toml::Table;
use toml::Value;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigToml {
    #[serde(default = "EvcxrToml::new")]
    evcxr: EvcxrToml,
    #[serde(default = "Default::default")]
    dependencies: Table,
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct EvcxrToml {
    tmpdir: Option<String>,
    #[serde(default = "default_value::preserve_vars_on_panic")]
    preserve_vars_on_panic: bool,
    #[serde(default = "default_value::offline_mode")]
    offline_mode: bool,
    sccache: Option<String>,
    #[serde(default = "default_value::allow_static_linking")]
    allow_static_linking: bool,
    prelude: Option<String>,
    #[serde(default = "default_value::opt_level")]
    opt_level: String,
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
        config.preserve_vars_on_panic = self.evcxr.preserve_vars_on_panic;
        // config.tmpdir = self.get_tmp_dir()?.get_path()?;
        config.offline_mode = self.evcxr.offline_mode;
        config.sccache = self.evcxr.sccache.map(PathBuf::from);
        config.allow_static_linking = self.evcxr.allow_static_linking;
        config.opt_level = self.evcxr.opt_level;
        Ok(())
    }

    pub(crate) fn get_dep_string(&self) -> Result<Option<String>> {
        let mut res = vec![];
        if self.dependencies.is_empty() {
            return Ok(None);
        }
        for (crate_name, crate_config) in self.dependencies.iter() {
            let res_config_v = match crate_config {
                Value::Table(table) => {
                    let res_config_v = toml::to_string(&table)?
                        .lines()
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(r#"{{{res_config_v}}}"#)
                }
                other => {
                    format!("{other}")
                }
            };
            let res_part = format!(":dep {crate_name} = {res_config_v}\n");
            res.push(res_part);
        }
        Ok(Some(res.concat()))
    }

    pub(crate) fn get_dep_string_versions(&self) -> Result<Option<String>> {
        match self.get_dep_string()? {
            Some(x) => Ok(Some(x)),
            None => get_evcxr_config_content("init.evcxr"),
        }
    }

    pub(crate) fn get_prelude_string(&self) -> Result<Option<String>> {
        Ok(self.evcxr.prelude.clone())
    }

    pub(crate) fn get_prelude_string_versions(&self) -> Result<Option<String>> {
        match self.get_prelude_string()? {
            Some(x) => Ok(Some(x)),
            None => get_evcxr_config_content("prelude.rs"),
        }
    }
}

fn get_evcxr_config_content(file_name: &str) -> Result<Option<String>> {
    match crate::config_dir() {
        None => Ok(None),
        Some(evcxr_config_dir) => {
            let config_dir_prelude_path = evcxr_config_dir.join(file_name);
            match config_dir_prelude_path.exists() {
                false => Ok(None),
                true => {
                    let res = std::fs::read_to_string(&config_dir_prelude_path)?;
                    Ok(Some(res))
                }
            }
        }
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
