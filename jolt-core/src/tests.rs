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

    #[test]
    fn take_response_returns_none_when_no_response_stashed() {
        // JOLT-RS-035: the stash defaults to empty so a finished-without-stash
        // request can be distinguished from a finished-with-stash one. The
        // caller (Router::call) decides what fallback to send when the stash
        // is empty.
        let ext = RequestExt::new();
        assert!(ext.take_response().is_none());
    }

    #[test]
    fn set_response_then_take_response_yields_the_stashed_response() {
        use axum::body::Body;
        use axum::http::StatusCode;

        let ext = RequestExt::new();
        let response = axum::response::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::empty())
            .unwrap();
        ext.set_response(response);

        let taken = ext.take_response().expect("response was stashed");
        assert_eq!(taken.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn take_response_drains_the_stash() {
        // Take is destructive: a second take must observe an empty stash so
        // the Router doesn't accidentally reuse a response that has already
        // been emitted.
        use axum::body::Body;
        use axum::http::StatusCode;

        let ext = RequestExt::new();
        ext.set_response(
            axum::response::Response::builder()
                .status(StatusCode::OK)
                .body(Body::empty())
                .unwrap(),
        );
        let _first = ext.take_response().expect("first take returns response");
        assert!(
            ext.take_response().is_none(),
            "second take must observe an empty stash"
        );
    }

    #[test]
    fn set_response_replaces_a_previously_stashed_response() {
        // Locks in last-writer-wins semantics so a later layer that wants to
        // override an earlier layer's stash (e.g. an error mapper running
        // after auth) doesn't have to reach into the option directly.
        use axum::body::Body;
        use axum::http::StatusCode;

        let ext = RequestExt::new();
        ext.set_response(
            axum::response::Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .unwrap(),
        );
        ext.set_response(
            axum::response::Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::empty())
                .unwrap(),
        );

        let taken = ext.take_response().expect("response was stashed");
        assert_eq!(taken.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn set_response_does_not_implicitly_mark_finished() {
        // mark_finished and set_response are deliberately independent so a
        // middleware can pre-stash a response (e.g. a default body) without
        // committing to short-circuit.
        use axum::body::Body;
        use axum::http::StatusCode;

        let ext = RequestExt::new();
        ext.set_response(
            axum::response::Response::builder()
                .status(StatusCode::OK)
                .body(Body::empty())
                .unwrap(),
        );
        assert!(!ext.is_finished());
    }

    #[test]
    fn mark_finished_does_not_clear_a_previously_stashed_response() {
        // Symmetric to the set_response/mark_finished independence above:
        // marking finished must preserve any stash so the canonical pattern
        // (set_response(..); mark_finished();) does not race the latch
        // against the stash.
        use axum::body::Body;
        use axum::http::StatusCode;

        let ext = RequestExt::new();
        ext.set_response(
            axum::response::Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .unwrap(),
        );
        ext.mark_finished();
        assert!(ext.is_finished());
        let taken = ext.take_response().expect("stash survived mark_finished");
        assert_eq!(taken.status(), StatusCode::UNAUTHORIZED);
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

    #[test]
    fn sort_orders_paths_by_length_descending() {
        // PRD-mandated verification for JOLT-RS-030: registering ["/", "/api/hello", "/api"]
        // and calling sort yields ["/api/hello", "/api", "/"]. Longest-first matters
        // because JOLT-RS-031's `build_router` will pick the first matching route, so
        // `/api/hello` must be checked before its `/api` prefix.
        let mut registry = EndpointRegistry::new();
        registry.register(Stub {
            path: "/",
            method: Method::Get,
        });
        registry.register(Stub {
            path: "/api/hello",
            method: Method::Get,
        });
        registry.register(Stub {
            path: "/api",
            method: Method::Get,
        });
        registry.sort();
        let paths: Vec<&str> = registry.iter().map(Endpoint::path).collect();
        assert_eq!(paths, vec!["/api/hello", "/api", "/"]);
    }

    #[test]
    fn sort_is_stable_for_equal_length_paths() {
        // Vec::sort_by_key is documented stable; this test pins that contract so
        // a future swap to an unstable sort (e.g. sort_unstable_by_key) trips here
        // rather than producing nondeterministic route order.
        let mut registry = EndpointRegistry::new();
        registry.register(Stub {
            path: "/aaa",
            method: Method::Get,
        });
        registry.register(Stub {
            path: "/bbb",
            method: Method::Get,
        });
        registry.register(Stub {
            path: "/ccc",
            method: Method::Get,
        });
        registry.sort();
        let paths: Vec<&str> = registry.iter().map(Endpoint::path).collect();
        assert_eq!(paths, vec!["/aaa", "/bbb", "/ccc"]);
    }

    #[test]
    fn sort_on_empty_registry_is_a_noop() {
        // Guards the empty-registry contract that JOLT-RS-031's build_router
        // also relies on; calling sort on zero entries must not panic.
        let mut registry = EndpointRegistry::new();
        registry.sort();
        assert!(registry.is_empty());
    }

    /// JOLT-RS-031 verification: build_router from two endpoints produces an
    /// axum Router whose routes dispatch to the matching endpoint handler.
    /// Driven via `tower::ServiceExt::oneshot` so the test stays fully
    /// in-process — no TCP bind, no signal handling.
    mod build_router {
        use crate::{Endpoint, EndpointFuture, EndpointRegistry, Method, Request};
        use axum::body::{to_bytes, Body};
        use axum::http::{Request as AxumRequest, StatusCode};
        use tower::ServiceExt;

        struct EchoEndpoint {
            path: &'static str,
            method: Method,
            body: &'static str,
        }

        impl Endpoint for EchoEndpoint {
            fn path(&self) -> &str {
                self.path
            }

            fn method(&self) -> Method {
                self.method
            }

            fn handler(&self, _req: Request) -> EndpointFuture {
                let body = self.body;
                Box::pin(async move {
                    axum::response::Response::builder()
                        .status(StatusCode::OK)
                        .body(Body::from(body))
                        .unwrap()
                })
            }
        }

        struct CapturingEndpoint;

        impl Endpoint for CapturingEndpoint {
            fn path(&self) -> &str {
                "/capture"
            }

            fn method(&self) -> Method {
                Method::Post
            }

            fn handler(&self, req: Request) -> EndpointFuture {
                let body = format!(
                    "method={} path={} q_n={} body_len={} cookie={}",
                    req.method,
                    req.path,
                    req.query("n").unwrap_or("?"),
                    req.body.len(),
                    req.cookie("sid").map(|c| c.value.as_str()).unwrap_or("?"),
                );
                Box::pin(async move {
                    axum::response::Response::builder()
                        .status(StatusCode::OK)
                        .body(Body::from(body))
                        .unwrap()
                })
            }
        }

        async fn body_string(resp: axum::response::Response) -> String {
            let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
            String::from_utf8(bytes.to_vec()).unwrap()
        }

        #[tokio::test]
        async fn empty_registry_yields_router_that_404s() {
            let registry = EndpointRegistry::new();
            let router = registry.build_router();
            let req = AxumRequest::builder()
                .method("GET")
                .uri("/anything")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn two_endpoints_each_dispatch_correctly() {
            // PRD-mandated verification for JOLT-RS-031: two endpoints, two
            // working routes. Distinct method+path pairs prove dispatch is
            // keyed on both, not just path.
            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/foo",
                method: Method::Get,
                body: "FOO",
            });
            registry.register(EchoEndpoint {
                path: "/bar",
                method: Method::Post,
                body: "BAR",
            });
            let router = registry.build_router();

            let req = AxumRequest::builder()
                .method("GET")
                .uri("/foo")
                .body(Body::empty())
                .unwrap();
            let resp = router.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(body_string(resp).await, "FOO");

            let req = AxumRequest::builder()
                .method("POST")
                .uri("/bar")
                .body(Body::empty())
                .unwrap();
            let resp = router.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(body_string(resp).await, "BAR");
        }

        #[tokio::test]
        async fn wrong_method_for_registered_path_is_405() {
            // axum's MethodRouter answers 405 (not 404) when the path matches
            // but the verb doesn't. Locking in that contract here so a future
            // build_router refactor can't silently downgrade it to a 404.
            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/only-get",
                method: Method::Get,
                body: "ok",
            });
            let router = registry.build_router();
            let req = AxumRequest::builder()
                .method("POST")
                .uri("/only-get")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        }

        #[tokio::test]
        async fn handler_sees_method_path_query_body_and_cookie() {
            // Locks in the cross-phase glue: the axum→Jolt request conversion
            // populates method/path/query_params/body/cookies before invoking
            // the handler. If a future refactor of build_jolt_request drops
            // any of these, this single test trips.
            let mut registry = EndpointRegistry::new();
            registry.register(CapturingEndpoint);
            let router = registry.build_router();
            let req = AxumRequest::builder()
                .method("POST")
                .uri("/capture?n=42")
                .header("Cookie", "sid=abc123")
                .body(Body::from("hello"))
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(
                body_string(resp).await,
                "method=POST path=/capture q_n=42 body_len=5 cookie=abc123"
            );
        }
    }
}

