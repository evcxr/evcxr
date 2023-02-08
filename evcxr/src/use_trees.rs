use ra_ap_syntax::ast;

// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Import {
    /// use x as _;
    /// use x::*;
    Unnamed(String),
    /// use x::y;
    /// use x::y as z;
    Named { name: String, code: String },
}

impl Import {
    fn format(name: &str, path: &[String]) -> Import {
        let joined_path = path.join("::");
        let code = if path.last().map(String::as_str) == Some(name) {
            format!("use {joined_path};")
        } else {
            format!("use {joined_path} as {name};")
        };
        if name == "_" || name == "*" {
            Import::Unnamed(code)
        } else {
            Import::Named {
                name: name.to_string(),
                code,
            }
        }
    }
}

pub(crate) fn use_tree_names_do(use_tree: &ast::UseTree, out: &mut impl FnMut(Import)) {
    fn process_use_tree(use_tree: &ast::UseTree, prefix: &[String], out: &mut impl FnMut(Import)) {
        if let Some(path) = use_tree.path() {
            // If we get ::self, ignore it and use what we've got so far.
            if path.segment().and_then(|segment| segment.kind())
                == Some(ast::PathSegmentKind::SelfKw)
            {
                if let Some(last) = prefix.last() {
                    out(Import::format(last, prefix));
                }
                return;
            }

            // Collect the components of `path`.
            let mut path = path;
            let mut path_parts = Vec::new();
            loop {
                if let Some(segment) = path.segment() {
                    if let Some(name_ref) = segment.name_ref() {
                        path_parts.push(name_ref.text().to_owned());
                    } else if let Some(token) = segment.crate_token() {
                        path_parts.push(token.text().to_owned());
                    }
                    if let Some(qualifier) = path.qualifier() {
                        path = qualifier;
                        continue;
                    }
                }
                break;
            }
            path_parts.reverse();

            // Combine the existing prefix with the new path components.
            let mut new_prefix = Vec::with_capacity(prefix.len() + path_parts.len());
            new_prefix.extend(prefix.iter().cloned());
            new_prefix.append(&mut path_parts);

            // Recurse into any subtree.
            if let Some(tree_list) = use_tree.use_tree_list() {
                for subtree in tree_list.use_trees() {
                    process_use_tree(&subtree, &new_prefix, out);
                }
            } else if let Some(rename) = use_tree.rename() {
                if let Some(name) = ast::HasName::name(&rename) {
                    out(Import::format(&name.text(), &new_prefix));
                } else if let Some(underscore) = rename.underscore_token() {
                    out(Import::format(underscore.text(), &new_prefix));
                }
            } else if let Some(star_token) = use_tree.star_token() {
                new_prefix.push(star_token.text().to_owned());
                out(Import::format(star_token.text(), &new_prefix));
            } else if let Some(ident) = new_prefix.last() {
                out(Import::format(ident, &new_prefix));
            }
        }
    }

    process_use_tree(use_tree, &[], out);
}

#[cfg(test)]
mod tests {
    use super::use_tree_names_do;
    use super::Import;
    use ra_ap_syntax::ast;
    use ra_ap_syntax::ast::HasModuleItem;

    fn use_tree_names(code: &str) -> Vec<Import> {
        let mut out = Vec::new();
        let file = ast::SourceFile::parse(code);
        for item in file.tree().items() {
            if let ast::Item::Use(use_stmt) = item {
                if let Some(use_tree) = use_stmt.use_tree() {
                    use_tree_names_do(&use_tree, &mut |import| {
                        out.push(import);
                    });
                }
            }
        }
        out
    }

    fn unnamed(code: &str) -> Import {
        Import::Unnamed(code.to_owned())
    }

    fn named(name: &str, code: &str) -> Import {
        Import::Named {
            name: name.to_owned(),
            code: code.to_owned(),
        }
    }

    #[test]
    fn test_complex_tree() {
        assert_eq!(
            use_tree_names(
                "use std::collections::{self, hash_map::{HashMap}, HashSet as MyHashSet};"
            ),
            vec![
                named("collections", "use std::collections;"),
                named("HashMap", "use std::collections::hash_map::HashMap;"),
                named("MyHashSet", "use std::collections::HashSet as MyHashSet;")
            ]
        );
    }

    #[test]
    fn test_underscore() {
        assert_eq!(
            use_tree_names("use foo::bar::MyTrait as _;"),
            vec![unnamed("use foo::bar::MyTrait as _;"),]
        );
    }

    #[test]
    fn test_glob() {
        assert_eq!(
            use_tree_names("use foo::bar::*;"),
            vec![unnamed("use foo::bar::*;"),]
        );
    }
}
