//! Comprehensive variant + round-trip coverage for `Method` and `StatusCode`.
//!
//! Submodule names are deliberate: `cargo test -p jolt-core -- tests::method`
//! and `cargo test -p jolt-core -- tests::status_code` filter cleanly to the
//! relevant slice.

mod method {
    use crate::{Method, ParseMethodError};
    use std::str::FromStr;

    const ALL_METHODS: &[(Method, &str)] = &[
        (Method::Get, "GET"),
        (Method::Post, "POST"),
        (Method::Put, "PUT"),
        (Method::Patch, "PATCH"),
        (Method::Delete, "DELETE"),
        (Method::Options, "OPTIONS"),
        (Method::Head, "HEAD"),
    ];

    #[test]
    fn as_str_matches_canonical_name_for_all_variants() {
        for (method, name) in ALL_METHODS {
            assert_eq!(method.as_str(), *name);
        }
    }

    #[test]
    fn display_matches_as_str_for_all_variants() {
        for (method, name) in ALL_METHODS {
            assert_eq!(method.to_string(), *name);
        }
    }

    #[test]
    fn from_str_round_trips_all_variants() {
        for (method, name) in ALL_METHODS {
            assert_eq!(Method::from_str(name), Ok(*method));
        }
    }

    #[test]
    fn as_str_then_from_str_is_identity() {
        for (method, _) in ALL_METHODS {
            assert_eq!(Method::from_str(method.as_str()), Ok(*method));
        }
    }

    #[test]
    fn from_str_is_case_sensitive() {
        assert!(Method::from_str("get").is_err());
        assert!(Method::from_str("Get").is_err());
        assert!(Method::from_str("gEt").is_err());
    }

    #[test]
    fn from_str_rejects_empty_and_whitespace() {
        assert!(Method::from_str("").is_err());
        assert!(Method::from_str(" GET").is_err());
        assert!(Method::from_str("GET ").is_err());
    }

    #[test]
    fn from_str_rejects_unknown_verb() {
        assert!(Method::from_str("BOGUS").is_err());
        assert!(Method::from_str("CONNECT").is_err());
        assert!(Method::from_str("TRACE").is_err());
    }

    #[test]
    fn parse_error_preserves_offending_input_in_display() {
        let err = Method::from_str("BOGUS").unwrap_err();
        let rendered = err.to_string();
        assert!(
            rendered.contains("BOGUS"),
            "expected error display to mention input, got: {rendered}"
        );
    }

    #[test]
    fn parse_error_implements_std_error() {
        fn assert_error<E: std::error::Error>(_: &E) {}
        let err: ParseMethodError = Method::from_str("nope").unwrap_err();
        assert_error(&err);
    }
}

mod status_code {
    use crate::StatusCode;

    const NAMED_VARIANTS: &[(StatusCode, u16, &str)] = &[
        (StatusCode::Ok, 200, "200 OK"),
        (StatusCode::Created, 201, "201 Created"),
        (StatusCode::NoContent, 204, "204 No Content"),
        (StatusCode::BadRequest, 400, "400 Bad Request"),
        (StatusCode::Unauthorized, 401, "401 Unauthorized"),
        (StatusCode::Forbidden, 403, "403 Forbidden"),
        (StatusCode::NotFound, 404, "404 Not Found"),
        (StatusCode::MethodNotAllowed, 405, "405 Method Not Allowed"),
        (StatusCode::Conflict, 409, "409 Conflict"),
        (
            StatusCode::InternalServerError,
            500,
            "500 Internal Server Error",
        ),
    ];

    #[test]
    fn from_u16_maps_each_named_code_to_its_variant() {
        for (variant, code, _) in NAMED_VARIANTS {
            assert_eq!(StatusCode::from_u16(*code), *variant);
        }
    }

    #[test]
    fn as_u16_returns_each_variants_canonical_code() {
        for (variant, code, _) in NAMED_VARIANTS {
            assert_eq!(variant.as_u16(), *code);
        }
    }

