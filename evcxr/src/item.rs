// Copyright 2018 Google Inc.
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

use proc_macro2;
use syn;

/// Returns the name of an item if it has one.
pub(crate) fn item_name(item: &syn::Item) -> Option<String> {
    item_ident(item).map(|ident| format!("{}", ident))
}

/// Returns the ident of an item if it has one.
fn item_ident(item: &syn::Item) -> Option<&proc_macro2::Ident> {
    Some(match item {
        syn::Item::Static(i) => &i.ident,
        syn::Item::Const(i) => &i.ident,
        syn::Item::Fn(i) => &i.sig.ident,
        syn::Item::Mod(i) => &i.ident,
        syn::Item::Type(i) => &i.ident,
        syn::Item::Struct(i) => &i.ident,
        syn::Item::Enum(i) => &i.ident,
        syn::Item::Union(i) => &i.ident,
        syn::Item::Trait(i) => &i.ident,
        _ => return None,
    })
}
