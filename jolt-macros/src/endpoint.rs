//! `#[endpoint("/path")]` attribute macro — phase08 parsing surface.
//!
//! Phase08 ladder:
//! - JOLT-RS-038: parse the path string literal from the attribute tokens. (landed)
//! - JOLT-RS-039 (this iteration): scan the impl block for `#[get]`/`#[post]`/
//!   `#[put]`/`#[patch]`/`#[delete]` methods, collect their signatures.
//! - JOLT-RS-040: validate handler signatures.
//! - JOLT-RS-041: emit the (Method, path) -> handler match.
//!
//! The parsing entry points are split out from `lib.rs` so they can be
//! unit-tested against a `proc_macro2::TokenStream` / parsed `syn::ItemImpl`
//! (proc-macro entry points themselves cannot be invoked outside the compiler).
//!
//! Verb attributes (`#[get]`, `#[post]`, ...) are treated as **magic markers**:
//! `#[endpoint]` recognizes them when scanning, but they are not registered as
//! their own proc-macro attributes. JOLT-RS-041 will strip them from the
//! re-emitted impl so rustc never sees them.

use syn::{parse2, Attribute, ImplItem, ItemImpl, LitStr, Meta, Signature};

/// Parsed shape of `#[endpoint("/path")]`'s attribute argument.
#[allow(dead_code)] // `path` is read by tests this iteration; JOLT-RS-041 wires it into codegen.
pub(crate) struct EndpointAttr {
    pub(crate) path: LitStr,
}

/// Parse the attribute tokens of `#[endpoint(...)]` into an [`EndpointAttr`].
///
/// The single positional argument MUST be a string literal — the route path.
/// Empty input or non-string-literal input is rejected with a `syn::Error`
/// pointing at the offending span (or at call-site for empty input).
pub(crate) fn parse_endpoint_attr(
    tokens: proc_macro2::TokenStream,
) -> syn::Result<EndpointAttr> {
    let path: LitStr = parse2(tokens)?;
    Ok(EndpointAttr { path })
}