mod router {
    //! PRD JOLT-RS-033 verification ("Service impl compiles") plus a behavioral
    //! check that the wrapper attaches a fresh `Arc<RequestExt>` to every
    //! incoming request before the inner axum router sees it. The injection
    //! contract is what JOLT-RS-035's finished-flag short-circuit reads from;
    //! locking it down here means a regression in 033 surfaces as a focused
    //! failure rather than a confusing 035 test break later.

    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::Request as AxumRequest;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    use crate::router::Router;
    use crate::RequestExt;

    #[tokio::test]
    async fn service_call_injects_request_ext_into_extensions() {
        // The probe handler returns 200 if the extension is present and 500
        // otherwise — turns the assertion into the response status so the test
        // exercises the full Service::call path (not just a side-channel poke).
        let inner = axum::Router::new().route(
            "/probe",
            axum::routing::get(|req: AxumRequest| async move {
                match req.extensions().get::<Arc<RequestExt>>() {
                    Some(ext) => {
                        assert!(!ext.is_finished(), "freshly-injected RequestExt must default to not-finished");
                        StatusCode::OK
                    }
                    None => StatusCode::INTERNAL_SERVER_ERROR,
                }
            }),
        );
        let router = Router::from_axum(inner);

        let req = AxumRequest::builder()
            .uri("/probe")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn service_call_preserves_caller_supplied_request_ext() {
        // JOLT-RS-035 inverted the JOLT-RS-033 contract: Router now PRESERVES a
        // caller-supplied `Arc<RequestExt>` so an outer tower layer's
        // `mark_finished()` is observable by Router's dispatch loop. The
        // identity check (`Arc::ptr_eq`) is the load-bearing assertion; without
        // it, a regression that re-introduces the overwrite would still
        // produce a not-finished latch (the value is correct by coincidence)
        // and pass a value-only assertion.
        let captured: Arc<std::sync::Mutex<Option<Arc<RequestExt>>>> =
            Arc::new(std::sync::Mutex::new(None));
        let captured_in_handler = Arc::clone(&captured);
        let inner = axum::Router::new().route(
            "/probe",
            axum::routing::get(move |req: AxumRequest| {
                let captured = Arc::clone(&captured_in_handler);
                async move {
                    let ext = req
                        .extensions()
                        .get::<Arc<RequestExt>>()
                        .expect("RequestExt must be present")
                        .clone();
                    *captured.lock().unwrap() = Some(ext);
                    StatusCode::OK
                }
            }),
        );
        let router = Router::from_axum(inner);

        let outer_handle = Arc::new(RequestExt::new());
        let outer_ptr = Arc::as_ptr(&outer_handle);

        let mut req = AxumRequest::builder()
            .uri("/probe")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&outer_handle));

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let handler_handle = captured
            .lock()
            .unwrap()
            .take()
            .expect("handler must observe a RequestExt");
        assert!(
            std::ptr::eq(Arc::as_ptr(&handler_handle), outer_ptr),
            "Router::call must preserve the caller-supplied Arc<RequestExt> so middleware mark_finished() is observable downstream"
        );
    }

    #[tokio::test]
    async fn service_call_inserts_fresh_request_ext_when_none_supplied() {
        // The other half of the preserve-or-inject contract: when no upstream
        // layer has placed an `Arc<RequestExt>` in extensions, Router must
        // insert one so handlers can rely on the extension being present.
        let inner = axum::Router::new().route(
            "/probe",
            axum::routing::get(|req: AxumRequest| async move {
                match req.extensions().get::<Arc<RequestExt>>() {
                    Some(ext) => {
                        assert!(
                            !ext.is_finished(),
                            "freshly-injected RequestExt must default to not-finished"
                        );
                        StatusCode::OK
                    }
                    None => StatusCode::INTERNAL_SERVER_ERROR,
                }
            }),
        );
        let router = Router::from_axum(inner);

        let req = AxumRequest::builder()
            .uri("/probe")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn service_call_forwards_to_inner_axum_router_for_unmatched_paths() {
        // The wrapper is supposed to be transparent on the routing dimension:
        // unmatched paths still 404 via the inner axum Router, not bypassed.
        let inner = axum::Router::new();
        let router = Router::from_axum(inner);

        let req = AxumRequest::builder()
            .uri("/missing")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    mod registry_dispatch {
        //! PRD JOLT-RS-034 verification: `Router::call` parses path + method
        //! from the inbound request, walks the (sorted) endpoint registry, and
        //! either dispatches to a matching handler or returns 404.
        //!
        //! 405 method-mismatch differentiation is JOLT-RS-037's territory; per
        //! 034's step text ("On no match, return 404."), unknown paths AND
        //! method mismatches both surface as 404 here. The
        //! `method_mismatch_returns_404` test pins that contract so the 037
        //! refinement is intentional rather than accidental.

        use std::sync::Arc;

        use axum::body::{to_bytes, Body};
        use axum::extract::Request as AxumRequest;
        use axum::http::StatusCode;
        use tower::ServiceExt;

        use crate::router::Router;
        use crate::{Endpoint, EndpointFuture, EndpointRegistry, Method, Request};

        struct EchoEndpoint {
            path: &'static str,
            method: Method,
            body: &'static str,
        }

        impl Endpoint for EchoEndpoint {
            fn path(&self) -> &str {
                self.path
            }

            fn method(&self) -> Method {
                self.method
            }

            fn handler(&self, _req: Request) -> EndpointFuture {
                let body = self.body;
                Box::pin(async move {
                    axum::response::Response::builder()
                        .status(StatusCode::OK)
                        .body(Body::from(body))
                        .unwrap()
                })
            }
        }

        async fn body_string(resp: axum::response::Response) -> String {
            let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
            String::from_utf8(bytes.to_vec()).unwrap()
        }

        #[tokio::test]
        async fn registered_get_hello_returns_200() {
            // PRD-mandated verification (verbatim half 1): registered GET
            // /hello → GET /hello returns 200, with the handler body flowing
            // through.
            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/hello",
                method: Method::Get,
                body: "hi",
            });
            let router = Router::from_registry(registry);

            let req = AxumRequest::builder()
                .method("GET")
                .uri("/hello")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(body_string(resp).await, "hi");
        }

        #[tokio::test]
        async fn unknown_path_returns_404() {
            // PRD-mandated verification (verbatim half 2): GET /unknown → 404.
            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/hello",
                method: Method::Get,
                body: "hi",
            });
            let router = Router::from_registry(registry);

            let req = AxumRequest::builder()
                .method("GET")
                .uri("/unknown")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn empty_registry_returns_404_for_any_request() {
            // The walk-and-miss path with zero entries is structurally
            // distinct from "walked all entries, none matched"; both must
            // produce the same 404 outcome.
            let router = Router::from_registry(EndpointRegistry::new());
            let req = AxumRequest::builder()
                .method("GET")
                .uri("/anything")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn method_mismatch_on_registered_path_returns_405() {
            // PRD JOLT-RS-037 verification: a registered path hit with the
            // wrong verb surfaces 405 (not 404). This inverts the provisional
            // 034-era contract pinned by `method_mismatch_on_registered_path
            // _returns_404`; the registry walk now collects path-match-method-
            // miss entries and surfaces 405 if no verb matched. RFC 9110
            // §15.5.6 distinguishes "no resource at this path" from "resource
            // exists, method not supported," and Jolt's router now honors that
            // distinction.
            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/hello",
                method: Method::Get,
                body: "hi",
            });
            let router = Router::from_registry(registry);

            let req = AxumRequest::builder()
                .method("POST")
                .uri("/hello")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        }

        #[tokio::test]
        async fn method_mismatch_returns_405_with_allow_header_listing_registered_method() {
            // Pins RFC 9110 §15.5.6's "MUST advertise allowed methods" via
            // the `Allow` header. Status-only assertion would let a regression
            // that returned 405 with no Allow header pass (and clients that
            // rely on Allow to retry with the correct verb would silently
            // break).
            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/hello",
                method: Method::Get,
                body: "hi",
            });
            let router = Router::from_registry(registry);

            let req = AxumRequest::builder()
                .method("POST")
                .uri("/hello")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
            let allow = resp
                .headers()
                .get("Allow")
                .expect("405 must include Allow header per RFC 9110 §15.5.6")
                .to_str()
                .expect("Allow header must be ASCII");
            assert_eq!(allow, "GET");
        }

        #[tokio::test]
        async fn method_mismatch_returns_405_with_allow_listing_all_registered_methods() {
            // When the same path is registered under multiple verbs, the
            // Allow header must enumerate ALL of them so a client can retry
            // the request with any supported verb. Insertion order is
            // load-bearing: stable `sort_by_key` in `EndpointRegistry::sort`
            // preserves order among same-path entries, so the listing is
            // deterministic. A regression that switched to an unstable sort
            // (or reordered entries) would surface here.
            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/hello",
                method: Method::Get,
                body: "hi",
            });
            registry.register(EchoEndpoint {
                path: "/hello",
                method: Method::Post,
                body: "posted",
            });
            let router = Router::from_registry(registry);

            let req = AxumRequest::builder()
                .method("DELETE")
                .uri("/hello")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
            let allow = resp
                .headers()
                .get("Allow")
                .expect("405 must include Allow header per RFC 9110 §15.5.6")
                .to_str()
                .expect("Allow header must be ASCII");
            assert_eq!(allow, "GET, POST");
        }

        #[tokio::test]
        async fn unknown_path_returns_404_not_405_even_when_registry_is_populated() {
            // The 405 refinement must NOT bleed into unknown-path responses:
            // 405 is reserved for "path matches a registered route, verb
            // doesn't." A different path — even with verb that's used
            // elsewhere — must still 404. Without this test, a regression
            // that flipped the inner conditional ("if endpoint.method() ==
            // method") would still pass the unknown-path test but break the
            // path-discrimination contract.
            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/hello",
                method: Method::Get,
                body: "hi",
            });
            let router = Router::from_registry(registry);

            let req = AxumRequest::builder()
                .method("GET")
                .uri("/missing")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
            assert!(
                resp.headers().get("Allow").is_none(),
                "404 must NOT include Allow header — that's reserved for 405"
            );
        }

        #[tokio::test]
        async fn unparseable_http_verb_returns_404_even_for_registered_path() {
            // Pins the conservative-404 contract for HTTP verbs Jolt's
            // `Method` enum doesn't recognize (e.g. CONNECT, TRACE). Returning
            // 405 here would require enumerating Jolt's vocabulary to a
            // caller that already sent something Jolt doesn't speak; 404
            // keeps the fingerprint surface small. If a future PRD wants 501
            // Not Implemented (RFC 9110 §15.6.2), this test pins the current
            // behavior so the change is intentional.
            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/hello",
                method: Method::Get,
                body: "hi",
            });
            let router = Router::from_registry(registry);

            let req = AxumRequest::builder()
                .method("CONNECT")
                .uri("/hello")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn finished_flag_with_stashed_response_short_circuits_handler() {
            // PRD-mandated verification for JOLT-RS-035: an "auth middleware"
            // (simulated here as a pre-inserted Arc<RequestExt> with a stashed
            // 401 + the finished latch set) → Router takes the stash and
            // returns it without invoking the matched endpoint's handler.
            // The handler tracks invocation via an AtomicBool so the test
            // distinguishes "handler skipped" from "handler ran but returned
            // 401."
            use std::sync::atomic::{AtomicBool, Ordering};

            use crate::request_ext::RequestExt;

            struct TrackingEndpoint {
                invoked: Arc<AtomicBool>,
            }

            impl Endpoint for TrackingEndpoint {
                fn path(&self) -> &str {
                    "/protected"
                }

                fn method(&self) -> Method {
                    Method::Get
                }

                fn handler(&self, _req: Request) -> EndpointFuture {
                    let invoked = Arc::clone(&self.invoked);
                    Box::pin(async move {
                        invoked.store(true, Ordering::Relaxed);
                        axum::response::Response::builder()
                            .status(StatusCode::OK)
                            .body(Body::empty())
                            .unwrap()
                    })
                }
            }

            let invoked = Arc::new(AtomicBool::new(false));
            let mut registry = EndpointRegistry::new();
            registry.register(TrackingEndpoint {
                invoked: Arc::clone(&invoked),
            });
            let router = Router::from_registry(registry);

            let ext = Arc::new(RequestExt::new());
            ext.set_response(
                axum::response::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(Body::from("auth required"))
                    .unwrap(),
            );
            ext.mark_finished();

            let mut req = AxumRequest::builder()
                .method("GET")
                .uri("/protected")
                .body(Body::empty())
                .unwrap();
            req.extensions_mut().insert(Arc::clone(&ext));

            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(body_string(resp).await, "auth required");
            assert!(
                !invoked.load(Ordering::Relaxed),
                "handler must not be invoked when RequestExt is finished"
            );
        }

        #[tokio::test]
        async fn finished_flag_without_stashed_response_falls_back_to_500() {
            // Defensive contract: a middleware that calls mark_finished()
            // without stashing a response leaves Router with no body to send.
            // Surfacing 500 (rather than 200 or whatever the handler would
            // have returned) makes the bug visible at the boundary instead of
            // letting it propagate as a misleading success.
            use std::sync::atomic::{AtomicBool, Ordering};

            use crate::request_ext::RequestExt;

            struct TrackingEndpoint {
                invoked: Arc<AtomicBool>,
            }

            impl Endpoint for TrackingEndpoint {
                fn path(&self) -> &str {
                    "/protected"
                }

                fn method(&self) -> Method {
                    Method::Get
                }

                fn handler(&self, _req: Request) -> EndpointFuture {
                    let invoked = Arc::clone(&self.invoked);
                    Box::pin(async move {
                        invoked.store(true, Ordering::Relaxed);
                        axum::response::Response::builder()
                            .status(StatusCode::OK)
                            .body(Body::empty())
                            .unwrap()
                    })
                }
            }

            let invoked = Arc::new(AtomicBool::new(false));
            let mut registry = EndpointRegistry::new();
            registry.register(TrackingEndpoint {
                invoked: Arc::clone(&invoked),
            });
            let router = Router::from_registry(registry);

            let ext = Arc::new(RequestExt::new());
            ext.mark_finished();

            let mut req = AxumRequest::builder()
                .method("GET")
                .uri("/protected")
                .body(Body::empty())
                .unwrap();
            req.extensions_mut().insert(Arc::clone(&ext));

            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
            assert!(
                !invoked.load(Ordering::Relaxed),
                "handler must not be invoked when RequestExt is finished"
            );
        }

        #[tokio::test]
        async fn caller_supplied_not_finished_request_ext_dispatches_normally() {
            // The third leaf of the JOLT-RS-035 contract: a caller-supplied
            // not-finished RequestExt must NOT short-circuit. Otherwise the
            // preserve-existing semantics would silently break every request
            // that flows through middleware in the not-failed case.
            use crate::request_ext::RequestExt;

            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/hello",
                method: Method::Get,
                body: "hi",
            });
            let router = Router::from_registry(registry);

            let ext = Arc::new(RequestExt::new());

            let mut req = AxumRequest::builder()
                .method("GET")
                .uri("/hello")
                .body(Body::empty())
                .unwrap();
            req.extensions_mut().insert(Arc::clone(&ext));

            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(body_string(resp).await, "hi");
        }

        #[tokio::test]
        async fn finished_flag_short_circuits_even_when_path_does_not_match() {
            // Pins the early-out: the finished check runs BEFORE the registry
            // walk so a finishing middleware's response is the source of truth
            // regardless of which (or whether) a route would have matched. The
            // alternative (walk-then-check) would leak a 404 in any
            // finishing-middleware-without-route path.
            use crate::request_ext::RequestExt;

            let registry = EndpointRegistry::new();
            let router = Router::from_registry(registry);

            let ext = Arc::new(RequestExt::new());
            ext.set_response(
                axum::response::Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(Body::empty())
                    .unwrap(),
            );
            ext.mark_finished();

            let mut req = AxumRequest::builder()
                .method("GET")
                .uri("/no-route")
                .body(Body::empty())
                .unwrap();
            req.extensions_mut().insert(Arc::clone(&ext));

            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        }

        #[tokio::test]
        async fn longest_path_wins_when_multiple_registered() {
            // `from_registry` calls `sort()` internally so longer paths match
            // first. This test pins that contract: register `/api` first and
            // `/api/hello` second; a request to `/api/hello` must hit the
            // longer route, not the shorter one.
            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/api",
                method: Method::Get,
                body: "API",
            });
            registry.register(EchoEndpoint {
                path: "/api/hello",
                method: Method::Get,
                body: "API_HELLO",
            });
            let router = Router::from_registry(registry);

            let req = AxumRequest::builder()
                .method("GET")
                .uri("/api/hello")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(body_string(resp).await, "API_HELLO");
        }

        #[tokio::test]
        async fn new_constructor_yields_ready_to_serve_router() {
            // PRD-mandated verification for JOLT-RS-036: `Router::new(registry)`
            // produces a ready-to-serve tower::Service. Driving a registered
            // route end-to-end (200 + body match) proves both the dispatch
            // wiring and the longest-path-first sort that `from_registry`
            // shares with `new` are intact behind the canonical constructor.
            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/hello",
                method: Method::Get,
                body: "hi",
            });
            let router = Router::new(registry);

            let req = AxumRequest::builder()
                .method("GET")
                .uri("/hello")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(body_string(resp).await, "hi");
        }

        #[tokio::test]
        async fn new_constructor_sorts_registry_for_longest_path_match() {
            // Pins that `Router::new` delegates to `from_registry` (and thus
            // inherits the longest-path-first sort) rather than re-implementing
            // construction. Without the sort, `/api` would match before
            // `/api/hello` and the wrong handler would respond.
            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/api",
                method: Method::Get,
                body: "API",
            });
            registry.register(EchoEndpoint {
                path: "/api/hello",
                method: Method::Get,
                body: "API_HELLO",
            });
            let router = Router::new(registry);

            let req = AxumRequest::builder()
                .method("GET")
                .uri("/api/hello")
                .body(Body::empty())
                .unwrap();
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(body_string(resp).await, "API_HELLO");
        }

        #[tokio::test]
        async fn new_constructor_composes_with_tower_service_builder() {
            // PRD-036's "with optional tower Layer stack" phrase is satisfied
            // by Router being a tower::Service that callers wrap with
            // ServiceBuilder externally. This test pins that compositional
            // contract by routing a request through a ServiceBuilder-built
            // service whose final `.service(router)` is the registry-driven
            // Router. ServiceBuilder with no `.layer(...)` calls is the
            // identity case; if Router ever stops being a tower::Service or
            // breaks Service<AxumRequest, Response = Response, Error =
            // Infallible>, this test fails to compile rather than at runtime.
            use tower::ServiceBuilder;

            let mut registry = EndpointRegistry::new();
            registry.register(EchoEndpoint {
                path: "/hello",
                method: Method::Get,
                body: "hi",
            });
            let router = Router::new(registry);
            let svc = ServiceBuilder::new().service(router);

            let req = AxumRequest::builder()
                .method("GET")
                .uri("/hello")
                .body(Body::empty())
                .unwrap();
            let resp = svc.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(body_string(resp).await, "hi");
        }
    }
}

