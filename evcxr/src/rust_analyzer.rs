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

use anyhow::{anyhow, Context, Result};
use ra_ap_base_db::SourceRoot;
use ra_ap_hir as ra_hir;
use ra_ap_ide as ra_ide;
use ra_ap_paths::AbsPathBuf;
use ra_ap_project_model::{CargoConfig, ProcMacroClient, ProjectManifest, ProjectWorkspace};
use ra_ap_syntax::ast::{self, AstNode};
use ra_ap_vfs as ra_vfs;
use ra_ap_vfs_notify as vfs_notify;
use std::sync::mpsc;
use std::{collections::HashMap, convert::TryFrom, path::Path, sync::Arc};

pub(crate) struct RustAnalyzer {
    with_sysroot: bool,
    root_directory: AbsPathBuf,
    analysis_host: ra_ide::AnalysisHost,
    vfs: ra_vfs::Vfs,
    loader: vfs_notify::NotifyHandle,
    message_receiver: mpsc::Receiver<ra_vfs::loader::Message>,
    last_cargo_toml: Option<Vec<u8>>,
}

pub(crate) struct VariableInfo {
    /// The variable's type as Rust code.
    pub(crate) type_name: String,
    /// Whether the variable is declared as mutable.
    pub(crate) is_mutable: bool,
}

impl RustAnalyzer {
    pub(crate) fn new(root_directory: &Path) -> Result<RustAnalyzer> {
        use ra_vfs::loader::Handle;
        let (message_sender, message_receiver) = std::sync::mpsc::channel();
        let mut ra = RustAnalyzer {
            with_sysroot: true,
            root_directory: AbsPathBuf::try_from(root_directory.to_owned())
                .map_err(|path| anyhow!("Evcxr tmpdir is not absolute: '{:?}'", path))?,
            analysis_host: Default::default(),
            vfs: Default::default(),
            loader: vfs_notify::NotifyHandle::spawn(Box::new(move |message| {
                let _ = message_sender.send(message);
            })),
            message_receiver,
            last_cargo_toml: None,
        };
        // Pre-allocate an ID for our main source file so that set_source can assume that it exists.
        ra.vfs.set_file_contents(ra.source_file().into(), None);
        Ok(ra)
    }

    pub(crate) fn set_source(&mut self, source: String) -> Result<()> {
        let mut change = ra_ide::Change::new();

        // We need to write the file to the filesystem even though we subsequently set the file
        // contents via the vfs and change.change_file. This is because the loader checks for the
        // files existence when determining the crate structure.
        let src_dir = self.root_directory.join("src");
        std::fs::create_dir_all(&src_dir)
            .with_context(|| format!("Failed to create directory {:?}", src_dir))?;
        std::fs::write(self.source_file().as_path(), &source)
            .with_context(|| format!("Failed to write {:?}", self.source_file()))?;
        self.vfs
            .set_file_contents(self.source_file().into(), Some(source.bytes().collect()));
        change.change_file(
            self.vfs.file_id(&self.source_file().into()).unwrap(),
            Some(Arc::new(source.to_owned())),
        );

        // Check to see if we haven't yet loaded Cargo.toml, or if it's changed since we read it.
        let cargo_toml = Some(std::fs::read(self.cargo_toml_filename())?);
        if cargo_toml != self.last_cargo_toml {
            self.load_cargo_toml(&mut change)?;
            self.last_cargo_toml = cargo_toml;
        }

        self.analysis_host.apply_change(change);
        Ok(())
    }

