//! `#[endpoint("/path")]` attribute macro — JOLT-RS-038.
//!
//! Phase08 ladder:
//! - JOLT-RS-038 (this file): parse the path string literal from the attribute tokens.
//! - JOLT-RS-039: scan the impl block for `#[get]`/`#[post]`/`#[put]`/`#[patch]`/`#[delete]` methods.
//! - JOLT-RS-040: validate handler signatures.
//! - JOLT-RS-041: emit the (Method, path) -> handler match.
//!
//! The parsing entry point is split out from `lib.rs` so it can be unit-tested
//! against a `proc_macro2::TokenStream` (proc-macro entry points themselves
//! cannot be invoked outside the compiler).

use syn::{parse2, LitStr};

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
}