mod server {
    use crate::{CorsConfig, Endpoint, EndpointFuture, JoltServer, Method, Request};
    use axum::body::Body;

    struct StubEndpoint;

    impl Endpoint for StubEndpoint {
        fn path(&self) -> &str {
            "/stub"
        }

        fn method(&self) -> Method {
            Method::Get
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
    fn new_uses_default_port_8080() {
        // PRD-mandated verification for JOLT-RS-023: defaults include port=8080.
        let server = JoltServer::new();
        assert_eq!(server.port, 8080);
    }

    #[test]
    fn new_starts_with_empty_registry() {
        // Locks in the default-empty contract that JOLT-RS-026's `endpoint`
        // builder method extends — every increment must come from a register call.
        let server = JoltServer::new();
        assert!(server.registry.is_empty());
    }

    #[test]
    fn endpoint_builder_registers_into_registry() {
        // PRD-mandated verification for JOLT-RS-026: register an endpoint,
        // verify it appears in the registry.
        let server = JoltServer::new().endpoint(StubEndpoint);
        assert_eq!(server.registry.len(), 1);
    }

    #[test]
    fn endpoint_builder_chains_with_other_builder_methods() {
        // The whole point of `endpoint(self) -> Self` is fluent chaining
        // alongside `port`/`threads`/`cors`. If a future change accidentally
        // takes `&mut self`, this test stops compiling.
        let server = JoltServer::new()
            .port(3000)
            .endpoint(StubEndpoint)
            .threads(2)
            .endpoint(StubEndpoint);
        assert_eq!(server.port, 3000);
        assert_eq!(server.threads, 2);
        assert_eq!(server.registry.len(), 2);
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
        let server = JoltServer::new().cors(CorsConfig::default());
        assert!(server.cors_config.is_some());
    }

    #[test]
    fn cors_config_default_is_empty_and_restrictive() {
        // PRD-mandated verification for JOLT-RS-055: "Struct compiles with
        // defaults." `CorsConfig::default()` must yield an empty config — no
        // origins, no methods, no headers, max_age = 0, no exposed headers —
        // so a server that wires it in without further configuration grants
        // no CORS access. JOLT-RS-057 added `expose_headers`; default remains
        // restrictive.
        let cfg = CorsConfig::default();
        assert!(cfg.allow_origins.is_empty());
        assert!(cfg.allow_methods.is_empty());
        assert!(cfg.allow_headers.is_empty());
        assert_eq!(cfg.max_age, 0);
        assert!(cfg.expose_headers.is_empty());
    }

    #[test]
    fn cors_config_constructed_with_explicit_fields() {
        // Pins the public field surface 056..058 will read at request time.
        // If a future refactor renames a field or boxes a Vec, this test
        // breaks at the construction site rather than deep inside the layer.
        let cfg = CorsConfig {
            allow_origins: vec!["https://example.com".to_string()],
            allow_methods: vec![Method::Get, Method::Post],
            allow_headers: vec!["content-type".to_string()],
            max_age: 600,
            expose_headers: vec!["x-request-id".to_string()],
        };
        assert_eq!(cfg.allow_origins, vec!["https://example.com".to_string()]);
        assert_eq!(cfg.allow_methods, vec![Method::Get, Method::Post]);
        assert_eq!(cfg.allow_headers, vec!["content-type".to_string()]);
        assert_eq!(cfg.max_age, 600);
        assert_eq!(cfg.expose_headers, vec!["x-request-id".to_string()]);
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

mod cors {
    //! PRD-mandated verification for JOLT-RS-056: "Unit test: OPTIONS /api/test
    //! → 204 with CORS headers, finished flag set."
    //!
    //! The layer wraps an inner stub service that panics if invoked, so the
    //! short-circuit contract (OPTIONS never reaches inner) is structurally
    //! enforced — a regression that delegated `OPTIONS` to inner before
    //! returning would crash this test on the panic, not on a status mismatch.

    use std::convert::Infallible;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::Request as AxumRequest;
    use axum::http::header::{
        ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
        ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE,
        ORIGIN,
    };
    use axum::http::{Method as HttpMethod, StatusCode};
    use axum::response::Response;
    use tower::{Layer, ServiceExt};

    use crate::{CorsConfig, CorsLayer, Method, RequestExt};

    #[tokio::test]
    async fn options_request_returns_204_with_cors_headers_and_marks_finished() {
        // PRD-mandated verification for JOLT-RS-056. Builds a CorsLayer with a
        // populated CorsConfig (origin + methods + headers + max_age), wraps an
        // inner service that panics if invoked (structurally enforces the
        // short-circuit contract: any non-panic outcome has, by construction,
        // exercised the layer's OPTIONS-bypass path), sends
        // `OPTIONS /api/test` carrying a caller-supplied `Arc<RequestExt>`,
        // and verifies status, all four CORS headers, and the finished flag.
        let config = CorsConfig {
            allow_origins: vec!["https://example.com".to_string()],
            allow_methods: vec![Method::Get, Method::Post, Method::Options],
            allow_headers: vec!["content-type".to_string(), "authorization".to_string()],
            max_age: 600,
            // JOLT-RS-057's `expose_headers` field. Empty here so the OPTIONS
            // preflight assertions stay focused on the four headers the 056
            // verification mandates; non-OPTIONS expose-header injection is
            // covered by sibling tests in this module.
            expose_headers: vec![],
        };
        let layer = CorsLayer::new(config);
        // `service_fn` produces a service whose Future is a Send + 'static
        // async block — matches the bounds `CorsService::<S>: Service<AxumRequest>`
        // requires on S::Future.
        let inner = tower::service_fn(|_req: AxumRequest| async move {
            panic!("inner service must NOT be called for OPTIONS preflight");
            #[allow(unreachable_code)]
            Ok::<Response, Infallible>(Response::new(Body::empty()))
        });
        let svc = layer.layer(inner);

        // Caller-supplied RequestExt so the test can observe the finished flag
        // after the layer returns. The layer's preserve-or-inject contract
        // (mirroring Router's JOLT-RS-035 behavior) keeps THIS Arc alive in
        // extensions; the `mark_finished()` call inside the layer is observable
        // here because both sides hold clones of the same Arc.
        let ext = Arc::new(RequestExt::new());

        // The `Origin` header is required for the JOLT-RS-058 matching helper
        // to echo the configured origin back. Without it (or a wildcard
        // config), Allow-Origin is omitted — that case is covered by the
        // sibling no-match test rather than this verification path.
        let mut req = AxumRequest::builder()
            .method(HttpMethod::OPTIONS)
            .uri("/api/test")
            .header(ORIGIN, "https://example.com")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = svc.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let headers = resp.headers();
        assert_eq!(
            headers
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|v| v.to_str().ok()),
            Some("https://example.com"),
        );
        assert_eq!(
            headers
                .get(ACCESS_CONTROL_ALLOW_METHODS)
                .and_then(|v| v.to_str().ok()),
            Some("GET, POST, OPTIONS"),
        );
        assert_eq!(
            headers
                .get(ACCESS_CONTROL_ALLOW_HEADERS)
                .and_then(|v| v.to_str().ok()),
            Some("content-type, authorization"),
        );
        assert_eq!(
            headers
                .get(ACCESS_CONTROL_MAX_AGE)
                .and_then(|v| v.to_str().ok()),
            Some("600"),
        );

        assert!(
            ext.is_finished(),
            "CorsLayer must mark RequestExt finished on OPTIONS short-circuit"
        );
    }

    #[tokio::test]
    async fn non_options_response_gets_allow_origin_and_expose_headers_injected() {
        // PRD-mandated verification for JOLT-RS-057: "Unit test: GET /api/test
        // → response includes CORS origin header." Extends the verification to
        // also cover the parenthetical "(and optionally Access-Control-
        // Expose-Headers)" by configuring `expose_headers` and asserting the
        // injection. The inner service returns a 200 with a body; the layer
        // must NOT short-circuit, must NOT mark finished (that's the OPTIONS
        // contract, not the non-OPTIONS one), and must mutate the response's
        // headers in place to add the two CORS response headers.
        let config = CorsConfig {
            allow_origins: vec!["https://example.com".to_string()],
            allow_methods: vec![Method::Get, Method::Post],
            allow_headers: vec!["content-type".to_string()],
            max_age: 600,
            expose_headers: vec!["x-request-id".to_string(), "x-trace-id".to_string()],
        };
        let layer = CorsLayer::new(config);

        // Inner service returns a 200 with a body; presence of the body bytes
        // in the final response confirms the layer did not consume or replace
        // the inner's output, only added headers.
        let inner = tower::service_fn(|_req: AxumRequest| async move {
            let mut resp = Response::new(Body::from("hello"));
            *resp.status_mut() = StatusCode::OK;
            Ok::<Response, Infallible>(resp)
        });
        let svc = layer.layer(inner);

        let ext = Arc::new(RequestExt::new());
        // JOLT-RS-058 matching: include `Origin` so the configured origin is
        // echoed back. Without it, Allow-Origin would be omitted.
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/test")
            .header(ORIGIN, "https://example.com")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = svc.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let headers = resp.headers();
        assert_eq!(
            headers
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|v| v.to_str().ok()),
            Some("https://example.com"),
        );
        assert_eq!(
            headers
                .get(ACCESS_CONTROL_EXPOSE_HEADERS)
                .and_then(|v| v.to_str().ok()),
            Some("x-request-id, x-trace-id"),
        );
        // OPTIONS short-circuit semantics must not leak into the non-OPTIONS
        // path: the layer should pass through, not mark finished.
        assert!(
            !ext.is_finished(),
            "CorsLayer must NOT mark RequestExt finished on non-OPTIONS requests"
        );
    }

    #[tokio::test]
    async fn non_options_response_skips_headers_when_config_is_empty() {
        // Empty CorsConfig (the JOLT-RS-055 restrictive default) must produce
        // a non-OPTIONS response with NO CORS headers added — matches the
        // empty-default contract and the OPTIONS branch's behavior. Without
        // this guarantee a server that wires CorsLayer with the default
        // config would still leak CORS headers, defeating the spec's
        // "permissive-only-on-explicit-config" stance.
        let layer = CorsLayer::new(CorsConfig::default());

        let inner = tower::service_fn(|_req: AxumRequest| async move {
            Ok::<Response, Infallible>(Response::new(Body::from("hello")))
        });
        let svc = layer.layer(inner);

        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/test")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let headers = resp.headers();
        assert!(headers.get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
        assert!(
            headers
                .get(axum::http::header::ACCESS_CONTROL_EXPOSE_HEADERS)
                .is_none()
        );
    }

    #[tokio::test]
    async fn non_options_response_preserves_inner_set_cors_headers() {
        // If the inner service has already set `Access-Control-Allow-Origin`
        // (for instance, a per-route handler that wants origin-specific
        // policy), the layer must NOT overwrite it. The layer is additive —
        // it fills in the field when nothing else has, but it doesn't claim
        // exclusive ownership of the header. Same contract for
        // `Access-Control-Expose-Headers`.
        let config = CorsConfig {
            allow_origins: vec!["https://from-config.example.com".to_string()],
            allow_methods: vec![],
            allow_headers: vec![],
            max_age: 0,
            expose_headers: vec!["x-from-config".to_string()],
        };
        let layer = CorsLayer::new(config);

        let inner = tower::service_fn(|_req: AxumRequest| async move {
            let mut resp = Response::new(Body::empty());
            resp.headers_mut().insert(
                ACCESS_CONTROL_ALLOW_ORIGIN,
                axum::http::HeaderValue::from_static("https://from-handler.example.com"),
            );
            resp.headers_mut().insert(
                axum::http::header::ACCESS_CONTROL_EXPOSE_HEADERS,
                axum::http::HeaderValue::from_static("x-from-handler"),
            );
            Ok::<Response, Infallible>(resp)
        });
        let svc = layer.layer(inner);

        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/test")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();

        let headers = resp.headers();
        assert_eq!(
            headers
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|v| v.to_str().ok()),
            Some("https://from-handler.example.com"),
            "layer must not overwrite inner-set Access-Control-Allow-Origin",
        );
        assert_eq!(
            headers
                .get(ACCESS_CONTROL_EXPOSE_HEADERS)
                .and_then(|v| v.to_str().ok()),
            Some("x-from-handler"),
            "layer must not overwrite inner-set Access-Control-Expose-Headers",
        );
    }

    // -- JOLT-RS-058: shared origin-matching helper coverage. ---------------
    //
    // The four tests below pin the contract for `select_allowed_origin`,
    // shared by both the OPTIONS preflight and the non-OPTIONS injection:
    // wildcard always emits "*", a specific configured origin echoes the
    // request's `Origin` header back when it matches, a non-matching origin
    // suppresses the header entirely, and the preflight emits the configured
    // method list verbatim.

    #[tokio::test]
    async fn wildcard_origin_emits_star_regardless_of_request_origin() {
        // `allow_origins = vec!["*"]` is the wildcard shape. The matching
        // helper short-circuits before the Origin-header lookup and emits
        // a literal `*` — works whether or not the request advertises an
        // Origin (browsers always send one, but tests don't have to).
        let config = CorsConfig {
            allow_origins: vec!["*".to_string()],
            allow_methods: vec![],
            allow_headers: vec![],
            max_age: 0,
            expose_headers: vec![],
        };
        let layer = CorsLayer::new(config);

        let inner = tower::service_fn(|_req: AxumRequest| async move {
            Ok::<Response, Infallible>(Response::new(Body::from("ok")))
        });
        let svc = layer.layer(inner);

        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/test")
            .header(ORIGIN, "https://anywhere.example.com")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();

        assert_eq!(
            resp.headers()
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|v| v.to_str().ok()),
            Some("*"),
            "wildcard config must emit `*` for Access-Control-Allow-Origin",
        );
    }

