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

use syn;

/// Finds all idents inside the supplied pattern and passes them to the callback.
pub(crate) fn idents_do<F: FnMut(&syn::PatIdent)>(pat: &syn::Pat, callback: &mut F) {
    match pat {
        syn::Pat::Ident(ref pat_ident) => callback(pat_ident),
        syn::Pat::Struct(ref pat_struct) => {
            for field in &pat_struct.fields {
                idents_do(&field.pat, callback);
            }
        }
        syn::Pat::Tuple(ref pat_tuple) => {
            for member in &pat_tuple.front {
                idents_do(member, callback);
            }
        }
        syn::Pat::TupleStruct(ref pat_tuple) => {
            for member in &pat_tuple.pat.front {
                idents_do(member, callback);
            }
        }
        x => {
            println!("Unhandled pat kind: {:?}", x);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::idents_do;
    use syn;

    fn get_idents(code: &str) -> Vec<String> {
        let mut all_idents = Vec::new();
        idents_do(
            &syn::parse_str::<syn::Pat>(code).unwrap(),
            &mut |pat: &syn::PatIdent| all_idents.push(pat.ident.to_string()),
        );
        all_idents
    }

    #[test]
    fn simple_variable() {
        assert_eq!(get_idents("a"), vec!["a"]);
    }

    #[test]
    fn destructure_struct() {
        assert_eq!(get_idents("Point {x, y: y2}"), vec!["x", "y2"]);
    }

    #[test]
    fn destructure_tuple() {
        assert_eq!(get_idents("(a, b, c)"), vec!["a", "b", "c"]);
    }

    #[test]
    fn destructure_tuple_struct() {
        assert_eq!(get_idents("Foo(a, b, c)"), vec!["a", "b", "c"]);
    }
}
