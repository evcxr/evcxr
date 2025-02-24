// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use once_cell::sync::Lazy;
use ra_ap_base_db::SourceRoot;
use ra_ap_hir as ra_hir;
use ra_ap_ide as ra_ide;
use ra_ap_ide::FileRange;
use ra_ap_ide_db::imports::insert_use::ImportGranularity;
use ra_ap_ide_db::imports::insert_use::InsertUseConfig;
use ra_ap_ide_db::EditionedFileId;
use ra_ap_ide_db::FxHashMap;
use ra_ap_ide_db::SnippetCap;
use ra_ap_paths::AbsPathBuf;
use ra_ap_project_model::CargoConfig;
use ra_ap_project_model::ProjectManifest;
use ra_ap_project_model::ProjectWorkspace;
use ra_ap_project_model::RustLibSource;
use ra_ap_syntax::ast::AstNode;
use ra_ap_syntax::ast::{self};
use ra_ap_syntax::TextRange;
use ra_ap_vfs as ra_vfs;
use ra_ap_vfs::loader::LoadingProgress;
use ra_ap_vfs::FileId;
use ra_ap_vfs_notify as vfs_notify;
use ra_ide::CallableSnippets;
use ra_ide::Edition;
use ra_ide::HoverConfig;
use ra_ide::HoverResult;
use ra_ide::RangeInfo;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::path::Path;

pub(crate) const EDITION: Edition = Edition::Edition2021;

pub(crate) struct RustAnalyzer {
    with_sysroot: bool,
    root_directory: AbsPathBuf,
    analysis_host: ra_ide::AnalysisHost,
    vfs: ra_vfs::Vfs,
    loader: vfs_notify::NotifyHandle,
    message_receiver: crossbeam_channel::Receiver<ra_vfs::loader::Message>,
    last_cargo_toml: Option<Vec<u8>>,
    source_file: AbsPathBuf,
    source_file_id: FileId,
    current_source: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TypeName {
    Named(String),
    Closure,
    Unknown,
}

#[derive(Debug)]
pub(crate) struct VariableInfo {
    /// The variable's type as Rust code, or None if we couldn't determine it.
    pub(crate) type_name: TypeName,
    /// Whether the variable is declared as mutable.
    pub(crate) is_mutable: bool,
}

impl RustAnalyzer {
    pub(crate) fn new(root_directory: &Path) -> Result<RustAnalyzer> {
        use ra_vfs::loader::Handle;
        let (message_sender, message_receiver) = crossbeam_channel::unbounded();
        let mut vfs = ra_vfs::Vfs::default();
        let root_directory = AbsPathBuf::try_from(
            root_directory
                .to_str()
                .context("Root directory is not UTF-8")?,
        )
        .map_err(|path| anyhow!("Evcxr tmpdir is not absolute: '{:?}'", path))?;
        let source_file = root_directory.join("src/lib.rs");
        // We need to write the file to the filesystem even though we subsequently set the file
        // contents via the vfs and change.change_file. This is because the loader checks for the
        // files existence when determining the crate structure.
        let src_dir = root_directory.join("src");
        std::fs::create_dir_all(&src_dir)
            .with_context(|| format!("Failed to create directory `{src_dir:?}`"))?;
        // Pre-allocate an ID for our main source file.
        let vfs_source_file: ra_vfs::VfsPath = source_file.clone().into();
        vfs.set_file_contents(vfs_source_file.clone(), Some(vec![]));
        let source_file_id = vfs.file_id(&vfs_source_file).unwrap();
        Ok(RustAnalyzer {
            with_sysroot: true,
            root_directory,
            analysis_host: Default::default(),
            vfs,
            loader: vfs_notify::NotifyHandle::spawn(message_sender),
            message_receiver,
            last_cargo_toml: None,
            source_file,
            source_file_id,
            current_source: String::new(),
        })
    }

    pub(crate) fn set_source(&mut self, source: String) -> Result<()> {
        self.current_source = source;
        let mut change = ra_hir::ChangeWithProcMacros::new();

        std::fs::write(self.source_file.as_path(), &*self.current_source)
            .with_context(|| format!("Failed to write {:?}", self.source_file))?;
        self.vfs.set_file_contents(
            self.source_file.clone().into(),
            Some(self.current_source.bytes().collect()),
        );
        change.change_file(self.source_file_id, Some(self.current_source.clone()));

        // Check to see if we haven't yet loaded Cargo.toml, or if it's changed since we read it.
        let cargo_toml = Some(std::fs::read(self.cargo_toml_filename()).with_context(|| {
            format!(
                "Failed to read Cargo.toml from `{:?}`",
                self.cargo_toml_filename()
            )
        })?);
        if cargo_toml != self.last_cargo_toml {
            self.load_cargo_toml(&mut change)?;
            self.last_cargo_toml = cargo_toml;
        }

        self.analysis_host.apply_change(change);
        Ok(())
    }

