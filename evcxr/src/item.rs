// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use ra_ap_syntax::ast;

/// Returns the name of an item if it has one.
pub(crate) fn item_name(item: &ast::Item) -> Option<String> {
    item_ident(item).map(|ident| format!("{ident}"))
}

/// Returns the ident of an item if it has one.
fn item_ident(item: &ast::Item) -> Option<ast::Name> {
    match item {
        ast::Item::Const(i) => ast::HasName::name(i),
        ast::Item::Enum(i) => ast::HasName::name(i),
        ast::Item::Fn(i) => ast::HasName::name(i),
        ast::Item::MacroRules(i) => ast::HasName::name(i),
        ast::Item::MacroDef(i) => ast::HasName::name(i),
        ast::Item::Module(i) => ast::HasName::name(i),
        ast::Item::Static(i) => ast::HasName::name(i),
        ast::Item::Struct(i) => ast::HasName::name(i),
        ast::Item::Trait(i) => ast::HasName::name(i),
        ast::Item::TypeAlias(i) => ast::HasName::name(i),
        ast::Item::Union(i) => ast::HasName::name(i),
        _ => None,
    }
}