    #[tokio::test]
    async fn specific_origin_match_echoes_request_origin() {
        // Multi-origin allow-list with the request's Origin landing on the
        // SECOND entry — proves the matching helper does an actual lookup,
        // not the prior first-entry simplification. The echoed value must be
        // the request's Origin, not the first configured entry.
        let config = CorsConfig {
            allow_origins: vec![
                "https://a.example.com".to_string(),
                "https://b.example.com".to_string(),
            ],
            allow_methods: vec![],
            allow_headers: vec![],
            max_age: 0,
            expose_headers: vec![],
        };
        let layer = CorsLayer::new(config);

        let inner = tower::service_fn(|_req: AxumRequest| async move {
            Ok::<Response, Infallible>(Response::new(Body::from("ok")))
        });
        let svc = layer.layer(inner);

        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/test")
            .header(ORIGIN, "https://b.example.com")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();

        assert_eq!(
            resp.headers()
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|v| v.to_str().ok()),
            Some("https://b.example.com"),
            "matched origin must be echoed back, not the first config entry",
        );
    }

    #[tokio::test]
    async fn no_origin_match_omits_allow_origin_header() {
        // The request advertises an Origin that is NOT in the allow-list
        // (and the config does not contain `*`). The helper must return
        // None so neither branch emits Access-Control-Allow-Origin —
        // refusing a CORS grant is the spec-correct response, NOT echoing
        // the unauthorized origin or falling back to a configured default.
        let config = CorsConfig {
            allow_origins: vec!["https://allowed.example.com".to_string()],
            allow_methods: vec![],
            allow_headers: vec![],
            max_age: 0,
            expose_headers: vec![],
        };
        let layer = CorsLayer::new(config);

        let inner = tower::service_fn(|_req: AxumRequest| async move {
            Ok::<Response, Infallible>(Response::new(Body::from("ok")))
        });
        let svc = layer.layer(inner);

        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/test")
            .header(ORIGIN, "https://attacker.example.com")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();

        assert!(
            resp.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
            "non-matching origin must NOT receive Access-Control-Allow-Origin",
        );
    }