/// HTTP verb tagged on a method via a magic-marker attribute. Mirrors
/// `jolt_core::Method` but lives in the macro crate to avoid a cyclic dep
/// (jolt-core depends on jolt-macros at the workspace level once macros are
/// re-exported). Codegen in JOLT-RS-041 will emit `::jolt_core::Method::Get`
/// etc. from this enum via [`HttpMethod::as_jolt_core_variant_ident`] (added
/// when 041 lands).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // variants are constructed via `from_attr_name`; tests read them.
pub(crate) enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    /// Returns `Some(verb)` if `name` is one of the recognized verb-attribute
    /// names (`get`, `post`, `put`, `patch`, `delete`). Returns `None` for any
    /// other identifier — including verbs we don't currently route (`head`,
    /// `options`) which are handled by jolt-core's auto-OPTIONS / Allow-header
    /// machinery rather than by user-defined endpoints.
    #[allow(dead_code)] // tests consume it; lib.rs wires it in once JOLT-RS-041 lands.
    fn from_attr_name(name: &str) -> Option<Self> {
        match name {
            "get" => Some(Self::Get),
            "post" => Some(Self::Post),
            "put" => Some(Self::Put),
            "patch" => Some(Self::Patch),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

/// One method discovered inside an `impl` block by [`scan_methods`].
///
/// `sig` is cloned from the source `ImplItemFn` so callers can inspect it
/// without holding a borrow on the input `ItemImpl`. JOLT-RS-040 will read
/// `sig.inputs` (first arg must be `&self`) and `sig.output` (must be
/// `Response<T>` or `Result<Response<T>, E>`); JOLT-RS-041 will read
/// `sig.ident` and `http_method` to emit the dispatch match arm.
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields are read by tests this iteration; JOLT-RS-040/041 read them in codegen.
pub(crate) struct DiscoveredMethod {
    pub(crate) http_method: HttpMethod,
    pub(crate) sig: Signature,
}

/// Walk `item.items`, find each `ImplItem::Fn` whose attribute list contains
/// exactly one of the recognized verb attributes, and collect the discovered
/// method's verb + signature.
///
/// Methods with no verb attribute are skipped (an impl block can have helper
/// methods alongside endpoint handlers). Non-fn impl items (consts, type
/// aliases, sub-macros) are ignored entirely.
///
/// A method tagged with two or more verb attributes (e.g. both `#[get]` and
/// `#[post]`) is a parse-time error — the second verb attribute carries the
/// span. Routing the same handler under multiple verbs is not currently
/// supported; if a future PRD wants it, this check relaxes into a `Vec<HttpMethod>`
/// per discovered method.
#[allow(dead_code)] // tests consume it; lib.rs wires it in once JOLT-RS-041 lands.
pub(crate) fn scan_methods(item: &ItemImpl) -> syn::Result<Vec<DiscoveredMethod>> {
    let mut discovered = Vec::new();
    for impl_item in &item.items {
        let ImplItem::Fn(method_fn) = impl_item else {
            continue;
        };
        let mut matched: Option<HttpMethod> = None;
        for attr in &method_fn.attrs {
            let Some(verb) = verb_from_attr(attr) else {
                continue;
            };
            if matched.is_some() {
                return Err(syn::Error::new_spanned(
                    attr,
                    "method has more than one HTTP verb attribute (use only one of #[get], #[post], #[put], #[patch], #[delete])",
                ));
            }
            matched = Some(verb);
        }
        if let Some(http_method) = matched {
            discovered.push(DiscoveredMethod {
                http_method,
                sig: method_fn.sig.clone(),
            });
        }
    }
    Ok(discovered)
}

/// Extract the verb from a single attribute, if it is one of the recognized
/// magic markers. Path-style attributes like `#[axum::get]` or `#[get(...)]`
/// are NOT matched: only bare `#[get]`-shape attributes whose path is a single
/// identifier matching one of the five verb names AND that carry no argument
/// list. This keeps the marker surface narrow — a user who writes `#[axum::get]`
/// or `#[get("/path")]` clearly meant a different macro (likely axum's) and we
/// should not steal it.
#[allow(dead_code)] // exercised via `scan_methods` (also dead this iteration).
fn verb_from_attr(attr: &Attribute) -> Option<HttpMethod> {
    if !matches!(attr.meta, Meta::Path(_)) {
        return None;
    }
    let ident = attr.path().get_ident()?;
    HttpMethod::from_attr_name(&ident.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::TokenStream;
    use std::str::FromStr;

    #[test]
    fn extracts_simple_path() {
        let tokens = TokenStream::from_str(r#""/api/test""#).unwrap();
        let parsed = parse_endpoint_attr(tokens).expect("parses");
        assert_eq!(parsed.path.value(), "/api/test");
    }

    #[test]
    fn extracts_root_path() {
        let tokens = TokenStream::from_str(r#""/""#).unwrap();
        let parsed = parse_endpoint_attr(tokens).expect("parses");
        assert_eq!(parsed.path.value(), "/");
    }

    #[test]
    fn extracts_path_with_param_placeholder() {
        let tokens = TokenStream::from_str(r#""/users/:id""#).unwrap();
        let parsed = parse_endpoint_attr(tokens).expect("parses");
        assert_eq!(parsed.path.value(), "/users/:id");
    }

    #[test]
    fn rejects_empty_attr() {
        let tokens = TokenStream::new();
        assert!(parse_endpoint_attr(tokens).is_err());
    }

    #[test]
    fn rejects_integer_literal() {
        let tokens = TokenStream::from_str("123").unwrap();
        assert!(parse_endpoint_attr(tokens).is_err());
    }

    #[test]
    fn rejects_bare_identifier() {
        let tokens = TokenStream::from_str("path").unwrap();
        assert!(parse_endpoint_attr(tokens).is_err());
    }

    #[test]
    fn rejects_trailing_tokens_after_path() {
        let tokens = TokenStream::from_str(r#""/api/test", "extra""#).unwrap();
        assert!(parse_endpoint_attr(tokens).is_err());
    }

    // ----- JOLT-RS-039: scan_methods -----

    fn parse_impl(src: &str) -> ItemImpl {
        syn::parse_str::<ItemImpl>(src).expect("test input parses as ItemImpl")
    }

    #[test]
    fn discovers_single_get_method() {
        // PRD-mandated verification: impl block with #[get] fn hello(...) ->
        // found method 'hello' tagged as GET.
        let item = parse_impl(
            r#"
            impl Hello {
                #[get]
                fn hello(&self) {}
            }
            "#,
        );
        let found = scan_methods(&item).expect("scan succeeds");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].http_method, HttpMethod::Get);
        assert_eq!(found[0].sig.ident.to_string(), "hello");
    }

    #[test]
    fn discovers_one_method_per_verb() {
        let item = parse_impl(
            r#"
            impl Multi {
                #[get] fn list(&self) {}
                #[post] fn create(&self) {}
                #[put] fn replace(&self) {}
                #[patch] fn update(&self) {}
                #[delete] fn destroy(&self) {}
            }
            "#,
        );
        let found = scan_methods(&item).expect("scan succeeds");
        let actual: Vec<(HttpMethod, String)> = found
            .iter()
            .map(|d| (d.http_method, d.sig.ident.to_string()))
            .collect();
        assert_eq!(
            actual,
            vec![
                (HttpMethod::Get, "list".to_string()),
                (HttpMethod::Post, "create".to_string()),
                (HttpMethod::Put, "replace".to_string()),
                (HttpMethod::Patch, "update".to_string()),
                (HttpMethod::Delete, "destroy".to_string()),
            ],
        );
    }

    #[test]
    fn skips_methods_without_verb_attribute() {
        // Helper methods on an endpoint impl are common — `validate`, `db`, etc.
        // They MUST be ignored, not error.
        let item = parse_impl(
            r#"
            impl Mixed {
                #[get]
                fn handler(&self) {}

                fn helper(&self) {}

                #[inline]
                fn other_helper(&self, x: i32) -> i32 { x }
            }
            "#,
        );
        let found = scan_methods(&item).expect("scan succeeds");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].sig.ident.to_string(), "handler");
    }

    #[test]
    fn ignores_non_fn_impl_items() {
        // Impl blocks can also hold associated consts and types. These are not
        // endpoints — they must not be scanned (and must not crash the walk).
        let item = parse_impl(
            r#"
            impl WithConsts {
                const MAX: usize = 32;
                type Output = String;

                #[get]
                fn handler(&self) {}
            }
            "#,
        );
        let found = scan_methods(&item).expect("scan succeeds");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].http_method, HttpMethod::Get);
    }

    #[test]
    fn empty_impl_returns_empty_vec() {
        let item = parse_impl("impl Empty {}");
        let found = scan_methods(&item).expect("scan succeeds");
        assert!(found.is_empty());
    }

    #[test]
    fn captures_signature_inputs_and_output() {
        // 040 will read `sig.inputs` (first arg `&self`) and `sig.output`
        // (return type). Pin that the signature survives intact.
        let item = parse_impl(
            r#"
            impl WithSig {
                #[post]
                fn create(&self, body: String) -> Result<(), Error> {
                    Ok(())
                }
            }
            "#,
        );
        let found = scan_methods(&item).expect("scan succeeds");
        assert_eq!(found.len(), 1);
        let sig = &found[0].sig;
        assert_eq!(sig.inputs.len(), 2); // &self + body
        match &sig.output {
            syn::ReturnType::Type(_, _) => {} // has explicit return type
            syn::ReturnType::Default => panic!("expected explicit return type"),
        }
    }

    #[test]
    fn rejects_method_with_two_verb_attributes() {
        let item = parse_impl(
            r#"
            impl Bad {
                #[get]
                #[post]
                fn confused(&self) {}
            }
            "#,
        );
        let err = scan_methods(&item).expect_err("two verb attributes is an error");
        let msg = err.to_string();
        assert!(
            msg.contains("more than one HTTP verb attribute"),
            "expected diagnostic about multi-verb, got: {msg}",
        );
    }

    #[test]
    fn ignores_path_style_and_argful_verb_attributes() {
        // `#[axum::get]` (multi-segment path) and `#[get("/path")]` (path with
        // arglist) are NOT magic markers — the marker surface is bare `#[get]`
        // only. This avoids stealing attributes that belong to other macros
        // (e.g. axum's `#[get("/path")]`-shape route attributes).
        let item = parse_impl(
            r#"
            impl Pathy {
                #[axum::get]
                fn axum_handler(&self) {}

                #[get("/with-path")]
                fn args_handler(&self) {}

                #[get]
                fn marker_handler(&self) {}
            }
            "#,
        );
        let found = scan_methods(&item).expect("scan succeeds");
        let names: Vec<String> = found.iter().map(|d| d.sig.ident.to_string()).collect();
        assert_eq!(names, vec!["marker_handler".to_string()]);
    }
}