    #[test]
    fn from_u16_then_as_u16_round_trips_named_codes() {
        for (_, code, _) in NAMED_VARIANTS {
            assert_eq!(StatusCode::from_u16(*code).as_u16(), *code);
        }
    }

    #[test]
    fn unknown_codes_round_trip_through_other() {
        for code in [100u16, 301, 418, 429, 503, 599] {
            let s = StatusCode::from_u16(code);
            assert_eq!(s, StatusCode::Other(code));
            assert_eq!(s.as_u16(), code);
        }
    }

    #[test]
    fn other_with_named_code_value_still_round_trips_to_u16() {
        // Constructing Other(200) directly is unusual but the u16 round-trip
        // must hold because as_u16 reads the inner value.
        assert_eq!(StatusCode::Other(200).as_u16(), 200);
    }

    #[test]
    fn into_axum_status_matches_constants_for_each_named_variant() {
        let cases: &[(StatusCode, axum::http::StatusCode)] = &[
            (StatusCode::Ok, axum::http::StatusCode::OK),
            (StatusCode::Created, axum::http::StatusCode::CREATED),
            (StatusCode::NoContent, axum::http::StatusCode::NO_CONTENT),
            (StatusCode::BadRequest, axum::http::StatusCode::BAD_REQUEST),
            (StatusCode::Unauthorized, axum::http::StatusCode::UNAUTHORIZED),
            (StatusCode::Forbidden, axum::http::StatusCode::FORBIDDEN),
            (StatusCode::NotFound, axum::http::StatusCode::NOT_FOUND),
            (
                StatusCode::MethodNotAllowed,
                axum::http::StatusCode::METHOD_NOT_ALLOWED,
            ),
            (StatusCode::Conflict, axum::http::StatusCode::CONFLICT),
            (
                StatusCode::InternalServerError,
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (jolt, axum_status) in cases {
            let converted: axum::http::StatusCode = (*jolt).into();
            assert_eq!(converted, *axum_status);
        }
    }

    #[test]
    fn into_axum_status_preserves_other_in_range() {
        let converted: axum::http::StatusCode = StatusCode::Other(418).into();
        assert_eq!(converted.as_u16(), 418);
    }

    #[test]
    fn display_named_variants_use_canonical_reason_phrases() {
        for (variant, _, rendered) in NAMED_VARIANTS {
            assert_eq!(variant.to_string(), *rendered);
        }
    }

    #[test]
    fn display_other_with_known_code_uses_reason_phrase() {
        assert_eq!(StatusCode::Other(418).to_string(), "418 I'm a teapot");
    }

    #[test]
    fn display_other_with_unknown_code_falls_back_to_numeric() {
        // 599 is reserved/unassigned in the IANA registry; axum's
        // canonical_reason returns None, so Display should print just the code.
        assert_eq!(StatusCode::Other(599).to_string(), "599");
    }
}

mod request {
    use crate::{Cookie, Method, Request};
    use axum::http::{HeaderMap, HeaderName, HeaderValue};
    use std::collections::HashMap;

    fn empty_request() -> Request {
        Request {
            method: Method::Get,
            path: "/".to_string(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            body: Vec::new(),
            cookies: Vec::new(),
            finished: false,
        }
    }

    #[test]
    fn struct_literal_construction_reaches_every_field() {
        let req = Request {
            method: Method::Post,
            path: "/api/items".to_string(),
            headers: HeaderMap::new(),
            query_params: HashMap::from([("page".to_string(), "1".to_string())]),
            body: b"{}".to_vec(),
            cookies: vec![Cookie {
                name: "sid".to_string(),
                value: "abc".to_string(),
            }],
            finished: false,
        };

        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/api/items");
        assert!(req.headers.is_empty());
        assert_eq!(req.query_params.get("page").map(String::as_str), Some("1"));
        assert_eq!(req.body, b"{}");
        assert_eq!(req.cookies.len(), 1);
        assert_eq!(req.cookies[0].name, "sid");
        assert_eq!(req.cookies[0].value, "abc");
        assert!(!req.finished);
    }

    #[test]
    fn header_lookup_is_case_insensitive() {
        let mut req = empty_request();
        req.headers.insert(
            HeaderName::from_static("x-test"),
            HeaderValue::from_static("value"),
        );

        assert_eq!(req.header("x-test"), Some("value"));
        assert_eq!(req.header("X-Test"), Some("value"));
        assert_eq!(req.header("X-TEST"), Some("value"));
    }

    #[test]
    fn header_returns_none_for_missing_name() {
        let req = empty_request();
        assert_eq!(req.header("x-missing"), None);
    }

    #[test]
    fn header_returns_none_for_non_visible_ascii_value() {
        let mut req = empty_request();
        req.headers.insert(
            HeaderName::from_static("x-binary"),
            HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        );
        assert_eq!(req.header("x-binary"), None);
    }

    #[test]
    fn query_returns_value_for_present_key() {
        let mut req = empty_request();
        req.query_params
            .insert("page".to_string(), "1".to_string());
        assert_eq!(req.query("page"), Some("1"));
    }

    #[test]
    fn query_returns_none_for_missing_key() {
        let req = empty_request();
        assert_eq!(req.query("page"), None);
    }

    #[test]
    fn query_lookup_is_case_sensitive() {
        let mut req = empty_request();
        req.query_params
            .insert("Page".to_string(), "1".to_string());
        assert_eq!(req.query("page"), None);
        assert_eq!(req.query("Page"), Some("1"));
    }

    #[test]
    fn json_deserializes_body_into_struct() {
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct Item {
            name: String,
            count: u32,
        }

        let mut req = empty_request();
        req.body = br#"{"name":"widget","count":3}"#.to_vec();

        let parsed: Item = req.json().unwrap();
        assert_eq!(
            parsed,
            Item {
                name: "widget".to_string(),
                count: 3,
            }
        );
    }

    #[test]
    fn json_returns_err_for_malformed_body() {
        let mut req = empty_request();
        req.body = b"not json".to_vec();
        let result: serde_json::Result<serde_json::Value> = req.json();
        assert!(result.is_err());
    }

    #[test]
    fn json_returns_err_for_empty_body() {
        let req = empty_request();
        let result: serde_json::Result<serde_json::Value> = req.json();
        assert!(result.is_err());
    }

    #[test]
    fn cookie_returns_matching_entry() {
        let mut req = empty_request();
        req.cookies.push(Cookie {
            name: "sid".to_string(),
            value: "abc".to_string(),
        });
        req.cookies.push(Cookie {
            name: "theme".to_string(),
            value: "dark".to_string(),
        });

        let found = req.cookie("theme").unwrap();
        assert_eq!(found.name, "theme");
        assert_eq!(found.value, "dark");
    }

    #[test]
    fn cookie_returns_none_for_missing_name() {
        let req = empty_request();
        assert!(req.cookie("sid").is_none());
    }

    #[test]
    fn cookie_lookup_is_case_sensitive() {
        let mut req = empty_request();
        req.cookies.push(Cookie {
            name: "SID".to_string(),
            value: "abc".to_string(),
        });
        assert!(req.cookie("sid").is_none());
        assert!(req.cookie("SID").is_some());
    }

    #[test]
    fn cookie_returns_first_match_when_duplicate_names_exist() {
        let mut req = empty_request();
        req.cookies.push(Cookie {
            name: "sid".to_string(),
            value: "first".to_string(),
        });
        req.cookies.push(Cookie {
            name: "sid".to_string(),
            value: "second".to_string(),
        });
        assert_eq!(req.cookie("sid").unwrap().value, "first");
    }

    #[test]
    fn has_finished_defaults_to_false() {
        let req = empty_request();
        assert!(!req.has_finished());
    }

    #[test]
    fn has_finished_reflects_finished_flag() {
        let mut req = empty_request();
        req.finished = true;
        assert!(req.has_finished());
    }
}

mod response {
    use crate::{Response, StatusCode};

    #[test]
    fn new_constructs_response_with_given_status_and_body() {
        let res: Response<u32> = Response::new(StatusCode::Ok, 42);
        assert_eq!(res.status, StatusCode::Ok);
        assert_eq!(res.body, 42);
        assert!(res.headers.is_empty());
    }

    #[test]
    fn struct_literal_construction_reaches_every_field() {
        let res = Response {
            status: StatusCode::NoContent,
            headers: axum::http::HeaderMap::new(),
            body: (),
        };
        assert_eq!(res.status, StatusCode::NoContent);
        assert!(res.headers.is_empty());
        let _: () = res.body;
    }

    #[tokio::test]
    async fn into_axum_response_serializes_json_with_content_type_and_status() {
        use axum::http::header::CONTENT_TYPE;
        use serde_json::json;

        let jolt_response = Response::new(StatusCode::Ok, json!({"hello": "world"}));
        let axum_response: axum::response::Response = jolt_response.into();

        assert_eq!(axum_response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            axum_response.headers().get(CONTENT_TYPE).unwrap(),
            "application/json"
        );

        let body_bytes = axum::body::to_bytes(axum_response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&body_bytes[..], br#"{"hello":"world"}"#);
    }

    #[tokio::test]
    async fn into_axum_response_forwards_custom_headers() {
        use axum::http::header::CONTENT_TYPE;
        use axum::http::{HeaderName, HeaderValue};

        let mut jolt_response = Response::new(StatusCode::Created, 7u32);
        jolt_response.headers.insert(
            HeaderName::from_static("x-trace-id"),
            HeaderValue::from_static("abc-123"),
        );
        let axum_response: axum::response::Response = jolt_response.into();

        assert_eq!(axum_response.status(), axum::http::StatusCode::CREATED);
        assert_eq!(
            axum_response
                .headers()
                .get(HeaderName::from_static("x-trace-id"))
                .unwrap(),
            "abc-123"
        );
        assert_eq!(
            axum_response.headers().get(CONTENT_TYPE).unwrap(),
            "application/json"
        );
    }

    #[tokio::test]
    async fn into_axum_response_with_string_body_uses_text_plain() {
        use axum::http::header::CONTENT_TYPE;

        let jolt_response = Response::new(StatusCode::Ok, "hello".to_string());
        let axum_response: axum::response::Response = jolt_response.into();

        assert_eq!(axum_response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            axum_response.headers().get(CONTENT_TYPE).unwrap(),
            "text/plain; charset=utf-8"
        );

        let body_bytes = axum::body::to_bytes(axum_response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&body_bytes[..], b"hello");
    }

    #[tokio::test]
    async fn into_axum_response_with_str_body_uses_text_plain() {
        use axum::http::header::CONTENT_TYPE;

        let jolt_response = Response::new(StatusCode::Ok, "world");
        let axum_response: axum::response::Response = jolt_response.into();

        assert_eq!(axum_response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            axum_response.headers().get(CONTENT_TYPE).unwrap(),
            "text/plain; charset=utf-8"
        );

        let body_bytes = axum::body::to_bytes(axum_response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&body_bytes[..], b"world");
    }

    #[tokio::test]
    async fn into_axum_response_with_string_body_forwards_custom_headers() {
        use axum::http::{HeaderName, HeaderValue};

        let mut jolt_response = Response::new(StatusCode::Created, "ok".to_string());
        jolt_response.headers.insert(
            HeaderName::from_static("x-trace-id"),
            HeaderValue::from_static("zzz-9"),
        );
        let axum_response: axum::response::Response = jolt_response.into();

        assert_eq!(axum_response.status(), axum::http::StatusCode::CREATED);
        assert_eq!(
            axum_response
                .headers()
                .get(HeaderName::from_static("x-trace-id"))
                .unwrap(),
            "zzz-9"
        );
    }

    async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap()
            .to_vec()
    }

    #[tokio::test]
    async fn into_axum_response_with_unit_body_serializes_as_json_null() {
        use axum::http::header::CONTENT_TYPE;

        let response: axum::response::Response = Response::new(StatusCode::Ok, ()).into();

        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "application/json"
        );
        assert_eq!(body_bytes(response).await, b"null");
    }

    #[tokio::test]
    async fn into_axum_response_with_bool_body_serializes_as_json_boolean() {
        let true_response: axum::response::Response = Response::new(StatusCode::Ok, true).into();
        assert_eq!(body_bytes(true_response).await, b"true");

        let false_response: axum::response::Response = Response::new(StatusCode::Ok, false).into();
        assert_eq!(body_bytes(false_response).await, b"false");
    }

    #[tokio::test]
    async fn into_axum_response_with_integer_body_serializes_as_json_number() {
        let response: axum::response::Response = Response::new(StatusCode::Ok, 42i32).into();
        assert_eq!(body_bytes(response).await, b"42");
    }

    #[tokio::test]
    async fn into_axum_response_with_float_body_serializes_as_json_number() {
        let response: axum::response::Response = Response::new(StatusCode::Ok, 1.5f64).into();
        assert_eq!(body_bytes(response).await, b"1.5");
    }

    #[tokio::test]
    async fn into_axum_response_with_json_array_value_serializes_correctly() {
        use serde_json::json;

        let response: axum::response::Response =
            Response::new(StatusCode::Ok, json!([1, 2, 3])).into();
        assert_eq!(body_bytes(response).await, b"[1,2,3]");
    }

    #[tokio::test]
    async fn into_axum_response_with_user_defined_struct_body_serializes_as_json() {
        use axum::http::header::CONTENT_TYPE;
        use crate::JsonBody;

        #[derive(serde::Serialize)]
        struct Item {
            name: &'static str,
            count: u32,
        }
        impl JsonBody for Item {}

        let response: axum::response::Response = Response::new(
            StatusCode::Ok,
            Item {
                name: "widget",
                count: 3,
            },
        )
        .into();

        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "application/json"
        );
        assert_eq!(body_bytes(response).await, br#"{"name":"widget","count":3}"#);
    }

    #[tokio::test]
    async fn into_axum_response_with_empty_string_body_sends_empty_body() {
        use axum::http::header::CONTENT_TYPE;

        let response: axum::response::Response =
            Response::new(StatusCode::Ok, String::new()).into();

        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "text/plain; charset=utf-8"
        );
        assert!(body_bytes(response).await.is_empty());
    }

    #[tokio::test]
    async fn into_axum_response_with_empty_str_body_sends_empty_body() {
        let response: axum::response::Response = Response::new(StatusCode::Ok, "").into();
        assert!(body_bytes(response).await.is_empty());
    }

    #[tokio::test]
    async fn into_axum_response_status_passes_through_named_variants() {
        let cases: &[(StatusCode, axum::http::StatusCode)] = &[
            (StatusCode::Ok, axum::http::StatusCode::OK),
            (StatusCode::Created, axum::http::StatusCode::CREATED),
            (StatusCode::NoContent, axum::http::StatusCode::NO_CONTENT),
            (StatusCode::BadRequest, axum::http::StatusCode::BAD_REQUEST),
            (StatusCode::NotFound, axum::http::StatusCode::NOT_FOUND),
            (
                StatusCode::InternalServerError,
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (jolt, expected) in cases {
            let response: axum::response::Response = Response::new(*jolt, 0u32).into();
            assert_eq!(response.status(), *expected, "status mismatch for {jolt:?}");
        }
    }

    #[tokio::test]
    async fn into_axum_response_status_passes_through_other_variant() {
        let response: axum::response::Response =
            Response::new(StatusCode::Other(418), 0u32).into();
        assert_eq!(response.status().as_u16(), 418);
    }

    #[tokio::test]
    async fn into_axum_response_no_content_with_string_body_sends_body_as_is() {
        // Documents current behavior: the bridge does NOT enforce RFC 7230's
        // "204 MUST have empty body" — the caller's body is sent verbatim.
        // If enforcement is added later, this test should be updated, not deleted.
        let response: axum::response::Response =
            Response::new(StatusCode::NoContent, "leftover".to_string()).into();
        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
        assert_eq!(body_bytes(response).await, b"leftover");
    }

    #[tokio::test]
    async fn into_axum_response_overrides_caller_set_content_type() {
        use axum::http::header::CONTENT_TYPE;
        use axum::http::HeaderValue;

        let mut jolt_response = Response::new(StatusCode::Ok, 1u32);
        jolt_response
            .headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/xml"));
        let response: axum::response::Response = jolt_response.into();

        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "application/json"
        );
    }
}

mod request_ext {
    use crate::RequestExt;
    use std::sync::atomic::Ordering;

    #[test]
    fn new_constructs_with_finished_defaulting_to_false() {
        let ext = RequestExt::new();
        assert!(!ext.finished.load(Ordering::Relaxed));
    }

    #[test]
    fn new_into_inner_reports_false() {
        // Independent of any Ordering choice the load API picks: into_inner
        // consumes the atomic and returns the underlying bool directly.
        let ext = RequestExt::new();
        assert!(!ext.finished.into_inner());
    }

    #[test]
    fn is_finished_is_false_after_construction() {
        let ext = RequestExt::new();
        assert!(!ext.is_finished());
    }

    #[test]
    fn mark_finished_flips_is_finished_to_true() {
        let ext = RequestExt::new();
        ext.mark_finished();
        assert!(ext.is_finished());
    }

    #[test]
    fn mark_finished_is_idempotent() {
        // Latch semantics: repeated calls must not regress the flag.
        let ext = RequestExt::new();
        ext.mark_finished();
        ext.mark_finished();
        assert!(ext.is_finished());
    }

    #[test]
    fn mark_finished_takes_shared_reference() {
        // Locks in &self (not &mut self): the whole point of AtomicBool is
        // shared mutation across middleware layers without &mut threading.
        let ext = RequestExt::new();
        let shared = &ext;
        shared.mark_finished();
        assert!(shared.is_finished());
    }

    #[test]
    fn is_finished_observes_direct_field_store() {
        // Proves is_finished() reads the same atomic the public field exposes —
        // a future refactor that introduces a shadow field would fail this.
        let ext = RequestExt::new();
        ext.finished.store(true, Ordering::Relaxed);
        assert!(ext.is_finished());
    }
}

mod request_ext_extensions {
    //! `axum::http::Extensions` requires `T: Clone + Send + Sync + 'static`.
    //! `RequestExt` is intentionally `!Clone` (AtomicBool's interior mutability
    //! makes a per-field clone silently break the shared-state invariant), so
    //! the canonical embedding is `Arc<RequestExt>`. Cloning the Arc still
    //! points at the same atomic — verified by `mark_through_one_handle_visible_through_another`.

    use crate::RequestExt;
    use axum::http::Extensions;
    use std::sync::Arc;

    #[test]
    fn insert_then_get_returns_same_instance() {
        let mut extensions = Extensions::new();
        let inserted = Arc::new(RequestExt::new());
        let inserted_ptr = Arc::as_ptr(&inserted);
        extensions.insert(inserted);

        let retrieved = extensions
            .get::<Arc<RequestExt>>()
            .expect("Arc<RequestExt> should be retrievable after insert");
        assert!(std::ptr::eq(Arc::as_ptr(retrieved), inserted_ptr));
    }

    #[test]
    fn mark_finished_through_extensions_is_observable_on_retrieval() {
        // The PRD-mandated verification: insert RequestExt into Extensions,
        // retrieve, mark finished, verify.
        let mut extensions = Extensions::new();
        extensions.insert(Arc::new(RequestExt::new()));

        let retrieved = extensions
            .get::<Arc<RequestExt>>()
            .expect("Arc<RequestExt> should be retrievable after insert");
        assert!(!retrieved.is_finished());
        retrieved.mark_finished();

        let retrieved_again = extensions
            .get::<Arc<RequestExt>>()
            .expect("Arc<RequestExt> should still be retrievable");
        assert!(retrieved_again.is_finished());
    }

    #[test]
    fn get_returns_none_when_not_inserted() {
        let extensions = Extensions::new();
        assert!(extensions.get::<Arc<RequestExt>>().is_none());
    }

    #[test]
    fn mark_through_one_handle_visible_through_another() {
        // Locks in shared-state semantics: a clone of the Arc points at the
        // same atomic, so mutation via one handle is observable via another.
        // This is the property that makes `Arc<RequestExt>` the correct
        // embedding (instead of a per-field Clone impl that would silently
        // duplicate state).
        let mut extensions = Extensions::new();
        let outside_handle = Arc::new(RequestExt::new());
        extensions.insert(Arc::clone(&outside_handle));

        let inside_handle = extensions.get::<Arc<RequestExt>>().unwrap();
        inside_handle.mark_finished();

        assert!(outside_handle.is_finished());
    }
}

mod endpoint {
    //! PRD JOLT-RS-028 verification ("Trait compiles") plus the load-bearing
    //! contracts the registry layer (JOLT-RS-029) and Router layer
    //! (JOLT-RS-033..) will lean on: object-safety with `Send + Sync`, and a
    //! handler future that actually resolves to an axum response.

    use crate::{Endpoint, EndpointFuture, Method, Request};
    use axum::body::Body;
    use axum::http::HeaderMap;
    use std::collections::HashMap;

    struct StaticHello;

    impl Endpoint for StaticHello {
        fn path(&self) -> &str {
            "/hello"
        }

        fn method(&self) -> Method {
            Method::Get
        }

        fn handler(&self, _req: Request) -> EndpointFuture {
            Box::pin(async {
                axum::response::Response::builder()
                    .status(axum::http::StatusCode::OK)
                    .body(Body::from("hello"))
                    .unwrap()
            })
        }
    }

    fn empty_request() -> Request {
        Request {
            method: Method::Get,
            path: "/hello".to_string(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            body: Vec::new(),
            cookies: Vec::new(),
            finished: false,
        }
    }

    #[test]
    fn trait_is_object_safe_with_send_sync_bounds() {
        // Locks in JOLT-RS-029's prerequisite: the registry stores
        // `Box<dyn Endpoint + Send + Sync>`. If a future change adds an
        // associated type or generic method to the trait, this assignment
        // stops compiling — exactly the regression we want to catch here.
        let endpoint: Box<dyn Endpoint + Send + Sync> = Box::new(StaticHello);
        assert_eq!(endpoint.path(), "/hello");
        assert_eq!(endpoint.method(), Method::Get);
    }

    #[tokio::test]
    async fn handler_future_resolves_to_axum_response() {
        let endpoint = StaticHello;
        let response = endpoint.handler(empty_request()).await;

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&body_bytes[..], b"hello");
    }
}

mod endpoint_registry {
    //! PRD JOLT-RS-029 verification ("register two endpoints, registry has
    //! length 2") plus the empty-registry contract that JOLT-RS-031's
    //! build_router() will rely on.

    use crate::{Endpoint, EndpointFuture, EndpointRegistry, Method, Request};
    use axum::body::Body;

    struct Stub {
        path: &'static str,
        method: Method,
    }

    impl Endpoint for Stub {
        fn path(&self) -> &str {
            self.path
        }

        fn method(&self) -> Method {
            self.method
        }

        fn handler(&self, _req: Request) -> EndpointFuture {
            Box::pin(async {
                axum::response::Response::builder()
                    .status(axum::http::StatusCode::OK)
                    .body(Body::empty())
                    .unwrap()
            })
        }
    }

    #[test]
    fn new_registry_is_empty() {
        let registry = EndpointRegistry::new();
        assert_eq!(registry.len(), 0);
        assert!(registry.is_empty());
    }

    #[test]
    fn register_two_endpoints_yields_length_two() {
        // PRD-mandated verification for JOLT-RS-029.
        let mut registry = EndpointRegistry::new();
        registry.register(Stub {
            path: "/a",
            method: Method::Get,
        });
        registry.register(Stub {
            path: "/b",
            method: Method::Post,
        });
        assert_eq!(registry.len(), 2);
        assert!(!registry.is_empty());
    }

    #[test]
    fn default_matches_new() {
        // Default impl is what `JoltServer` will use to embed an empty
        // registry without an explicit `EndpointRegistry::new()` call.
        let registry = EndpointRegistry::default();
        assert!(registry.is_empty());
    }
}

mod server {
    use crate::{CorsConfig, JoltServer};

    #[test]
    fn new_uses_default_port_8080() {
        // PRD-mandated verification for JOLT-RS-023: defaults include port=8080.
        let server = JoltServer::new();
        assert_eq!(server.port, 8080);
    }

    #[test]
    fn builder_chain_sets_port_and_threads() {
        // PRD-mandated verification for JOLT-RS-024: chained .port(3000).threads(4) sets values.
        let server = JoltServer::new().port(3000).threads(4);
        assert_eq!(server.port, 3000);
        assert_eq!(server.threads, 4);
    }

    #[test]
    fn cors_builder_wraps_arg_in_some() {
        let server = JoltServer::new().cors(CorsConfig);
        assert!(server.cors_config.is_some());
    }

    #[test]
    fn builder_methods_preserve_unspecified_defaults() {
        // .port() must not clobber threads, and vice versa.
        let default_threads = JoltServer::new().threads;
        let server = JoltServer::new().port(9090);
        assert_eq!(server.port, 9090);
        assert_eq!(server.threads, default_threads);
        assert!(server.cors_config.is_none());
        assert!(server.tls_config.is_none());
    }

    #[tokio::test]
    async fn start_binds_and_serves_404_on_empty_router() {
        // PRD-mandated verification for JOLT-RS-025 ("server starts, curl
        // localhost:8080 returns 404"), automated via a port-0 probe so the
        // test is hermetic and doesn't collide with anything actually on 8080.
        use axum::Router;
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::{TcpListener, TcpStream};
        use tokio::time::timeout;

        // Bind 127.0.0.1:0, capture the OS-assigned port, then drop the probe
        // so `start` can claim it. The few-microsecond TOCTOU window is
        // acceptable for a unit test.
        let probe = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let server = tokio::spawn(JoltServer::new().port(port).start(Router::new()));

        // Retry-connect: `start` returns a future that hasn't yet bound at
        // spawn time; it takes a few ms to be ready. 50 × 20ms = ~1s budget.
        let mut stream = None;
        for _ in 0..50 {
            if let Ok(s) = TcpStream::connect(("127.0.0.1", port)).await {
                stream = Some(s);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let mut stream =
            stream.expect("JoltServer::start should accept connections within 1s");

        timeout(
            Duration::from_secs(2),
            stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n"),
        )
        .await
        .expect("HTTP request write should not time out")
        .unwrap();

        let mut buf = [0u8; 64];
        let n = timeout(Duration::from_secs(2), stream.read(&mut buf))
            .await
            .expect("HTTP response read should not time out")
            .unwrap();
        let response = String::from_utf8_lossy(&buf[..n]);
        assert!(
            response.starts_with("HTTP/1.1 404"),
            "expected 404 on empty router, got: {response}"
        );

        server.abort();
    }
}