    #[tokio::test]
    async fn preflight_emits_correct_allow_methods_header() {
        // OPTIONS preflight with a configured method list must produce
        // `Access-Control-Allow-Methods` joined with ", " in the order
        // the methods appear in `allow_methods`. The wildcard origin keeps
        // matching out of the way so this test focuses on the methods header.
        let config = CorsConfig {
            allow_origins: vec!["*".to_string()],
            allow_methods: vec![Method::Get, Method::Post, Method::Delete],
            allow_headers: vec![],
            max_age: 0,
            expose_headers: vec![],
        };
        let layer = CorsLayer::new(config);

        let inner = tower::service_fn(|_req: AxumRequest| async move {
            panic!("inner service must NOT be called for OPTIONS preflight");
            #[allow(unreachable_code)]
            Ok::<Response, Infallible>(Response::new(Body::empty()))
        });
        let svc = layer.layer(inner);

        let req = AxumRequest::builder()
            .method(HttpMethod::OPTIONS)
            .uri("/api/test")
            .header(ORIGIN, "https://example.com")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            resp.headers()
                .get(ACCESS_CONTROL_ALLOW_METHODS)
                .and_then(|v| v.to_str().ok()),
            Some("GET, POST, DELETE"),
            "preflight Access-Control-Allow-Methods must list configured methods in order",
        );
    }
}

