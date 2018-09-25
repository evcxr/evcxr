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

/// Returns whether the suplied item and all its fields (in the case of structs) are marked as pub.
pub(crate) fn is_item_public(item: &syn::Item) -> bool {
    fn is_public(vis: &syn::Visibility) -> bool {
        // syn::Visibility appears to not implement Eq.
        if let syn::Visibility::Public(..) = vis {
            true
        } else {
            false
        }
    }

    match item {
        syn::Item::Static(i) => is_public(&i.vis),
        syn::Item::Const(i) => is_public(&i.vis),
        syn::Item::Fn(i) => is_public(&i.vis),
        syn::Item::Mod(i) => is_public(&i.vis),
        syn::Item::Type(i) => is_public(&i.vis),
        syn::Item::Struct(i) => is_public(&i.vis) && i.fields.iter().all(|f| is_public(&f.vis)),
        syn::Item::Enum(i) => is_public(&i.vis),
        syn::Item::Union(i) => is_public(&i.vis),
        syn::Item::Trait(i) => is_public(&i.vis),
        syn::Item::Impl(i) => {
            i.trait_.is_some() || i.items.iter().all(|i2| match i2 {
                syn::ImplItem::Const(i) => is_public(&i.vis),
                syn::ImplItem::Method(i) => is_public(&i.vis),
                syn::ImplItem::Type(i) => is_public(&i.vis),
                _ => true,
            })
        }
        _ => true,
    }
}

/// Returns the name of an item if it has one.
pub(crate) fn item_name(item: &syn::Item) -> Option<String> {
    item_ident(item).map(|ident| format!("{}", ident))
}

/// Returns the ident of an item if it has one.
fn item_ident(item: &syn::Item) -> Option<&proc_macro2::Ident> {
    Some(match item {
        syn::Item::Static(i) => &i.ident,
        syn::Item::Const(i) => &i.ident,
        syn::Item::Fn(i) => &i.ident,
        syn::Item::Mod(i) => &i.ident,
        syn::Item::Type(i) => &i.ident,
        syn::Item::Struct(i) => &i.ident,
        syn::Item::Enum(i) => &i.ident,
        syn::Item::Union(i) => &i.ident,
        syn::Item::Trait(i) => &i.ident,
        _ => return None,
    })
}