    /// Returns top-level variable names and their types in the specified function.
    pub(crate) fn top_level_variables(&self, function_name: &str) -> HashMap<String, VariableInfo> {
        use ra_ap_syntax::ast::HasModuleItem;
        use ra_ap_syntax::ast::HasName;
        let mut result = HashMap::new();
        let sema = ra_ide::Semantics::new(self.analysis_host.raw_database());
        let source_file = sema.parse(EditionedFileId::new(self.source_file_id, EDITION));
        for item in source_file.items() {
            if let ast::Item::Fn(function) = item {
                if function
                    .name()
                    .map(|n| n.text() == function_name)
                    .unwrap_or(false)
                {
                    let Some(body) = function.body() else {
                        continue;
                    };
                    let module = sema
                        .scope(function.syntax())
                        .map(|scope| scope.module())
                        .unwrap();
                    for statement in body.statements() {
                        if let ast::Stmt::LetStmt(let_stmt) = statement {
                            if let Some(pat) = let_stmt.pat() {
                                if !add_variable_for_pattern(
                                    &pat,
                                    &sema,
                                    let_stmt.ty(),
                                    module,
                                    &mut result,
                                ) {
                                    // We didn't add a variable for `pat`, possibly because it's a
                                    // more complex pattern that needs destructuring. Try for each
                                    // sub pattern. This time, we ignore the explicit type, because
                                    // it applies to the whole pattern, not to its parts. Note, this
                                    // will attempt `pat` again, but that's OK, since it failed
                                    // above, so will fail again.
                                    for d in pat.syntax().descendants() {
                                        if let Some(sub_pat) = ast::Pat::cast(d) {
                                            add_variable_for_pattern(
                                                &sub_pat,
                                                &sema,
                                                None,
                                                module,
                                                &mut result,
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
        result
    }

    fn load_cargo_toml(&mut self, change: &mut ra_hir::ChangeWithProcMacros) -> Result<()> {
        let manifest = ProjectManifest::from_manifest_file(self.cargo_toml_filename())?;
        let sysroot = if self.with_sysroot {
            Some(RustLibSource::Discover)
        } else {
            None
        };
        let config = CargoConfig {
            sysroot,
            ..CargoConfig::default()
        };
        let workspace = ProjectWorkspace::load(manifest, &config, &|_| {})?;
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
            version: 1,
            load,
            watch: vec![],
        });

        for message in &self.message_receiver {
            match message {
                ra_vfs::loader::Message::Progress {
                    n_total: _, n_done, ..
                } => {
                    if n_done == LoadingProgress::Finished {
                        break;
                    }
                }
                ra_vfs::loader::Message::Loaded { files }
                | ra_vfs::loader::Message::Changed { files } => {
                    for (path, contents) in files {
                        let vfs_path: ra_vfs::VfsPath = path.to_path_buf().into();
                        self.vfs
                            .set_file_contents(vfs_path.clone(), contents.clone());
                    }
                }
            }
        }

        for (file_id, changed_file) in self.vfs.take_changes() {
            let mut new_contents = None;
            if let ra_vfs::Change::Create(v, _hash) | ra_vfs::Change::Modify(v, _hash) =
                changed_file.change
            {
                if let Ok(text) = std::str::from_utf8(&v) {
                    new_contents = Some(text.to_owned());
                }
            }
            change.change_file(file_id, new_contents);
        }
        change.set_roots(
            ra_vfs::file_set::FileSetConfig::default()
                .partition(&self.vfs)
                .into_iter()
                .map(SourceRoot::new_local)
                .collect(),
        );
        let (crate_graph, _) = workspace.to_crate_graph(
            &mut |path| self.vfs.file_id(&path.to_path_buf().into()),
            &FxHashMap::default(),
        );
        let num_crates = crate_graph.len();
        change.set_crate_graph(crate_graph);
        change.set_target_data_layouts(vec![workspace.target_layout; num_crates]);
        change.set_toolchains(vec![workspace.toolchain; num_crates]);
        Ok(())
    }

    fn cargo_toml_filename(&self) -> AbsPathBuf {
        self.root_directory.join("Cargo.toml")
    }

    pub(crate) fn completions(&self, position: usize) -> Result<Completions> {
        let mut completions = Vec::new();
        let mut range = None;
        let config = ra_ide::CompletionConfig {
            enable_postfix_completions: true,
            snippet_cap: SnippetCap::new(true),
            enable_imports_on_the_fly: false,
            enable_self_on_the_fly: true,
            enable_private_editable: true,
            prefer_no_std: false,
            full_function_signatures: true,
            snippets: vec![],
            insert_use: InsertUseConfig {
                prefix_kind: ra_hir::PrefixKind::ByCrate,
                group: false,
                granularity: ImportGranularity::Item,
                enforce_granularity: false,
                skip_glob_imports: false,
            },
            callable: Some(CallableSnippets::FillArguments),
            limit: None,
            prefer_prelude: false,
            enable_term_search: true,
            term_search_fuel: 400,
            prefer_absolute: false,
        };
        if let Ok(Some(completion_items)) = self.analysis_host.analysis().completions(
            &config,
            ra_ide::FilePosition {
                file_id: self.source_file_id,
                offset: (position as u32).into(),
            },
            None,
        ) {
            for item in completion_items {
                use regex::Regex;
                static ARG_PLACEHOLDER: Lazy<Regex> =
                    Lazy::new(|| Regex::new("\\$\\{[0-9]+:([^}]*)\\}").unwrap());
                let mut indels = item.text_edit.iter();
                if let Some(indel) = indels.next() {
                    let text_to_delete = &self.current_source[indel.delete];
                    // Rust analyzer returns all available methods/fields etc. It's up to us to
                    // decide how what we filter and what we keep.
                    if !item.lookup().starts_with(text_to_delete) {
                        continue;
                    }
                    completions.push(Completion {
                        code: ARG_PLACEHOLDER
                            .replace_all(&indel.insert, "$1")
                            .replace("$0", ""),
                    });
                    if let Some(previous_range) = range.as_ref() {
                        if *previous_range != indel.delete {
                            bail!("Different completions wanted to replace different parts of the text");
                        }
                    } else {
                        range = Some(indel.delete)
                    }
                }
                if indels.next().is_some() {
                    bail!("Completion unexpectedly provided more than one insertion/deletion");
                }
            }
        }
        Ok(Completions {
            completions,
            start_offset: range.map(|range| range.start().into()).unwrap_or(position),
            end_offset: range.map(|range| range.end().into()).unwrap_or(position),
        })
    }

    pub(crate) fn hover(
        &self,
        text_range: TextRange,
        is_mark_down: bool,
    ) -> Result<Option<RangeInfo<HoverResult>>> {
        use ra_ide::HoverDocFormat as hdf;
        let hover_config: HoverConfig = HoverConfig {
            links_in_hover: true,
            memory_layout: None,
            documentation: true,
            keywords: true,
            format: if is_mark_down {
                hdf::Markdown
            } else {
                hdf::PlainText
            },
            max_trait_assoc_items_count: None,
            max_fields_count: Some(5),
            max_enum_variants_count: Some(5),
        };
        let file_range = FileRange {
            file_id: self.source_file_id,
            range: text_range,
        };
        match self
            .analysis_host
            .analysis()
            .hover(&hover_config, file_range)
        {
            Ok(range_info) => Ok(range_info),
            _ => bail!("hover fail"),
        }
    }
}

/// If `pat` represents a variable that is being defined, then record it in `result` and return
/// true.
fn add_variable_for_pattern(
    pat: &ast::Pat,
    sema: &ra_hir::Semantics<ra_ide::RootDatabase>,
    explicit_type: Option<ast::Type>,
    module: ra_hir::Module,
    result: &mut HashMap<String, VariableInfo>,
) -> bool {
    use ra_ap_syntax::ast::HasName;
    if let ast::Pat::IdentPat(ident_pat) = pat {
        if let Some(name) = ident_pat.name() {
            let type_name = get_type_name(
                explicit_type,
                sema.type_of_pat(pat).map(|info| info.original()),
                sema,
                module,
            );
            result.insert(
                name.text().to_string(),
                VariableInfo {
                    type_name,
                    is_mutable: ident_pat.mut_token().is_some(),
                },
            );
            return true;
        }
    }
    false
}

fn get_type_name(
    explicit_type: Option<ast::Type>,
    inferred_type: Option<ra_hir::Type>,
    sema: &ra_hir::Semantics<ra_ide::RootDatabase>,
    module: ra_hir::Module,
) -> TypeName {
    use ra_hir::HirDisplay;
    if let Some(explicit_type) = explicit_type {
        let type_name = explicit_type.syntax().text().to_string();
        if is_type_valid(&type_name) {
            return TypeName::Named(type_name);
        }
    }
    if let Some(ty) = inferred_type {
        if ty.is_closure() {
            return TypeName::Closure;
        }
        if let Ok(type_name) = ty.display_source_code(sema.db, module.into(), true) {
            if is_type_valid(&type_name) {
                return TypeName::Named(type_name);
            }
        }
    }
    TypeName::Unknown
}

/// Completions found in a particular context.
#[derive(Default)]
pub struct Completions {
    pub completions: Vec<Completion>,
    pub start_offset: usize,
    pub end_offset: usize,
}

/// A code completion. We use our own type rather than exposing rust-analyzer's CompletionItem,
/// since rust-analyzer is an internal implementation detail, so we don't want to expose it in a
/// public API.
#[derive(Debug, Eq, PartialEq)]
pub struct Completion {
    pub code: String,
}

/// Returns whether this appears to be a valid type. Rust analyzer, when asked to emit code for some
/// types, produces invalid code. In particular, fixed sized arrays come out without a size. e.g.
/// instead of `[i32, 5]`, we get `[i32, _]`.
pub(crate) fn is_type_valid(type_name: &str) -> bool {
    use ra_ap_syntax::SyntaxKind;
    let wrapped_source = format!("const _: {type_name} = foo();");
    let parsed = ast::SourceFile::parse(&wrapped_source, EDITION);
    if !parsed.errors().is_empty() {
        return false;
    }
    for node in parsed.syntax_node().descendants() {
        if node.kind() == SyntaxKind::ERROR || node.kind() == SyntaxKind::INFER_TYPE {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod test {
    use super::is_type_valid;
    use super::RustAnalyzer;
    use super::TypeName;
    use anyhow::Result;

    impl TypeName {
        fn named(name: &str) -> TypeName {
            TypeName::Named(name.to_owned())
        }
    }

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
            edition = "2021"

            [lib]
            "#,
        )?;

        ra.set_source(
            r#"
            struct Foo<const I: usize> {}
            struct Point {x: u8, y: u8}
            fn foo() {
                let v1 = true;
                let mut v1 = 42i32;
                let v2 = &[false];
                let v3: Foo<10> = Foo::<10> {};
                {
                    let v2 = false;
                    let v100 = true;
                }
                let (v4, ..) = (42u64, 43, 44);
                let p1 = Point {x: 1, y: 2};
                let Point {x, y: y2} = p1;
            }
            fn foo2() {
                let v9 = true;
            }"#
            .to_owned(),
        )?;
        let var_types = ra.top_level_variables("foo");
        assert_eq!(var_types["v1"].type_name, TypeName::named("i32"));
        assert!(var_types["v1"].is_mutable);
        assert_eq!(var_types["v2"].type_name, TypeName::named("&[bool; 1]"));
        assert!(!var_types["v2"].is_mutable);
        assert_eq!(var_types["v3"].type_name, TypeName::named("Foo<10>"));
        assert!(!var_types.contains_key("v100"));
        assert_eq!(var_types["v4"].type_name, TypeName::named("u64"));
        assert_eq!(var_types["x"].type_name, TypeName::named("u8"));
        assert_eq!(var_types["y2"].type_name, TypeName::named("u8"));

        ra.set_source(
            r#"
            fn foo() {
                let v1 = 1u16;
            }"#
            .to_owned(),
        )?;
        let var_types = ra.top_level_variables("foo");
        assert_eq!(var_types["v1"].type_name, TypeName::named("u16"));
        assert!(!var_types.contains_key("v2"));

        Ok(())
    }

    #[test]
    fn test_is_type_valid() {
        assert!(is_type_valid("Vec<String>"));
        assert!(is_type_valid("&[i32]"));
        assert!(!is_type_valid("[i32, _]"));
        assert!(!is_type_valid("Vec<_>"));
        assert!(is_type_valid("Foo<42>"));
    }
}