mod parse_body {
    //! PRD-mandated verification for JOLT-RS-059: "Unit test: valid JSON body
    //! → T populated in middleware struct."
    //!
    //! Two structural checks are stacked into this slice:
    //! 1. A valid JSON body that deserializes into `T` lands in request
    //!    extensions on the inner service's side — so an AutoMiddleware
    //!    consumer (or any handler) can pull it back out with the standard
    //!    `req.extensions().get::<T>()` API. This is the "T populated"
    //!    contract from the PRD.
    //! 2. The layer restores the buffered body bytes onto the request before
    //!    delegating, so downstream services (notably `build_jolt_request`'s
    //!    re-read in the registry path) continue to see the body. Without
    //!    this guarantee, wiring ParseBodyLayer ahead of Router would silently
    //!    blank-out the body for every handler that doesn't go through the
    //!    extensions channel — a regression that 060+ would have a hard time
    //!    catching.
    //!
    //! Parse-failure rejection (400 + mark_finished) lands in JOLT-RS-060's
    //! sibling test below: `invalid_json_body_short_circuits_with_400_and_marks_finished`.

    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::extract::Request as AxumRequest;
    use axum::http::{Method as HttpMethod, StatusCode};
    use axum::response::Response;
    use serde::Deserialize;
    use tower::{Layer, ServiceExt};

    use crate::request_ext::RequestExt;
    use crate::ParseBodyLayer;

    #[derive(Debug, Deserialize, Clone, PartialEq)]
    struct TestBody {
        name: String,
        age: u32,
    }

    #[tokio::test]
    async fn valid_json_body_is_parsed_and_inserted_into_extensions() {
        // Inner service captures whatever it sees in extensions plus a copy
        // of the (already-restored) body bytes — two observations from one
        // call, so the test asserts BOTH 059 contracts (extension insertion
        // AND body restoration) without running the request twice.
        let captured_body: Arc<Mutex<Option<TestBody>>> = Arc::new(Mutex::new(None));
        let captured_bytes: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let body_clone = Arc::clone(&captured_body);
        let bytes_clone = Arc::clone(&captured_bytes);
        let inner = tower::service_fn(move |req: AxumRequest| {
            let body_clone = Arc::clone(&body_clone);
            let bytes_clone = Arc::clone(&bytes_clone);
            async move {
                if let Some(body) = req.extensions().get::<TestBody>() {
                    *body_clone.lock().unwrap() = Some(body.clone());
                }
                // Drain the restored body so the test can verify the layer
                // didn't consume the bytes off the request.
                let (_, body) = req.into_parts();
                let bytes = axum::body::to_bytes(body, u32::MAX as usize)
                    .await
                    .unwrap_or_default();
                *bytes_clone.lock().unwrap() = bytes.to_vec();
                Ok::<Response, Infallible>(Response::new(Body::empty()))
            }
        });
        let layer = ParseBodyLayer::<TestBody>::new();
        let svc = layer.layer(inner);

        let payload = br#"{"name":"jolt","age":7}"#;
        let req = AxumRequest::builder()
            .method(HttpMethod::POST)
            .uri("/api/test")
            .header("content-type", "application/json")
            .body(Body::from(&payload[..]))
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let parsed = captured_body.lock().unwrap().clone();
        assert_eq!(
            parsed,
            Some(TestBody {
                name: "jolt".to_string(),
                age: 7
            }),
            "valid JSON body must deserialize into T and land in extensions",
        );

        let restored = captured_bytes.lock().unwrap().clone();
        assert_eq!(
            restored, payload,
            "layer must restore the buffered body bytes onto the request before delegating",
        );
    }