    /// Returns top-level variable names and their types in the specified function.
    pub(crate) fn top_level_variables(&self, function_name: &str) -> HashMap<String, VariableInfo> {
        use ra_ap_syntax::ast::{ModuleItemOwner, NameOwner};
        let mut result = HashMap::new();
        let sema = ra_ide::Semantics::new(self.analysis_host.raw_database());
        let main_rs = self.source_file();
        let source_file = sema.parse(self.vfs.file_id(&main_rs.into()).unwrap());
        for item in source_file.items() {
            if let ast::Item::Fn(function) = item {
                if function
                    .name()
                    .map(|n| n.text() == function_name)
                    .unwrap_or(false)
                {
                    let body = if let Some(b) = function.body() {
                        b
                    } else {
                        continue;
                    };
                    let module = sema.scope(&function.syntax()).module().unwrap();
                    for statement in body.statements() {
                        if let ast::Stmt::LetStmt(let_stmt) = statement {
                            if let Some(pat) = let_stmt.pat() {
                                if let ast::Pat::IdentPat(ident_pat) = pat.clone() {
                                    if let Some(name) = ident_pat.name() {
                                        use ra_hir::HirDisplay;
                                        if let Some(ty) = sema.type_of_pat(&pat) {
                                            if let Ok(type_name) =
                                                ty.display_source_code(sema.db, module.into())
                                            {
                                                result.insert(
                                                    name.text().to_string(),
                                                    VariableInfo {
                                                        type_name,
                                                        is_mutable: ident_pat.mut_token().is_some(),
                                                    },
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        result
    }

    fn load_cargo_toml(&mut self, change: &mut ra_ide::Change) -> Result<()> {
        let manifest = ProjectManifest::from_manifest_file(self.cargo_toml_filename())?;
        let workspace =
            ProjectWorkspace::load(manifest, &CargoConfig::default(), self.with_sysroot).unwrap();
        let load = workspace
            .to_roots()
            .iter()
            .map(|root| {
                ra_vfs::loader::Entry::Directories(ra_vfs::loader::Directories {
                    extensions: vec!["rs".to_owned()],
                    include: root.include.clone(),
                    exclude: root.exclude.clone(),
                })
            })
            .collect();
        // Note, set_config is what triggers loading and calling the callback that we registered when we created self.loader.
        use ra_vfs::loader::Handle;
        self.loader.set_config(ra_vfs::loader::Config {
            load,
            watch: vec![],
        });

        for message in &self.message_receiver {
            match message {
                ra_vfs::loader::Message::Progress { n_total, n_done } => {
                    if n_total == n_done {
                        break;
                    }
                }
                ra_vfs::loader::Message::Loaded { files } => {
                    for (path, contents) in files {
                        let vfs_path: ra_vfs::VfsPath = path.to_path_buf().into();
                        self.vfs
                            .set_file_contents(vfs_path.clone(), contents.clone());
                    }
                }
            }
        }

        for changed_file in self.vfs.take_changes() {
            let new_contents = if changed_file.exists() {
                String::from_utf8(self.vfs.file_contents(changed_file.file_id).to_owned())
                    .ok()
                    .map(|contents| Arc::new(contents))
            } else {
                None
            };
            change.change_file(changed_file.file_id, new_contents);
        }
        change.set_roots(
            ra_vfs::file_set::FileSetConfig::default()
                .partition(&self.vfs)
                .into_iter()
                .map(|file_set| SourceRoot::new_local(file_set))
                .collect(),
        );
        change.set_crate_graph(workspace.to_crate_graph(
            None,
            &ProcMacroClient::dummy(),
            &mut |path| self.vfs.file_id(&path.to_path_buf().into()),
        ));
        Ok(())
    }

    fn source_file(&self) -> AbsPathBuf {
        self.root_directory.join("src/lib.rs")
    }

    fn cargo_toml_filename(&self) -> AbsPathBuf {
        self.root_directory.join("Cargo.toml")
    }
}

/// Returns whether this appears to be a valid type. Rust analyzer, when asked to emit code for some
/// types, produces invalid code. In particular, fixed sized arrays come out without a size. e.g.
/// instead of `[i32, 5]`, we get `[i32, _]`.
pub(crate) fn is_type_valid(type_name: &str) -> bool {
    if let Ok(ty) = ast::Type::parse(&type_name) {
        for node in ty.syntax().descendants() {
            if node.kind() == ra_ap_syntax::SyntaxKind::ERROR {
                return false;
            }
        }
        return true;
    }
    false
}

#[cfg(test)]
mod test {
    use super::{is_type_valid, RustAnalyzer};
    use anyhow::Result;
    use tempfile;

    #[test]
    fn get_variable_types() -> Result<()> {
        let tmpdir = tempfile::tempdir()?;
        let mut ra = RustAnalyzer::new(tmpdir.path())?;
        ra.with_sysroot = false;
        std::fs::write(
            ra.cargo_toml_filename().to_path_buf(),
            r#"
            [package]
            name = "foo"
            version = "0.1.0"

            [lib]
            "#,
        )?;

        ra.set_source(
            r#"
            fn foo() {
                let v1 = true;
                let mut v1 = 42i32;
                let v2 = &[false];
                {
                    let v2 = false;
                    let v100 = true;
                }
            }
            fn foo2() {
                let v9 = true;
            }"#
            .to_owned(),
        )?;
        let var_types = ra.top_level_variables("foo");
        assert_eq!(var_types["v1"].type_name, "i32");
        assert!(var_types["v1"].is_mutable);
        assert_eq!(var_types["v2"].type_name, "&[bool; _]");
        assert!(!var_types["v2"].is_mutable);
        assert!(var_types.get("v100").is_none());

        ra.set_source(
            r#"
            fn foo() {
                let v1 = 1u16;
            }"#
            .to_owned(),
        )?;
        let var_types = ra.top_level_variables("foo");
        assert_eq!(var_types["v1"].type_name, "u16");
        assert!(var_types.get("v2").is_none());

        Ok(())
    }

    #[test]
    fn test_is_type_valid() {
        assert!(is_type_valid("Vec<String>"));
        assert!(is_type_valid("&[i32]"));
        assert!(!is_type_valid("[i32, _]"));
    }
}