    #[tokio::test]
    async fn invalid_json_body_short_circuits_with_400_and_marks_finished() {
        // JOLT-RS-060: malformed JSON must produce a 400 with an "Invalid
        // JSON: ..." text/plain body, AND the layer must flip the request's
        // RequestExt finished latch. The inner service is structurally
        // forbidden from running by handing it a panicking service_fn — any
        // delegation past the failure branch causes an unrelated test crash
        // rather than a silent contract regression.
        let inner = tower::service_fn(|_: AxumRequest| async move {
            panic!("ParseBodyService must short-circuit on parse failure and never call inner");
            #[allow(unreachable_code)]
            Ok::<Response, Infallible>(Response::new(Body::empty()))
        });
        let layer = ParseBodyLayer::<TestBody>::new();
        let svc = layer.layer(inner);

        // Pre-inject an Arc<RequestExt> so the test can observe the
        // finished-flag flip after the layer runs (the alternative — letting
        // the layer inject a fresh ext — would leave the test with no
        // handle on the same Arc).
        let request_ext = Arc::new(RequestExt::new());
        let mut req = AxumRequest::builder()
            .method(HttpMethod::POST)
            .uri("/api/test")
            .header("content-type", "application/json")
            .body(Body::from(&b"{not valid json"[..]))
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "invalid JSON must surface 400 Bad Request",
        );
        let content_type = resp
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .expect("400 response must carry a Content-Type")
            .to_str()
            .expect("Content-Type must be ASCII");
        assert!(
            content_type.starts_with("text/plain"),
            "400 body is text/plain; got {content_type}",
        );
        let body_bytes = axum::body::to_bytes(resp.into_body(), u32::MAX as usize)
            .await
            .unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("400 body is UTF-8");
        assert!(
            body_text.starts_with("Invalid JSON: "),
            "400 body must be prefixed with 'Invalid JSON: '; got {body_text:?}",
        );
        assert!(
            body_text.len() > "Invalid JSON: ".len(),
            "400 body must include the underlying serde error detail; got {body_text:?}",
        );

        assert!(
            request_ext.is_finished(),
            "ParseBodyService must flip RequestExt::mark_finished on parse failure",
        );
        assert!(
            request_ext.take_response().is_none(),
            "the 400 is returned directly from call(), not stashed in RequestExt",
        );
    }
}

mod parse_body_string {
    //! PRD-mandated verification for JOLT-RS-061: "Unit test: POST with
    //! text/plain body → extracted as String."
    //!
    //! Two structural checks stacked into the primary test:
    //! 1. A UTF-8 `text/plain` body lands in request extensions as `String`
    //!    on the inner service's side — the "extracted as String" contract.
    //! 2. The layer restores the buffered body bytes onto the request before
    //!    delegating, matching the contract `ParseBodyLayer` pinned in 059.
    //!
    //! The sibling `non_utf8_body_short_circuits_with_400_and_marks_finished`
    //! test pins the failure-rejection contract that mirrors 060's JSON-failure
    //! behavior: invalid bytes → 400 + mark_finished + direct return.

    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::extract::Request as AxumRequest;
    use axum::http::{Method as HttpMethod, StatusCode};
    use axum::response::Response;
    use tower::{Layer, ServiceExt};

    use crate::request_ext::RequestExt;
    use crate::ParseBodyStringLayer;

    #[tokio::test]
    async fn text_plain_body_is_decoded_and_inserted_into_extensions() {
        let captured_body: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let captured_bytes: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let body_clone = Arc::clone(&captured_body);
        let bytes_clone = Arc::clone(&captured_bytes);
        let inner = tower::service_fn(move |req: AxumRequest| {
            let body_clone = Arc::clone(&body_clone);
            let bytes_clone = Arc::clone(&bytes_clone);
            async move {
                if let Some(body) = req.extensions().get::<String>() {
                    *body_clone.lock().unwrap() = Some(body.clone());
                }
                let (_, body) = req.into_parts();
                let bytes = axum::body::to_bytes(body, u32::MAX as usize)
                    .await
                    .unwrap_or_default();
                *bytes_clone.lock().unwrap() = bytes.to_vec();
                Ok::<Response, Infallible>(Response::new(Body::empty()))
            }
        });
        let layer = ParseBodyStringLayer::new();
        let svc = layer.layer(inner);

        let payload = b"hello, jolt";
        let req = AxumRequest::builder()
            .method(HttpMethod::POST)
            .uri("/api/text")
            .header("content-type", "text/plain")
            .body(Body::from(&payload[..]))
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let extracted = captured_body.lock().unwrap().clone();
        assert_eq!(
            extracted,
            Some("hello, jolt".to_string()),
            "text/plain body must be decoded as UTF-8 and land in extensions as String",
        );

        let restored = captured_bytes.lock().unwrap().clone();
        assert_eq!(
            restored, payload,
            "layer must restore the buffered body bytes onto the request before delegating",
        );
    }

    #[tokio::test]
    async fn non_utf8_body_short_circuits_with_400_and_marks_finished() {
        // Mirror of JOLT-RS-060's invalid-JSON test, adapted for the UTF-8
        // failure surface. A panicking inner service structurally forbids
        // delegation past the failure branch.
        let inner = tower::service_fn(|_: AxumRequest| async move {
            panic!("ParseBodyStringService must short-circuit on UTF-8 failure and never call inner");
            #[allow(unreachable_code)]
            Ok::<Response, Infallible>(Response::new(Body::empty()))
        });
        let layer = ParseBodyStringLayer::new();
        let svc = layer.layer(inner);

        let request_ext = Arc::new(RequestExt::new());
        // 0xff is never a valid UTF-8 leading byte.
        let invalid_utf8: &[u8] = &[0xff, 0xfe, 0xfd];
        let mut req = AxumRequest::builder()
            .method(HttpMethod::POST)
            .uri("/api/text")
            .header("content-type", "text/plain")
            .body(Body::from(invalid_utf8.to_vec()))
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "invalid UTF-8 must surface 400 Bad Request",
        );
        let content_type = resp
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .expect("400 response must carry a Content-Type")
            .to_str()
            .expect("Content-Type must be ASCII");
        assert!(
            content_type.starts_with("text/plain"),
            "400 body is text/plain; got {content_type}",
        );
        let body_bytes = axum::body::to_bytes(resp.into_body(), u32::MAX as usize)
            .await
            .unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("400 body is UTF-8");
        assert!(
            body_text.starts_with("Invalid UTF-8: "),
            "400 body must be prefixed with 'Invalid UTF-8: '; got {body_text:?}",
        );
        assert!(
            body_text.len() > "Invalid UTF-8: ".len(),
            "400 body must include the underlying utf-8 error detail; got {body_text:?}",
        );

        assert!(
            request_ext.is_finished(),
            "ParseBodyStringService must flip RequestExt::mark_finished on UTF-8 failure",
        );
        assert!(
            request_ext.take_response().is_none(),
            "the 400 is returned directly from call(), not stashed in RequestExt",
        );
    }

    #[tokio::test]
    async fn empty_body_decodes_to_empty_string() {
        // Empty bytes are valid UTF-8 (the empty string). Pins the contract
        // that an empty body is NOT treated as a UTF-8 failure — empty-body
        // rejection is the user's responsibility, not this layer's.
        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let captured_clone = Arc::clone(&captured);
        let inner = tower::service_fn(move |req: AxumRequest| {
            let captured = Arc::clone(&captured_clone);
            async move {
                if let Some(body) = req.extensions().get::<String>() {
                    *captured.lock().unwrap() = Some(body.clone());
                }
                Ok::<Response, Infallible>(Response::new(Body::empty()))
            }
        });
        let layer = ParseBodyStringLayer::new();
        let svc = layer.layer(inner);

        let req = AxumRequest::builder()
            .method(HttpMethod::POST)
            .uri("/api/text")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            captured.lock().unwrap().clone(),
            Some(String::new()),
            "empty body must decode to an empty String (not 400)",
        );
    }
}

mod parse_query {
    //! PRD-mandated verification for JOLT-RS-063: "Unit test: ?id=42 → id
    //! parsed as \"42\"."
    //!
    //! The first test pins the PRD's `?id=42` example: a single-pair query
    //! string lands in extensions as a [`QueryParams`] map with `id → "42"`.
    //! Two sibling tests pin the always-insert contract from module docs
    //! decision 3:
    //! - A request with NO query string still gets an empty [`QueryParams`]
    //!   inserted (downstream consumers can `.get::<QueryParams>()` without a
    //!   `?query=` upstream).
    //! - A request with multiple `&`-joined pairs has every pair represented
    //!   in the map.

    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::extract::Request as AxumRequest;
    use axum::http::{Method as HttpMethod, StatusCode};
    use axum::response::Response;
    use tower::{Layer, ServiceExt};

    use crate::{ParseQueryLayer, QueryParams};

    /// Build an inner `service_fn` that captures the [`QueryParams`] extension
    /// (if any) into the supplied `Mutex` and returns 200. Hoisted into a
    /// helper so the three tests below assert on the same observation surface
    /// without duplicating the closure shape.
    fn capture_inner(
        captured: Arc<Mutex<Option<QueryParams>>>,
    ) -> impl tower::Service<
        AxumRequest,
        Response = Response,
        Error = Infallible,
        Future = impl std::future::Future<Output = Result<Response, Infallible>> + Send,
    > + Clone {
        tower::service_fn(move |req: AxumRequest| {
            let captured = Arc::clone(&captured);
            async move {
                if let Some(params) = req.extensions().get::<QueryParams>() {
                    *captured.lock().unwrap() = Some(params.clone());
                }
                Ok::<Response, Infallible>(Response::new(Body::empty()))
            }
        })
    }

    #[tokio::test]
    async fn single_pair_query_is_parsed_and_inserted_into_extensions() {
        // The PRD-mandated case: `?id=42` → `id` parsed as `"42"`. Asserts on
        // both the extension's presence (always-insert contract) AND the
        // single-pair value mapping.
        let captured: Arc<Mutex<Option<QueryParams>>> = Arc::new(Mutex::new(None));
        let svc = ParseQueryLayer::new().layer(capture_inner(Arc::clone(&captured)));

        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/test?id=42")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let params = captured
            .lock()
            .unwrap()
            .clone()
            .expect("ParseQueryService must always insert a QueryParams extension");
        assert_eq!(
            params.get("id").map(String::as_str),
            Some("42"),
            "?id=42 must parse `id` as the string \"42\" (PRD verification)",
        );
        assert_eq!(params.len(), 1, "single-pair query yields a single entry");
    }

    #[tokio::test]
    async fn missing_query_string_inserts_empty_params() {
        // Pins module docs decision 3: the extension is ALWAYS present after
        // the layer runs, even when the URI carries no `?…`. Without this
        // guarantee, every downstream consumer would have to handle two shapes
        // (present-empty vs. absent) for the same logical state.
        let captured: Arc<Mutex<Option<QueryParams>>> = Arc::new(Mutex::new(None));
        let svc = ParseQueryLayer::new().layer(capture_inner(Arc::clone(&captured)));

        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/test")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let params = captured
            .lock()
            .unwrap()
            .clone()
            .expect("missing query must still produce a QueryParams extension");
        assert!(
            params.is_empty(),
            "no `?` in URI → empty QueryParams; got {params:?}",
        );
    }

    #[tokio::test]
    async fn multiple_pairs_are_each_represented_in_the_map() {
        let captured: Arc<Mutex<Option<QueryParams>>> = Arc::new(Mutex::new(None));
        let svc = ParseQueryLayer::new().layer(capture_inner(Arc::clone(&captured)));

        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/test?id=42&name=jolt&active=true")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let params = captured.lock().unwrap().clone().expect("extension present");
        assert_eq!(params.get("id").map(String::as_str), Some("42"));
        assert_eq!(params.get("name").map(String::as_str), Some("jolt"));
        assert_eq!(params.get("active").map(String::as_str), Some("true"));
        assert_eq!(params.len(), 3);
    }
}

mod parse_query_typed {
    //! PRD-mandated verification for JOLT-RS-064: "Unit test: ?count=5 →
    //! `i32::from_str("5") == 5`. ?count=abc → 400."
    //!
    //! The 064 surface is a field-level extractor (`extract::<T>(params, key)`)
    //! that consumes the [`QueryParams`] map produced by JOLT-RS-063. Tests
    //! pin:
    //! - The PRD success path (`?count=5` → `Ok(5_i32)`).
    //! - The PRD failure path (`?count=abc` → `Err(Invalid …)` whose
    //!   `bad_request_for_query_error` rendering is a 400).
    //! - Float `FromStr` round-trip (the second half of the 064 mandate).
    //! - Missing key surfaces as the distinct `Missing` variant (load-bearing
    //!   for whichever 065+/codegen path wants to map it differently from
    //!   `Invalid`).
    //! - The 400 response body carries the typed-error `Display` payload so
    //!   downstream callers can read the failing key + value off the response.
    //!
    //! 065's bool/enum extractors are explicitly NOT covered here — they need
    //! their own helpers (bool's `FromStr` rejects `"1"`/`"0"`; enums need
    //! `TryFrom<&str>`). Those will land in a sibling test module when the
    //! PRD item ships.
    use std::collections::HashMap;

    use axum::body::to_bytes;
    use axum::http::StatusCode;

    use crate::{
        bad_request_for_query_error, extract_query, QueryExtractError, QueryParams,
    };

    fn params_from(pairs: &[(&str, &str)]) -> QueryParams {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        QueryParams::from(map)
    }

    #[test]
    fn int_value_parses_via_from_str() {
        // PRD: ?count=5 → i32::from_str("5") == 5.
        let params = params_from(&[("count", "5")]);
        let parsed: i32 =
            extract_query(&params, "count").expect("?count=5 must parse as i32");
        assert_eq!(parsed, 5);
    }

    #[tokio::test]
    async fn non_numeric_int_value_yields_invalid_error_and_400_response() {
        // PRD: ?count=abc → 400. The extractor returns `Invalid` carrying the
        // failing key, the rejected value, and the underlying ParseIntError's
        // message. `bad_request_for_query_error` renders that into a
        // `text/plain` 400 whose body includes the same detail so an HTTP
        // caller can see the failing field without having to introspect the
        // response code.
        let params = params_from(&[("count", "abc")]);
        let err = extract_query::<i32>(&params, "count")
            .expect_err("?count=abc must NOT parse as i32");

        match &err {
            QueryExtractError::Invalid {
                key,
                value,
                message,
            } => {
                assert_eq!(key, "count", "key field must echo the requested key");
                assert_eq!(value, "abc", "value field must echo the rejected raw value");
                assert!(
                    !message.is_empty(),
                    "message must carry the underlying parser's detail (got empty)",
                );
            }
            other => panic!("expected QueryExtractError::Invalid, got {other:?}"),
        }

        let resp = bad_request_for_query_error(&err);
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "PRD: ?count=abc → 400",
        );
        let content_type = resp
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .expect("400 response carries Content-Type")
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            content_type.starts_with("text/plain"),
            "400 body content-type should be text/plain, got {content_type}",
        );

        let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body_text = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert!(
            body_text.starts_with("Invalid query parameter 'count'='abc': "),
            "400 body must echo key+value+detail, got {body_text}",
        );
        assert!(
            body_text.len() > "Invalid query parameter 'count'='abc': ".len(),
            "400 body must include the underlying parser's detail (got bare prefix)",
        );
    }

    #[test]
    fn float_value_parses_via_from_str() {
        // The second half of the 064 mandate: floats use `.parse()` the same
        // way ints do. A single test pins both `f32` and `f64` so the generic
        // shape doesn't quietly regress to int-only.
        let params = params_from(&[("ratio", "1.5"), ("speed", "2.5")]);
        let ratio: f64 =
            extract_query(&params, "ratio").expect("?ratio=1.5 must parse as f64");
        assert!((ratio - 1.5).abs() < 1e-9, "f64 round-trip");

        let speed: f32 =
            extract_query(&params, "speed").expect("?speed=2.5 must parse as f32");
        assert!((speed - 2.5_f32).abs() < 1e-6, "f32 round-trip");
    }

    #[test]
    fn missing_key_yields_missing_variant_distinct_from_invalid() {
        // The Missing/Invalid split is load-bearing for 065+/codegen paths
        // that may want to map them differently (e.g., Missing → required-
        // field error message, Invalid → type-mismatch message). Asserting on
        // the variant here pins the contract; collapsing to a single variant
        // would silently break that future routing.
        let params = params_from(&[("other", "1")]);
        let err = extract_query::<i32>(&params, "count")
            .expect_err("absent key must NOT yield a parsed value");
        match err {
            QueryExtractError::Missing { key } => {
                assert_eq!(key, "count", "key field must echo the requested key");
            }
            other => panic!("expected QueryExtractError::Missing, got {other:?}"),
        }
    }
}
