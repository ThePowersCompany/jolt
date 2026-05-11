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

    // --- duplicate path/method detection (JOLT-RS-032) ---
    //
    // The registry is a Vec, not a set, so identical (path, method) pairs are
    // stored independently. The build_router path (axum) overwrites duplicate
    // routes (last one wins). The registry-driven dispatch path calls the first
    // matching endpoint due to sequential iteration. Both behaviors are
    // documented here so a future dedup or error-on-duplicate change must
    // update these tests intentionally.

    #[test]
    fn duplicate_path_and_method_both_stored_in_registry() {
        let mut registry = EndpointRegistry::new();
        registry.register(Stub {
            path: "/dup",
            method: Method::Get,
        });
        registry.register(Stub {
            path: "/dup",
            method: Method::Get,
        });
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn duplicate_path_different_methods_both_stored_in_registry() {
        let mut registry = EndpointRegistry::new();
        registry.register(Stub {
            path: "/shared",
            method: Method::Get,
        });
        registry.register(Stub {
            path: "/shared",
            method: Method::Post,
        });
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn duplicate_path_sort_preserves_registration_order_for_equal_lengths() {
        let mut registry = EndpointRegistry::new();
        registry.register(Stub {
            path: "/dup",
            method: Method::Get,
        });
        registry.register(Stub {
            path: "/dup",
            method: Method::Post,
        });
        registry.sort();
        let methods: Vec<Method> = registry.iter().map(Endpoint::method).collect();
        assert_eq!(methods, vec![Method::Get, Method::Post]);
    }

    // Wildcard/path-param extraction is not applicable to the current
    // Endpoint design (JOLT-RS-028), which uses exact string matching via
    // Endpoint::path() -> &str. Dynamic path segments (e.g. /api/:id) would
    // require a different trait or a pattern-matching layer — neither of
    // which is in scope for phase06. A future phase that adds path-param
    // extraction must update this comment and add corresponding tests.

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
        async fn finished_flag_short_circuits_handler_when_set_by_outer_tower_layer() {
            // PRD-mandated verification for JOLT-RS-079: a real `tower::Layer`
            // wrapping `Router::new(registry)` that flips `mark_finished()` +
            // stashes a response on the shared `Arc<RequestExt>` BEFORE
            // delegating to inner causes Router to take the stash and return
            // it without invoking the matched endpoint's handler. The handler
            // tracks invocation via an `AtomicBool` so the assertion
            // distinguishes "handler skipped" from "handler ran but returned
            // the same status the layer would have."
            //
            // Distinct from the sibling
            // `finished_flag_with_stashed_response_short_circuits_handler`,
            // which pre-inserts a marked-finished ext into the request
            // directly: this test routes through actual tower-layer
            // composition (`ServiceBuilder::layer(...).service(router)`) to
            // pin the contract end-to-end through Router's
            // preserve-existing-ext seam — a regression that broke the
            // tower-layer propagation of the latch (e.g., re-introducing the
            // pre-035 overwrite) would silently pass the direct-insertion
            // test but fail this one.
            use std::future::Future;
            use std::pin::Pin;
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::task::{Context, Poll};

            use tower::{Layer, Service, ServiceBuilder};

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

            #[derive(Clone)]
            struct FinishingLayer;

            impl<S> Layer<S> for FinishingLayer {
                type Service = FinishingService<S>;
                fn layer(&self, inner: S) -> Self::Service {
                    FinishingService { inner }
                }
            }

            #[derive(Clone)]
            struct FinishingService<S> {
                inner: S,
            }

            impl<S> Service<AxumRequest> for FinishingService<S>
            where
                S: Service<AxumRequest, Response = axum::response::Response>
                    + Clone
                    + Send
                    + 'static,
                S::Future: Send + 'static,
            {
                type Response = S::Response;
                type Error = S::Error;
                type Future =
                    Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

                fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
                    self.inner.poll_ready(cx)
                }

                fn call(&mut self, mut req: AxumRequest) -> Self::Future {
                    // Canonical "middleware rejected the request" pattern:
                    // grab (or inject) the Arc<RequestExt>, stash a 401
                    // response, flip the latch — then delegate to inner.
                    // Router's `call` will preserve this same Arc (per the
                    // JOLT-RS-035 preserve-existing-ext contract) and observe
                    // the finished latch on the SAME instance.
                    let ext: Arc<RequestExt> = match req.extensions().get::<Arc<RequestExt>>() {
                        Some(existing) => Arc::clone(existing),
                        None => {
                            let fresh = Arc::new(RequestExt::new());
                            req.extensions_mut().insert(Arc::clone(&fresh));
                            fresh
                        }
                    };
                    ext.set_response(
                        axum::response::Response::builder()
                            .status(StatusCode::UNAUTHORIZED)
                            .body(Body::from("layer rejected"))
                            .unwrap(),
                    );
                    ext.mark_finished();

                    let mut inner = self.inner.clone();
                    Box::pin(async move { inner.call(req).await })
                }
            }

            let invoked = Arc::new(AtomicBool::new(false));
            let mut registry = EndpointRegistry::new();
            registry.register(TrackingEndpoint {
                invoked: Arc::clone(&invoked),
            });
            let router = Router::new(registry);
            let svc = ServiceBuilder::new().layer(FinishingLayer).service(router);

            let req = AxumRequest::builder()
                .method("GET")
                .uri("/protected")
                .body(Body::empty())
                .unwrap();
            let resp = svc.oneshot(req).await.unwrap();

            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(body_string(resp).await, "layer rejected");
            assert!(
                !invoked.load(Ordering::Relaxed),
                "handler must not be invoked when an outer tower layer marks the request finished"
            );
        }

        #[tokio::test]
        async fn full_middleware_chain_propagates_auth_rejection_back_through_outer_layer() {
            // PRD-mandated verification for JOLT-RS-080: "Integration test:
            // full middleware chain, auth rejects → 401 returned, handler body
            // never executed."
            //
            // Stack (outer → inner): `CorsLayer → AuthBearerLayer →
            // Router(registry)`. Request is GET /protected with an Origin
            // header but NO Authorization header. Production flow:
            //   1. CorsLayer.call: GET (not OPTIONS), no upstream-finished →
            //      delegates to inner via the post-response injection path.
            //   2. AuthBearerLayer.call: no Authorization header → returns
            //      `401 Unauthorized` directly (with `WWW-Authenticate: Bearer`
            //      + "Missing Authorization header" body), flips
            //      `mark_finished` on the shared `Arc<RequestExt>`, and does
            //      NOT delegate to inner.
            //   3. Router is never invoked. The 401 propagates UP through
            //      CorsLayer's async block, where
            //      `inject_response_cors_headers` adds `Access-Control-Allow-
            //      Origin` on the way back per JOLT-RS-057's non-OPTIONS
            //      contract.
            //   4. The handler's `invoked` AtomicBool stays false across the
            //      whole chain.
            //
            // Distinct from the sibling
            // `finished_flag_short_circuits_handler_when_set_by_outer_tower_layer`
            // (JOLT-RS-079): that test uses a FAKE FinishingLayer that flips
            // the latch and DELEGATES to inner, exercising Router's
            // stash/take-on-finished path. THIS test uses the REAL
            // AuthBearerLayer, whose rejection branch returns the 401 DIRECTLY
            // without delegating to inner — so Router is never invoked at all.
            // Together the two tests pin both ladders of the early-termination
            // contract:
            //   - 079: Router-side stash-take on finished latch (layer flips
            //     latch then delegates → Router takes stash).
            //   - 080: layer-side direct return without delegation (layer
            //     returns its own response, propagates up through outer
            //     layers, never touches Router).
            //
            // The OUTER CorsLayer is load-bearing for the propagation
            // assertion: a regression that swallowed the inner response (or
            // replaced it with an unrelated default) would surface as a
            // missing WWW-Authenticate header, a missing 401 body, or — for
            // CorsLayer's own contract — a 401 without `Access-Control-Allow-
            // Origin`. All three are asserted below.
            use std::sync::atomic::{AtomicBool, Ordering};

            use axum::http::header::{
                ACCESS_CONTROL_ALLOW_ORIGIN, ORIGIN, WWW_AUTHENTICATE,
            };
            use tower::ServiceBuilder;

            use crate::{AuthBearerLayer, CorsConfig, CorsLayer};

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
                            .body(Body::from("handler-should-never-run"))
                            .unwrap()
                    })
                }
            }

            let invoked = Arc::new(AtomicBool::new(false));
            let mut registry = EndpointRegistry::new();
            registry.register(TrackingEndpoint {
                invoked: Arc::clone(&invoked),
            });
            let router = Router::new(registry);

            // Permissive "*" config so the outer CorsLayer has something to
            // inject on the way back. A restrictive default would skip the
            // injection, which would mask the propagation assertion.
            let cors = CorsLayer::new(CorsConfig {
                allow_origins: vec!["*".to_string()],
                ..Default::default()
            });

            // ServiceBuilder layer ordering: first `.layer()` is outermost.
            // CorsLayer wraps AuthBearerLayer wraps Router. Inbound:
            // Cors → Auth → Router. Outbound: Router → Auth → Cors → caller.
            let svc = ServiceBuilder::new()
                .layer(cors)
                .layer(AuthBearerLayer::new())
                .service(router);

            let req = AxumRequest::builder()
                .method("GET")
                .uri("/protected")
                // Origin header pins CorsLayer's `*`-injection branch on the
                // response side — without it, the outer layer's wildcard
                // emission still fires (the `select_allowed_origin` rule
                // returns `*` regardless of request Origin), but including the
                // header keeps the test's intent clear: a real cross-origin
                // request that auth rejects.
                .header(ORIGIN, "https://example.com")
                // DELIBERATELY no Authorization header — AuthBearerLayer's
                // MissingHeader rejection branch fires.
                .body(Body::empty())
                .unwrap();
            let resp = svc.oneshot(req).await.unwrap();

            assert_eq!(
                resp.status(),
                StatusCode::UNAUTHORIZED,
                "AuthBearerLayer's 401 must propagate up through the outer CorsLayer unchanged"
            );
            assert_eq!(
                resp.headers()
                    .get(WWW_AUTHENTICATE)
                    .and_then(|v| v.to_str().ok()),
                Some("Bearer"),
                "WWW-Authenticate header from AuthBearerLayer must survive propagation through the outer CorsLayer"
            );
            assert_eq!(
                resp.headers()
                    .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                    .and_then(|v| v.to_str().ok()),
                Some("*"),
                "CorsLayer's response-side ACAO injection must run on the propagated 401, not just on 2xx from the handler"
            );
            let body = body_string(resp).await;
            assert_eq!(
                body, "Missing Authorization header",
                "AuthBearerLayer's 401 body must survive propagation unchanged — the outer CorsLayer must not swallow or replace it"
            );
            assert!(
                !invoked.load(Ordering::Relaxed),
                "Router's handler must NEVER be invoked when an upstream auth layer rejects the request"
            );
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
    use std::sync::Arc;
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
    async fn start_returns_addr_in_use_when_port_already_bound() {
        // PRD-mandated verification for JOLT-RS-027: port binding conflict
        // handling. Bind a std::net::TcpListener (which does NOT set
        // SO_REUSEADDR) and HOLD it, then call start on the same port. The
        // bind inside start must fail with `io::ErrorKind::AddrInUse`.
        //
        // tokio::net::TcpListener sets SO_REUSEADDR before binding on all
        // platforms; on macOS this allows two listeners on the same address
        // (the OS round-robins accepts). Using std::net::TcpListener avoids
        // that and forces a real bind conflict.
        use axum::Router;
        use std::io;
        use std::net::TcpListener;

        let probe = TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();

        let result = JoltServer::new().port(port).start(Router::new()).await;
        let err = result.expect_err("start should fail on port already held by probe");
        assert_eq!(
            err.kind(),
            io::ErrorKind::AddrInUse,
            "expected AddrInUse, got {:?}",
            err
        );

        drop(probe);
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

    #[test]
    fn build_serving_router_emits_tracing_events_for_each_request() {
        // JOLT-RS-069: the customized TraceLayer in build_serving_router
        // must emit INFO-level tracing events with method, uri, status, and
        // latency_ms fields. Runs in a dedicated OS thread with its own
        // tokio runtime so `tracing::subscriber::with_default` can wrap
        // async work without nesting runtimes.
        use std::io;
        use std::sync::Mutex;

        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let events_cap = Arc::clone(&events);

        let make_writer = move || {
            let events = Arc::clone(&events_cap);
            struct CaptureWriter {
                events: Arc<Mutex<Vec<String>>>,
            }
            impl io::Write for CaptureWriter {
                fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                    if let Ok(s) = std::str::from_utf8(buf) {
                        self.events.lock().unwrap().push(s.to_string());
                    }
                    Ok(buf.len())
                }
                fn flush(&mut self) -> io::Result<()> {
                    Ok(())
                }
            }
            CaptureWriter { events }
        };

        let subscriber = tracing_subscriber::fmt()
            .compact()
            .with_target(false)
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_writer(make_writer)
            .with_max_level(tracing::Level::INFO)
            .finish();

        let events_ret = Arc::clone(&events);

        std::thread::spawn(move || {
            tracing::subscriber::with_default(subscriber, || {
                let rt = tokio::runtime::Runtime::new().expect("create runtime");
                rt.block_on(async {
                    let router =
                        JoltServer::new().build_serving_router(axum::Router::new());
                    let req = axum::http::Request::builder()
                        .method("GET")
                        .uri("/api/test")
                        .body(Body::empty())
                        .unwrap();
                    let _resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
                });
            });
        })
        .join()
        .expect("dedicated thread should not panic");

        let captured: Vec<String> = events_ret.lock().unwrap().clone();
        let all = captured.concat();

        assert!(
            all.contains("GET"),
            "log output must include HTTP method, got: {all}"
        );
        assert!(
            all.contains("/api/test"),
            "log output must include request path, got: {all}"
        );
        assert!(
            all.contains("404"),
            "log output must include status code 404, got: {all}"
        );
        assert!(
            all.contains("latency_ms"),
            "log output must include latency in ms, got: {all}"
        );
    }

    #[test]
    fn body_log_layer_emits_debug_event_for_request_body() {
        // JOLT-RS-070: the BodyLogLayer in build_serving_router must emit
        // DEBUG-level tracing events with the request body content for
        // non-sensitive paths.
        use std::io;
        use std::sync::Mutex;

        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let events_cap = Arc::clone(&events);

        let make_writer = move || {
            let events = Arc::clone(&events_cap);
            struct CaptureWriter {
                events: Arc<Mutex<Vec<String>>>,
            }
            impl io::Write for CaptureWriter {
                fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                    if let Ok(s) = std::str::from_utf8(buf) {
                        self.events.lock().unwrap().push(s.to_string());
                    }
                    Ok(buf.len())
                }
                fn flush(&mut self) -> io::Result<()> {
                    Ok(())
                }
            }
            CaptureWriter { events }
        };

        let subscriber = tracing_subscriber::fmt()
            .compact()
            .with_target(false)
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_writer(make_writer)
            .with_max_level(tracing::Level::DEBUG)
            .finish();

        let events_ret = Arc::clone(&events);

        std::thread::spawn(move || {
            tracing::subscriber::with_default(subscriber, || {
                let rt = tokio::runtime::Runtime::new().expect("create runtime");
                rt.block_on(async {
                    let router =
                        JoltServer::new().build_serving_router(axum::Router::new());
                    let req = axum::http::Request::builder()
                        .method("POST")
                        .uri("/api/echo")
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"message":"hello jolt"}"#))
                        .unwrap();
                    let _resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
                });
            });
        })
        .join()
        .expect("dedicated thread should not panic");

        let captured: Vec<String> = events_ret.lock().unwrap().clone();
        let all = captured.concat();

        assert!(
            all.contains(r#"{"message":"hello jolt"}"#),
            "DEBUG log must include request body, got: {all}"
        );
        assert!(
            all.contains("REQ"),
            "DEBUG log must include REQ direction tag, got: {all}"
        );
    }

    #[test]
    fn body_log_layer_suppresses_body_for_auth_paths() {
        // JOLT-RS-070: the BodyLogLayer must NOT log body content for
        // sensitive path prefixes (paths containing "/auth").
        use std::io;
        use std::sync::Mutex;

        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let events_cap = Arc::clone(&events);

        let make_writer = move || {
            let events = Arc::clone(&events_cap);
            struct CaptureWriter {
                events: Arc<Mutex<Vec<String>>>,
            }
            impl io::Write for CaptureWriter {
                fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                    if let Ok(s) = std::str::from_utf8(buf) {
                        self.events.lock().unwrap().push(s.to_string());
                    }
                    Ok(buf.len())
                }
                fn flush(&mut self) -> io::Result<()> {
                    Ok(())
                }
            }
            CaptureWriter { events }
        };

        let subscriber = tracing_subscriber::fmt()
            .compact()
            .with_target(false)
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_writer(make_writer)
            .with_max_level(tracing::Level::DEBUG)
            .finish();

        let events_ret = Arc::clone(&events);

        std::thread::spawn(move || {
            tracing::subscriber::with_default(subscriber, || {
                let rt = tokio::runtime::Runtime::new().expect("create runtime");
                rt.block_on(async {
                    let router =
                        JoltServer::new().build_serving_router(axum::Router::new());
                    let req = axum::http::Request::builder()
                        .method("POST")
                        .uri("/auth/login")
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"password":"secret123"}"#))
                        .unwrap();
                    let _resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
                });
            });
        })
        .join()
        .expect("dedicated thread should not panic");

        let captured: Vec<String> = events_ret.lock().unwrap().clone();
        let all = captured.concat();

        assert!(
            !all.contains("secret123"),
            "sensitive path /auth must NOT log body, got: {all}"
        );
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

    #[tokio::test]
    async fn empty_body_is_rejected_with_400_and_marks_finished() {
        // JOLT-RS-062: empty body → error. An empty body cannot deserialize
        // into any T (serde_json fails to parse zero bytes as a valid JSON
        // value for a struct), so ParseBodyLayer must short-circuit with 400.
        let inner = tower::service_fn(|_: AxumRequest| async move {
            panic!("ParseBodyService must short-circuit on empty body and never call inner");
            #[allow(unreachable_code)]
            Ok::<Response, Infallible>(Response::new(Body::empty()))
        });
        let layer = ParseBodyLayer::<TestBody>::new();
        let svc = layer.layer(inner);

        let request_ext = Arc::new(RequestExt::new());
        let mut req = AxumRequest::builder()
            .method(HttpMethod::POST)
            .uri("/api/test")
            .header("content-type", "application/json")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "empty body must surface 400 Bad Request",
        );
        let body_bytes = axum::body::to_bytes(resp.into_body(), u32::MAX as usize)
            .await
            .unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("400 body is UTF-8");
        assert!(
            body_text.starts_with("Invalid JSON: "),
            "empty body must produce 'Invalid JSON: ...' text; got {body_text:?}",
        );
        assert!(
            request_ext.is_finished(),
            "ParseBodyService must flip RequestExt::mark_finished on empty body",
        );
    }

    #[tokio::test]
    async fn oversized_body_is_rejected_with_413_and_marks_finished() {
        // JOLT-RS-062: oversized body → error. A body exceeding the
        // configured max_body_size must short-circuit with 413 Payload Too
        // Large and flip the finished latch, without invoking the inner svc.
        let inner = tower::service_fn(|_: AxumRequest| async move {
            panic!("ParseBodyService must short-circuit on oversized body and never call inner");
            #[allow(unreachable_code)]
            Ok::<Response, Infallible>(Response::new(Body::empty()))
        });
        let limit: usize = 32;
        let layer = ParseBodyLayer::<TestBody>::new().max_body_size(limit);
        let svc = layer.layer(inner);

        let request_ext = Arc::new(RequestExt::new());
        // Build a body that exceeds the 32-byte limit.
        let large_payload = b"x".repeat(limit + 1);
        let mut req = AxumRequest::builder()
            .method(HttpMethod::POST)
            .uri("/api/test")
            .header("content-type", "application/json")
            .body(Body::from(large_payload))
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::PAYLOAD_TOO_LARGE,
            "oversized body must surface 413 Payload Too Large",
        );
        let body_bytes = axum::body::to_bytes(resp.into_body(), u32::MAX as usize)
            .await
            .unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("413 body is UTF-8");
        assert!(
            body_text.starts_with("Body exceeds maximum allowed size:"),
            "413 body must state the size limit; got {body_text:?}",
        );
        assert!(
            request_ext.is_finished(),
            "ParseBodyService must flip RequestExt::mark_finished on oversized body",
        );
    }

    #[tokio::test]
    async fn text_plain_body_is_decoded_and_inserted_into_extensions() {
        // JOLT-RS-062: string body → success. A text/plain body must be
        // decoded as UTF-8 String and inserted into request extensions.
        use crate::ParseBodyStringLayer;

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

        let payload = b"hello, jolt";
        let req = AxumRequest::builder()
            .method(HttpMethod::POST)
            .uri("/api/text")
            .body(Body::from(&payload[..]))
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            captured.lock().unwrap().clone(),
            Some("hello, jolt".to_string()),
            "text/plain body must be decoded as UTF-8 and land in extensions as String",
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

mod parse_query_typed_extras {
    //! PRD-mandated verification for JOLT-RS-065: "Unit test: ?active=true →
    //! `bool true`. ?sort=asc → `enum Sort::Asc`."
    //!
    //! Three sibling extractors land in 065 because their target shapes
    //! don't match the 064 `extract::<T: FromStr>` bound:
    //!
    //! - `extract_bool` adds the `"1"`/`"0"` aliases (case-insensitive) on
    //!   top of bool's FromStr (which only accepts the literal "true" /
    //!   "false"). Tests pin all four accepted forms PLUS the rejection
    //!   path so a regression that loosens the alias set (e.g., starts
    //!   accepting "yes") gets caught.
    //! - `extract_string` is a pass-through; the test pins that intent
    //!   plus the missing-key path (the only failure mode this helper has).
    //! - `extract_enum` targets `TryFrom<&str>` so user enums plug in. The
    //!   PRD-mandated `?sort=asc → Sort::Asc` test uses a hand-rolled
    //!   `TryFrom<&str>` impl on a local enum; the rejection-path test
    //!   confirms the underlying error's `Display` is propagated into the
    //!   `Invalid` variant's `message` field so `bad_request_for_query_error`
    //!   has a useful payload for HTTP callers.
    //!
    //! 066's `Vec<T>` extractor is explicitly NOT covered here — it has its
    //! own per-element error story.
    use std::collections::HashMap;
    use std::fmt;

    use crate::{
        extract_query_bool, extract_query_enum, extract_query_string, QueryExtractError,
        QueryParams,
    };

    fn params_from(pairs: &[(&str, &str)]) -> QueryParams {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        QueryParams::from(map)
    }

    #[test]
    fn bool_extracts_true_from_all_accepted_aliases() {
        // PRD: ?active=true → bool true. Plus the 065-mandated aliases
        // ("True"/"TRUE"/"1") which bool's FromStr rejects, so this test
        // pins the full case-insensitive + numeric-alias surface that
        // distinguishes `extract_bool` from `extract::<bool>`.
        for alias in ["true", "True", "TRUE", "1"] {
            let params = params_from(&[("active", alias)]);
            let parsed = extract_query_bool(&params, "active")
                .unwrap_or_else(|err| panic!("?active={alias} must parse as true, got {err}"));
            assert!(parsed, "?active={alias} must yield true");
        }
    }

    #[test]
    fn bool_extracts_false_from_all_accepted_aliases() {
        for alias in ["false", "False", "FALSE", "0"] {
            let params = params_from(&[("active", alias)]);
            let parsed = extract_query_bool(&params, "active")
                .unwrap_or_else(|err| panic!("?active={alias} must parse as false, got {err}"));
            assert!(!parsed, "?active={alias} must yield false");
        }
    }

    #[test]
    fn bool_invalid_value_yields_invalid_error_with_alias_hint() {
        // A regression that starts accepting "yes" or "maybe" would silently
        // widen the bool surface beyond what the PRD mandates; this test
        // catches that. The Invalid message must name the four accepted
        // forms so the 400 body is self-explanatory to HTTP callers.
        let params = params_from(&[("active", "maybe")]);
        let err = extract_query_bool(&params, "active")
            .expect_err("?active=maybe must NOT parse as bool");
        match err {
            QueryExtractError::Invalid {
                key,
                value,
                message,
            } => {
                assert_eq!(key, "active");
                assert_eq!(value, "maybe");
                assert!(
                    message.contains("true") && message.contains("1"),
                    "Invalid message must enumerate accepted forms, got {message}",
                );
            }
            other => panic!("expected QueryExtractError::Invalid, got {other:?}"),
        }
    }

    #[test]
    fn string_pass_through_returns_raw_value() {
        // The pass-through helper exists to skip the Result<String,
        // Infallible> chain `extract::<String>` would emit. The test pins
        // that the value comes back byte-identical, including ASCII case
        // and embedded punctuation that a future "smart" parser might be
        // tempted to normalize.
        let params = params_from(&[("name", "Jolt-Rs/064")]);
        let parsed = extract_query_string(&params, "name").expect("present key must succeed");
        assert_eq!(parsed, "Jolt-Rs/064");
    }

    #[test]
    fn string_missing_key_yields_missing_variant() {
        // The pass-through helper still has to surface Missing for absent
        // keys (since pass-through is the only failure mode it has). A
        // regression that returned an empty String instead would silently
        // hide required-parameter omissions from the AutoMiddleware codegen.
        let params = params_from(&[("other", "x")]);
        let err = extract_query_string(&params, "name")
            .expect_err("absent key must NOT yield a string");
        match err {
            QueryExtractError::Missing { key } => assert_eq!(key, "name"),
            other => panic!("expected QueryExtractError::Missing, got {other:?}"),
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Sort {
        Asc,
        Desc,
    }

    #[derive(Debug)]
    struct ParseSortError(String);

    impl fmt::Display for ParseSortError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "expected one of asc|desc, got {}", self.0)
        }
    }

    impl<'a> TryFrom<&'a str> for Sort {
        type Error = ParseSortError;

        fn try_from(s: &'a str) -> Result<Self, Self::Error> {
            match s {
                "asc" => Ok(Sort::Asc),
                "desc" => Ok(Sort::Desc),
                other => Err(ParseSortError(other.to_string())),
            }
        }
    }

    #[test]
    fn enum_extracts_via_try_from_str() {
        // PRD: ?sort=asc → enum Sort::Asc. The hand-rolled TryFrom<&str>
        // impl above is the same shape `#[derive(strum::EnumString)]` would
        // emit; the HRTB on `extract_enum`'s bound is what lets both wire
        // in without lifetime gymnastics on the call site.
        let params = params_from(&[("sort", "asc")]);
        let parsed: Sort =
            extract_query_enum(&params, "sort").expect("?sort=asc must parse via TryFrom");
        assert_eq!(parsed, Sort::Asc);

        let params_desc = params_from(&[("sort", "desc")]);
        let parsed_desc: Sort = extract_query_enum(&params_desc, "sort")
            .expect("?sort=desc must parse via TryFrom");
        assert_eq!(parsed_desc, Sort::Desc);
    }

    #[test]
    fn enum_invalid_value_propagates_underlying_error_into_invalid_message() {
        // The Display impl on `ParseSortError` includes the rejected value;
        // this test pins that the helper forwards that detail into the
        // Invalid variant's `message` field rather than swallowing it. A
        // regression that wrote a generic "parse failed" message would
        // strip the actionable detail from the eventual 400 body.
        let params = params_from(&[("sort", "sideways")]);
        let err = extract_query_enum::<Sort>(&params, "sort")
            .expect_err("?sort=sideways must NOT parse via TryFrom");
        match err {
            QueryExtractError::Invalid {
                key,
                value,
                message,
            } => {
                assert_eq!(key, "sort");
                assert_eq!(value, "sideways");
                assert!(
                    message.contains("asc") && message.contains("desc"),
                    "Invalid message must propagate the underlying parser detail, got {message}",
                );
            }
            other => panic!("expected QueryExtractError::Invalid, got {other:?}"),
        }
    }
}

mod parse_query_typed_vec {
    //! PRD-mandated verification for JOLT-RS-066: "Unit test: ?ids=1,2,3 →
    //! `Vec<i32> = [1,2,3]`."
    //!
    //! `extract_vec<T: FromStr>` is a sibling of the 064 generic extractor,
    //! NOT an extension — the per-element split-and-parse loop produces a
    //! `Vec<T>` rather than a single `T`, and the element-level failure
    //! surface is its own `InvalidElement` variant (carries the failing
    //! element's zero-based index) rather than smuggling the position
    //! through the existing `Invalid` variant's message text.
    //!
    //! Tests pin:
    //! - The PRD success path (`?ids=1,2,3` → `Ok(vec![1, 2, 3])`).
    //! - Single-value path (no commas → one-element vec).
    //! - Element-level failure with index in `InvalidElement`.
    //! - 400 body renders with `[index]='value'` so HTTP callers can read
    //!   off the failing position without diffing the input.
    //! - Missing key → `Missing` variant (not an empty vec — preserves the
    //!   absent-vs-present distinction the rest of the typed-extractor
    //!   surface relies on).
    //! - Float `FromStr` round-trip — pins the generic shape so a
    //!   regression to int-only gets caught.
    use std::collections::HashMap;

    use axum::body::to_bytes;
    use axum::http::StatusCode;

    use crate::{
        bad_request_for_query_error, extract_query_vec, QueryExtractError, QueryParams,
    };

    fn params_from(pairs: &[(&str, &str)]) -> QueryParams {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        QueryParams::from(map)
    }

    #[test]
    fn vec_extracts_comma_separated_ints() {
        // PRD: ?ids=1,2,3 → Vec<i32> = [1,2,3].
        let params = params_from(&[("ids", "1,2,3")]);
        let parsed: Vec<i32> =
            extract_query_vec(&params, "ids").expect("?ids=1,2,3 must parse as Vec<i32>");
        assert_eq!(parsed, vec![1, 2, 3]);
    }

    #[test]
    fn vec_extracts_single_value_as_one_element_vec() {
        // No comma in the value → one-element vec. The split-and-parse
        // shape must NOT special-case "no comma → reject" or "no comma →
        // scalar"; it always yields a Vec<T>.
        let params = params_from(&[("ids", "42")]);
        let parsed: Vec<i32> =
            extract_query_vec(&params, "ids").expect("?ids=42 must parse as one-element Vec<i32>");
        assert_eq!(parsed, vec![42]);
    }

    #[test]
    fn vec_extracts_floats_via_from_str() {
        // The generic shape must work for any T: FromStr, not just ints.
        // Pinning `f64` here catches a regression that quietly narrowed
        // the bound to integer types only.
        let params = params_from(&[("ratios", "0.5,1.5,2.5")]);
        let parsed: Vec<f64> = extract_query_vec(&params, "ratios")
            .expect("?ratios=0.5,1.5,2.5 must parse as Vec<f64>");
        assert_eq!(parsed.len(), 3);
        assert!((parsed[0] - 0.5).abs() < 1e-9);
        assert!((parsed[1] - 1.5).abs() < 1e-9);
        assert!((parsed[2] - 2.5).abs() < 1e-9);
    }

    #[test]
    fn vec_invalid_element_yields_invalid_element_variant_with_index() {
        // ?ids=1,abc,3 → element at index 1 fails. The InvalidElement
        // variant must carry the failing index (1), the failing element's
        // raw value ("abc"), and the underlying parser's detail. A
        // regression that fell back to the scalar `Invalid` variant would
        // strip the index, and an HTTP caller debugging "?ids=1,abc,3"
        // would have to diff the input themselves to find the bad
        // element.
        let params = params_from(&[("ids", "1,abc,3")]);
        let err = extract_query_vec::<i32>(&params, "ids")
            .expect_err("?ids=1,abc,3 must NOT parse as Vec<i32>");
        match &err {
            QueryExtractError::InvalidElement {
                key,
                index,
                value,
                message,
            } => {
                assert_eq!(key, "ids");
                assert_eq!(*index, 1, "second element (zero-based 1) is the failing one");
                assert_eq!(value, "abc");
                assert!(
                    !message.is_empty(),
                    "InvalidElement message must carry the underlying parser detail",
                );
            }
            other => panic!("expected QueryExtractError::InvalidElement, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn vec_invalid_element_renders_into_400_with_index_in_body() {
        // The 400 body shape for InvalidElement must surface the failing
        // index AND the failing element's raw value so an HTTP caller can
        // read "element [1]='abc'" off the response without parsing the
        // request URI themselves. A regression that flattened InvalidElement
        // to the scalar Invalid format would strip the index from the body.
        let params = params_from(&[("ids", "1,abc,3")]);
        let err = extract_query_vec::<i32>(&params, "ids")
            .expect_err("?ids=1,abc,3 must NOT parse as Vec<i32>");

        let resp = bad_request_for_query_error(&err);
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body_text = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert!(
            body_text.starts_with("Invalid query parameter 'ids' element [1]='abc': "),
            "400 body must echo key + index + element value + detail, got {body_text}",
        );
        assert!(
            body_text.len() > "Invalid query parameter 'ids' element [1]='abc': ".len(),
            "400 body must include the underlying parser's detail (got bare prefix)",
        );
    }

    #[test]
    fn vec_missing_key_yields_missing_variant_not_empty_vec() {
        // The absent-vs-present distinction is load-bearing: the rest of
        // the typed-extractor surface preserves it (Missing → required-
        // parameter error; Invalid → bad-value error). Returning an empty
        // vec for an absent key would silently collapse those into one
        // shape and force the AutoMiddleware codegen to re-derive the
        // distinction from the response body.
        let params = params_from(&[("other", "1,2,3")]);
        let err = extract_query_vec::<i32>(&params, "ids")
            .expect_err("absent key must NOT yield a parsed Vec");
        match err {
            QueryExtractError::Missing { key } => assert_eq!(key, "ids"),
            other => panic!("expected QueryExtractError::Missing, got {other:?}"),
        }
    }
}

mod auth_bearer {
    //! PRD-mandated verification for JOLT-RS-071: "Unit test: no Authorization
    //! header → 401."
    //!
    //! The structural surface is wider than the single PRD-mandated case:
    //! 071's contract is the FORMAT validator (`Bearer <token>` shape) plus
    //! the rejection-side guarantees (401 + `WWW-Authenticate: Bearer` +
    //! `mark_finished` + inner-skip + distinct-body-per-reason). Each test
    //! below pins one of those guarantees so 072+'s additions (JWT validation,
    //! claims extraction) compose against a stable 071 surface rather than
    //! re-discovering it.
    //!
    //! Module is named `auth_bearer` so `cargo test -p jolt-core -- tests::auth_bearer`
    //! filters cleanly to this slice; matches the existing `parse_body` /
    //! `parse_query` naming convention for layer-scoped test bundles.
    use std::convert::Infallible;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::extract::Request as AxumRequest;
    use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE};
    use axum::http::{Method as HttpMethod, StatusCode};
    use axum::response::Response;
    use tower::{Layer, ServiceExt};

    use crate::auth_bearer::BearerToken;
    use crate::request_ext::RequestExt;
    use crate::AuthBearerLayer;

    /// Inner service that panics if invoked. Used by every rejection-path
    /// test so a regression that accidentally delegates past a malformed
    /// header (instead of short-circuiting) crashes loudly rather than
    /// passing silently.
    fn forbid_inner() -> impl tower::Service<
        AxumRequest,
        Response = Response,
        Error = Infallible,
        Future = std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Response, Infallible>> + Send>,
        >,
    > + Clone {
        tower::service_fn(|_req: AxumRequest| {
            Box::pin(async move {
                panic!("AuthBearerService must short-circuit on rejection and never call inner");
                #[allow(unreachable_code)]
                Ok::<Response, Infallible>(Response::new(Body::empty()))
            })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<Response, Infallible>> + Send>,
                >
        })
    }

    #[tokio::test]
    async fn missing_authorization_header_short_circuits_with_401() {
        // PRD-mandated test: no Authorization header → 401. Also pins the
        // four sibling guarantees (WWW-Authenticate challenge,
        // text/plain body, finished latch flipped, inner never invoked).
        let layer = AuthBearerLayer::new();
        let svc = layer.layer(forbid_inner());

        let request_ext = Arc::new(RequestExt::new());
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/protected")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "missing Authorization header must surface 401"
        );
        let www_auth = resp
            .headers()
            .get(WWW_AUTHENTICATE)
            .expect("401 must carry WWW-Authenticate")
            .to_str()
            .expect("WWW-Authenticate is ASCII");
        assert_eq!(
            www_auth, "Bearer",
            "WWW-Authenticate must advertise the Bearer scheme",
        );
        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .expect("401 must carry Content-Type")
            .to_str()
            .expect("Content-Type is ASCII");
        assert!(
            content_type.starts_with("text/plain"),
            "401 body is text/plain; got {content_type}"
        );
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("401 body is UTF-8");
        assert_eq!(
            body_text, "Missing Authorization header",
            "missing-header rejection must use the dedicated body shape"
        );

        assert!(
            request_ext.is_finished(),
            "AuthBearerService must flip RequestExt::mark_finished on rejection"
        );
        assert!(
            request_ext.take_response().is_none(),
            "the 401 is returned directly from call(), not stashed in RequestExt"
        );
    }

    #[tokio::test]
    async fn non_bearer_authorization_header_short_circuits_with_401() {
        // A scheme other than Bearer (e.g. `Basic dXNlcjpwYXNz`) is rejected
        // with the dedicated `MissingBearerPrefix` body so the caller sees
        // *why* the header was rejected rather than a generic 401.
        let layer = AuthBearerLayer::new();
        let svc = layer.layer(forbid_inner());

        let request_ext = Arc::new(RequestExt::new());
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/protected")
            .header(AUTHORIZATION, "Basic dXNlcjpwYXNz")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("body is UTF-8");
        assert_eq!(
            body_text, "Invalid Authorization header format: expected 'Bearer <token>'",
            "non-Bearer scheme must use the format-specific rejection body",
        );
        assert!(request_ext.is_finished());
    }

    #[tokio::test]
    async fn empty_bearer_token_short_circuits_with_401() {
        // Header of `Bearer ` (prefix + space, no token) is structurally
        // parseable but semantically empty. The dedicated `EmptyToken` body
        // disambiguates this case from the missing-header case.
        let layer = AuthBearerLayer::new();
        let svc = layer.layer(forbid_inner());

        let request_ext = Arc::new(RequestExt::new());
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/protected")
            .header(AUTHORIZATION, "Bearer ")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("body is UTF-8");
        assert_eq!(
            body_text, "Empty bearer token",
            "empty-token rejection must use the dedicated body shape",
        );
        assert!(request_ext.is_finished());
    }

    #[tokio::test]
    async fn non_ascii_authorization_header_short_circuits_with_401() {
        // A header carrying non-visible-ASCII bytes (e.g. an embedded NUL)
        // can't round-trip through HeaderValue::to_str. The layer rejects
        // with 401 + `NotAscii` body rather than 400, since the failure is
        // an auth-contract violation rather than a generic body issue.
        let layer = AuthBearerLayer::new();
        let svc = layer.layer(forbid_inner());

        let request_ext = Arc::new(RequestExt::new());
        // HeaderValue::from_bytes accepts visible ASCII + extended bytes
        // (0x80-0xFF). 0xC3 is an extended byte that to_str() rejects as
        // non-ASCII. Build the request via headers_mut() so we can plant a
        // raw HeaderValue without going through the builder's safer path.
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/protected")
            .body(Body::empty())
            .unwrap();
        let raw = axum::http::HeaderValue::from_bytes(b"Bearer \xC3\x28").unwrap();
        req.headers_mut().insert(AUTHORIZATION, raw);
        req.extensions_mut().insert(Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("body is UTF-8");
        assert_eq!(body_text, "Authorization header is not valid ASCII");
        assert!(request_ext.is_finished());
    }

    #[tokio::test]
    async fn valid_bearer_header_extracts_token_into_extensions() {
        // Happy path: `Authorization: Bearer <token>` → token lands in
        // extensions as `BearerToken`, finished latch UNTOUCHED, inner
        // service IS invoked and its response is propagated unchanged.
        let layer = AuthBearerLayer::new();
        let inner = tower::service_fn(|req: AxumRequest| async move {
            // Inner observes the BearerToken in extensions; if it's missing
            // or has the wrong content, surface a 500 so the test sees the
            // contract violation as a status-code mismatch.
            match req.extensions().get::<BearerToken>() {
                Some(token) if token.as_str() == "eyJhbGciOiJIUzI1NiJ9" => {
                    Ok::<Response, Infallible>(
                        Response::builder()
                            .status(StatusCode::OK)
                            .body(Body::from("authorized"))
                            .unwrap(),
                    )
                }
                _ => Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .unwrap()),
            }
        });
        let svc = layer.layer(inner);

        let request_ext = Arc::new(RequestExt::new());
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/protected")
            .header(AUTHORIZATION, "Bearer eyJhbGciOiJIUzI1NiJ9")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "valid bearer header must allow inner service to run",
        );
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        assert_eq!(&body_bytes[..], b"authorized");

        assert!(
            !request_ext.is_finished(),
            "happy path must NOT mark the request finished",
        );
    }

    #[tokio::test]
    async fn case_insensitive_scheme_is_accepted() {
        // RFC 7235 §2.1 declares the auth-scheme name case-insensitive, so
        // `bearer <token>` and `BEARER <token>` are accepted as semantically
        // identical to `Bearer <token>`. Pin both lowercase and uppercase
        // variants so a future iteration that drifts to a strict-case match
        // gets caught.
        for scheme in ["bearer", "BEARER", "BeArEr"] {
            let layer = AuthBearerLayer::new();
            let inner = tower::service_fn(|req: AxumRequest| async move {
                let token = req
                    .extensions()
                    .get::<BearerToken>()
                    .expect("case-insensitive scheme must still extract the token")
                    .clone();
                Ok::<Response, Infallible>(
                    Response::builder()
                        .status(StatusCode::OK)
                        .body(Body::from(token.as_str().to_owned()))
                        .unwrap(),
                )
            });
            let svc = layer.layer(inner);

            let header_value = format!("{scheme} eyJ.payload.sig");
            let req = AxumRequest::builder()
                .method(HttpMethod::GET)
                .uri("/api/protected")
                .header(AUTHORIZATION, header_value)
                .body(Body::empty())
                .unwrap();

            let resp = svc.oneshot(req).await.unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "scheme '{scheme}' must be accepted as semantically Bearer"
            );
            let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
            assert_eq!(
                &body_bytes[..],
                b"eyJ.payload.sig",
                "extracted token bytes must match the value past 'Bearer ' regardless of scheme casing",
            );
        }
    }

    #[tokio::test]
    async fn fresh_request_ext_is_injected_when_no_upstream_layer_set_one() {
        // Mirrors ParseBodyService's preserve-or-inject contract: when no
        // upstream Arc<RequestExt> is present, the layer injects a fresh one
        // before flipping mark_finished. Without this, a malformed-header
        // request that didn't go through Router/CorsLayer first would have
        // no observable handle on the finished latch — the contract would
        // exist but be unreachable.
        let layer = AuthBearerLayer::new();
        let svc = layer.layer(forbid_inner());

        // No request_ext insertion — let the layer inject one.
        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/protected")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "rejection must still produce 401 even without an upstream RequestExt",
        );
    }
}

mod auth_jwt {
    //! PRD-mandated verification for JOLT-RS-072: "Unit test: valid token →
    //! passes, expired token → 401."
    //!
    //! The structural surface is wider than the two PRD-mandated cases:
    //! 072's contract is the JWT validator that consumes the [`BearerToken`]
    //! stashed by [`AuthBearerLayer`] (071), calls
    //! [`jolt_utils::jwt::decode`], and short-circuits with 401 + the
    //! `mark_finished` latch on failure. The tests below pin each rejection
    //! path's distinct body so 073+'s downstream consumers can rely on a
    //! stable contract.
    //!
    //! Module is named `auth_jwt` so `cargo test -p jolt-core -- tests::auth_jwt`
    //! filters cleanly; matches the established `auth_bearer` / `parse_body` /
    //! `parse_query` naming convention.
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::body::{to_bytes, Body};
    use axum::extract::Request as AxumRequest;
    use axum::http::header::{CONTENT_TYPE, WWW_AUTHENTICATE};
    use axum::http::{Method as HttpMethod, StatusCode};
    use axum::response::Response;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use tower::{Layer, ServiceExt};

    use crate::auth_bearer::BearerToken;
    use crate::request_ext::RequestExt;
    use crate::{AuthJwtLayer, JwtClaims, JwtConfig};

    /// Inner service that panics if invoked. Used by every rejection-path
    /// test so a regression that accidentally delegates past a failed JWT
    /// decode (instead of short-circuiting) crashes loudly rather than
    /// passing silently. Mirrors the `forbid_inner()` helper in the
    /// sibling `auth_bearer` test module.
    fn forbid_inner() -> impl tower::Service<
        AxumRequest,
        Response = Response,
        Error = Infallible,
        Future = std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Response, Infallible>> + Send>,
        >,
    > + Clone {
        tower::service_fn(|_req: AxumRequest| {
            Box::pin(async move {
                panic!("AuthJwtService must short-circuit on rejection and never call inner");
                #[allow(unreachable_code)]
                Ok::<Response, Infallible>(Response::new(Body::empty()))
            })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<Response, Infallible>> + Send>,
                >
        })
    }

    fn now_secs() -> usize {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is sane")
            .as_secs() as usize
    }

    fn sign_hs256(secret: &[u8], claims: &JwtClaims) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            claims,
            &EncodingKey::from_secret(secret),
        )
        .expect("HS256 encode with static secret never fails")
    }

    /// Build an Authorization-bearing request whose `BearerToken` extension
    /// is already populated (simulating an upstream `AuthBearerLayer` having
    /// run). The `request_ext` Arc is also planted so the test can observe
    /// the `finished` latch.
    fn request_with_bearer_token(
        token: &str,
        request_ext: Arc<RequestExt>,
    ) -> AxumRequest {
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/protected")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&request_ext));
        req.extensions_mut().insert(BearerToken(token.to_owned()));
        req
    }

    #[tokio::test]
    async fn valid_jwt_passes_through_and_inner_runs() {
        // PRD-mandated half 1: valid token → passes. Also pins that the
        // parsed JwtClaims lands in extensions and the finished latch is
        // UNTOUCHED on the happy path.
        let secret = b"jolt-rs-072-auth-jwt-test-secret";
        let exp = now_secs() + 3600;
        let claims = JwtClaims {
            sub: "alice".to_owned(),
            exp,
            iat: Some(now_secs()),
            extra: serde_json::Map::new(),
        };
        let token = sign_hs256(secret, &claims);

        let layer = AuthJwtLayer::new(JwtConfig::new(secret.to_vec(), Algorithm::HS256));
        let inner = tower::service_fn(|req: AxumRequest| async move {
            // Inner observes the parsed JwtClaims in extensions and echoes
            // the sub in the response body so the test can pin the shape.
            let sub = req
                .extensions()
                .get::<JwtClaims>()
                .expect("valid token must surface JwtClaims in extensions")
                .sub
                .clone();
            Ok::<Response, Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from(sub))
                    .unwrap(),
            )
        });
        let svc = layer.layer(inner);

        let request_ext = Arc::new(RequestExt::new());
        let req = request_with_bearer_token(&token, Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "valid JWT must allow the inner service to run"
        );
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        assert_eq!(
            &body_bytes[..],
            b"alice",
            "parsed JwtClaims must land in extensions for the inner handler"
        );
        assert!(
            !request_ext.is_finished(),
            "happy path must NOT mark the request finished"
        );
    }

    #[tokio::test]
    async fn expired_jwt_short_circuits_with_401() {
        // PRD-mandated half 2: expired token → 401. Also pins the body
        // shape, the WWW-Authenticate challenge with the invalid_token
        // error parameter (RFC 6750 §3), the finished latch, and the
        // inner-never-invoked guarantee.
        let secret = b"jolt-rs-072-auth-jwt-test-secret";
        // exp = 1000 → 1970; well in the past.
        let claims = JwtClaims {
            sub: "alice".to_owned(),
            exp: 1_000,
            iat: None,
            extra: serde_json::Map::new(),
        };
        let token = sign_hs256(secret, &claims);

        let layer = AuthJwtLayer::new(JwtConfig::new(secret.to_vec(), Algorithm::HS256));
        let svc = layer.layer(forbid_inner());

        let request_ext = Arc::new(RequestExt::new());
        let req = request_with_bearer_token(&token, Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "expired JWT must surface 401"
        );
        let www_auth = resp
            .headers()
            .get(WWW_AUTHENTICATE)
            .expect("401 must carry WWW-Authenticate")
            .to_str()
            .expect("WWW-Authenticate is ASCII");
        assert_eq!(
            www_auth, r#"Bearer error="invalid_token""#,
            "rejected JWTs use the RFC 6750 invalid_token challenge parameter"
        );
        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .expect("401 must carry Content-Type")
            .to_str()
            .expect("Content-Type is ASCII");
        assert!(
            content_type.starts_with("text/plain"),
            "401 body is text/plain; got {content_type}"
        );
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("401 body is UTF-8");
        assert_eq!(
            body_text, "Token has expired",
            "expired-token rejection uses the dedicated body shape"
        );

        assert!(
            request_ext.is_finished(),
            "AuthJwtService must flip RequestExt::mark_finished on rejection"
        );
        assert!(
            request_ext.take_response().is_none(),
            "the 401 is returned directly from call(), not stashed in RequestExt"
        );
    }

    #[tokio::test]
    async fn invalid_signature_jwt_short_circuits_with_401() {
        // A token signed with a DIFFERENT secret than the validator carries
        // is rejected with the dedicated InvalidSignature body so callers
        // see *why* the token was rejected (vs. a generic 401).
        let claims = JwtClaims {
            sub: "alice".to_owned(),
            exp: now_secs() + 3600,
            iat: None,
            extra: serde_json::Map::new(),
        };
        let token = sign_hs256(b"signed-with-this-secret", &claims);

        let layer = AuthJwtLayer::new(JwtConfig::new(
            b"verify-with-DIFFERENT-secret".to_vec(),
            Algorithm::HS256,
        ));
        let svc = layer.layer(forbid_inner());

        let request_ext = Arc::new(RequestExt::new());
        let req = request_with_bearer_token(&token, Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("body is UTF-8");
        assert_eq!(
            body_text, "Invalid token signature",
            "wrong-secret rejection uses the dedicated body shape"
        );
        assert!(request_ext.is_finished());
    }

    #[tokio::test]
    async fn malformed_jwt_short_circuits_with_401() {
        // A bearer-token-like string that isn't a real JWT (no three
        // dot-separated segments) must reject with the Malformed body.
        let layer = AuthJwtLayer::new(JwtConfig::new(
            b"jolt-rs-072-auth-jwt-test-secret".to_vec(),
            Algorithm::HS256,
        ));
        let svc = layer.layer(forbid_inner());

        let request_ext = Arc::new(RequestExt::new());
        let req = request_with_bearer_token("definitely-not-a-jwt", Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("body is UTF-8");
        assert_eq!(body_text, "Malformed token");
        assert!(request_ext.is_finished());
    }

    #[tokio::test]
    async fn missing_bearer_token_short_circuits_with_401() {
        // AuthJwtLayer requires AuthBearerLayer upstream (or some equivalent
        // BearerToken-stashing layer). With no BearerToken in extensions,
        // the layer rejects with the dedicated MissingBearerToken body.
        let layer = AuthJwtLayer::new(JwtConfig::new(
            b"jolt-rs-072-auth-jwt-test-secret".to_vec(),
            Algorithm::HS256,
        ));
        let svc = layer.layer(forbid_inner());

        let request_ext = Arc::new(RequestExt::new());
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/protected")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&request_ext));
        // NO BearerToken inserted.

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("body is UTF-8");
        assert_eq!(
            body_text, "Missing bearer token",
            "no-token rejection uses the dedicated MissingBearerToken body"
        );
        assert!(request_ext.is_finished());
    }

    #[tokio::test]
    async fn fresh_request_ext_is_injected_when_no_upstream_layer_set_one() {
        // Mirrors AuthBearerService's preserve-or-inject contract: even when
        // no upstream Arc<RequestExt> is present, the layer injects a fresh
        // one before flipping mark_finished. Without this, a malformed-token
        // request reaching AuthJwtLayer standalone would have no observable
        // handle on the finished latch.
        let layer = AuthJwtLayer::new(JwtConfig::new(
            b"jolt-rs-072-auth-jwt-test-secret".to_vec(),
            Algorithm::HS256,
        ));
        let svc = layer.layer(forbid_inner());

        // No RequestExt, no BearerToken — the missing-token branch fires.
        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/protected")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "rejection must still produce 401 even without an upstream RequestExt"
        );
    }

    #[tokio::test]
    async fn valid_jwt_stashes_typed_claims_via_extension_key_jwtclaims() {
        // PRD-mandated verification for JOLT-RS-073: "valid token → JwtClaims
        // available via req.extensions().get::<JwtClaims>()". Pins the
        // extension-key contract independently of the response-body echo
        // covered by `valid_jwt_passes_through_and_inner_runs` (JOLT-RS-072):
        // every field of JwtClaims (sub, exp, iat) must be retrievable from
        // request extensions using the JwtClaims type as the key, with values
        // preserved verbatim from the minted token. Downstream handlers and
        // AutoMiddleware codegen (074+) depend on this contract.
        let secret = b"jolt-rs-073-extension-key-contract-secret";
        let minted_sub = "user-073-extension-key";
        let minted_exp = now_secs() + 7200;
        let minted_iat = now_secs();
        let claims = JwtClaims {
            sub: minted_sub.to_owned(),
            exp: minted_exp,
            iat: Some(minted_iat),
            extra: serde_json::Map::new(),
        };
        let token = sign_hs256(secret, &claims);

        let layer = AuthJwtLayer::new(JwtConfig::new(secret.to_vec(), Algorithm::HS256));

        // Inner service captures the stashed JwtClaims via the typed key and
        // returns the field values through a side channel so the test can
        // assert each field's exact value end-to-end. Using a side channel
        // (rather than serializing through the response body) keeps the
        // assertion focused on the extension-key contract itself.
        let captured: Arc<std::sync::Mutex<Option<JwtClaims>>> =
            Arc::new(std::sync::Mutex::new(None));
        let captured_for_inner = Arc::clone(&captured);
        let inner = tower::service_fn(move |req: AxumRequest| {
            let captured = Arc::clone(&captured_for_inner);
            async move {
                let claims = req
                    .extensions()
                    .get::<JwtClaims>()
                    .expect(
                        "AuthJwtLayer must stash JwtClaims into extensions \
                         under the JwtClaims type key on a valid token",
                    )
                    .clone();
                *captured.lock().unwrap() = Some(claims);
                Ok::<Response, Infallible>(
                    Response::builder()
                        .status(StatusCode::OK)
                        .body(Body::empty())
                        .unwrap(),
                )
            }
        });
        let svc = layer.layer(inner);

        let request_ext = Arc::new(RequestExt::new());
        let req = request_with_bearer_token(&token, Arc::clone(&request_ext));
        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let observed = captured
            .lock()
            .unwrap()
            .clone()
            .expect("inner service must have observed the stashed JwtClaims");
        assert_eq!(
            observed.sub, minted_sub,
            "sub field must round-trip verbatim through the extension key"
        );
        assert_eq!(
            observed.exp, minted_exp,
            "exp field must round-trip verbatim through the extension key"
        );
        assert_eq!(
            observed.iat,
            Some(minted_iat),
            "iat field (Option<usize>) must round-trip verbatim, preserving the Some discriminant"
        );
    }

    #[tokio::test]
    async fn valid_jwt_with_custom_claims_surfaces_extra_in_extension() {
        // PRD-mandated 074 verification (custom claims half): the closing
        // slice of phase16's test bundle. The five sibling cases (missing
        // header, malformed header, wrong algorithm, expired token, valid
        // token) are pinned upstream:
        //   * missing header  — 071's `missing_authorization_header_short_circuits_with_401`
        //   * malformed header — 071's `non_bearer_authorization_header_short_circuits_with_401`
        //   * wrong algorithm  — jolt-utils' `decode_algorithm_mismatch_yields_invalid_algorithm_variant`
        //   * expired token    — 072's `expired_jwt_short_circuits_with_401`
        //   * valid token      — 072's `valid_jwt_passes_through_and_inner_runs`
        //                        + 073's `valid_jwt_stashes_typed_claims_via_extension_key_jwtclaims`
        // This test closes phase16 by pinning the custom-claims contract end-
        // to-end through the AuthJwtLayer (rather than at the jolt-utils
        // decode call alone): a token minted with `role`/`scopes` custom
        // claims must surface those claims via `JwtClaims::extra` on the
        // request-extensions handle used by downstream handlers.
        let secret = b"jolt-rs-074-auth-jwt-custom-claims-secret";
        let mut minted_extra = serde_json::Map::new();
        minted_extra.insert(
            "role".to_owned(),
            serde_json::Value::String("admin".to_owned()),
        );
        minted_extra.insert(
            "scopes".to_owned(),
            serde_json::json!(["read", "write", "admin"]),
        );
        let claims = JwtClaims {
            sub: "user-074".to_owned(),
            exp: now_secs() + 3600,
            iat: Some(now_secs()),
            extra: minted_extra,
        };
        let token = sign_hs256(secret, &claims);

        let layer = AuthJwtLayer::new(JwtConfig::new(secret.to_vec(), Algorithm::HS256));

        // Side-channel capture mirrors the 073 test's pattern: keep the
        // assertion focused on the extension-key contract rather than on
        // serialization-through-HTTP.
        let captured: Arc<std::sync::Mutex<Option<JwtClaims>>> =
            Arc::new(std::sync::Mutex::new(None));
        let captured_for_inner = Arc::clone(&captured);
        let inner = tower::service_fn(move |req: AxumRequest| {
            let captured = Arc::clone(&captured_for_inner);
            async move {
                let claims = req
                    .extensions()
                    .get::<JwtClaims>()
                    .expect(
                        "valid custom-claims token must surface JwtClaims via the typed key",
                    )
                    .clone();
                *captured.lock().unwrap() = Some(claims);
                Ok::<Response, Infallible>(
                    Response::builder()
                        .status(StatusCode::OK)
                        .body(Body::empty())
                        .unwrap(),
                )
            }
        });
        let svc = layer.layer(inner);

        let request_ext = Arc::new(RequestExt::new());
        let req = request_with_bearer_token(&token, Arc::clone(&request_ext));
        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "valid custom-claims token must allow the inner handler to run"
        );

        let observed = captured
            .lock()
            .unwrap()
            .clone()
            .expect("inner service must have observed the stashed JwtClaims");
        assert_eq!(observed.sub, "user-074");
        assert_eq!(
            observed.extra.get("role"),
            Some(&serde_json::Value::String("admin".to_owned())),
            "string-valued custom claim `role` must surface verbatim through extra"
        );
        assert_eq!(
            observed.extra.get("scopes"),
            Some(&serde_json::json!(["read", "write", "admin"])),
            "array-valued custom claim `scopes` must surface verbatim through extra"
        );
        assert!(
            !request_ext.is_finished(),
            "custom-claims happy path must NOT mark the request finished"
        );
    }
}

mod auth_websocket {
    //! PRD-mandated verification for JOLT-RS-075: "Unit test: header
    //! 'jolt-jwt, eyJ...' → extracted token 'eyJ...'."
    //!
    //! The structural surface is wider than the single PRD-mandated case:
    //! 075's contract is the FORMAT extractor for the
    //! `Sec-WebSocket-Protocol: jolt-jwt, <token>` shape. Each rejection-path
    //! variant pinned by [`WsTokenRejectReason`] gets a dedicated test below
    //! so JOLT-RS-076's future tower::Layer can produce a distinct 401 body
    //! per reason.
    //!
    //! Module is named `auth_websocket` so
    //! `cargo test -p jolt-core -- tests::auth_websocket` filters cleanly to
    //! this slice; matches the established `auth_bearer` / `auth_jwt`
    //! naming convention.
    use axum::http::HeaderValue;

    use crate::auth_websocket::{
        extract_jwt_token, WsJwtToken, WsTokenRejectReason, JOLT_JWT_SUBPROTOCOL,
    };

    /// Distinctive non-trivial JWT-shaped string used across tests so a
    /// false-positive (e.g. the helper returning "" or a stripped substring)
    /// surfaces in the assertion. Three dot-separated base64url-ish chunks,
    /// matching real JWT layout.
    const SAMPLE_JWT: &str =
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1c2VyIn0.aBcDeFgHiJkLmNoPqRsTuV";

    #[test]
    fn extract_jwt_token_canonical_shape_yields_token() {
        // PRD-mandated verification: header "jolt-jwt, eyJ..." → token "eyJ...".
        let header = HeaderValue::from_str(&format!("{JOLT_JWT_SUBPROTOCOL}, {SAMPLE_JWT}"))
            .expect("static header value is valid ASCII");
        let extracted = extract_jwt_token(Some(&header))
            .expect("canonical jolt-jwt subprotocol shape must yield the token");
        assert_eq!(
            extracted, SAMPLE_JWT,
            "the second comma-separated subprotocol value is the JWT verbatim"
        );
    }

    #[test]
    fn extract_jwt_token_no_space_after_comma_yields_token() {
        // Whitespace after the comma is BWS (RFC 7230 §7) and must be
        // tolerated. Pinned because browsers occasionally normalize the
        // header without inserting the canonical space.
        let header = HeaderValue::from_str(&format!("{JOLT_JWT_SUBPROTOCOL},{SAMPLE_JWT}"))
            .expect("static header value is valid ASCII");
        let extracted = extract_jwt_token(Some(&header)).expect("BWS-free shape must extract");
        assert_eq!(extracted, SAMPLE_JWT);
    }

    #[test]
    fn extract_jwt_token_extra_whitespace_around_comma_yields_token() {
        // Extra spaces before AND after the comma: still RFC 7230 BWS, still
        // tolerated. `.trim()` (rather than `.trim_start_matches(' ')`)
        // handles tabs as well, but this test pins the spaces case which is
        // what real clients emit.
        let header =
            HeaderValue::from_str(&format!("  {JOLT_JWT_SUBPROTOCOL}   ,   {SAMPLE_JWT}   "))
                .expect("static header value is valid ASCII");
        let extracted = extract_jwt_token(Some(&header)).expect("BWS-tolerant shape must extract");
        assert_eq!(extracted, SAMPLE_JWT);
    }

    #[test]
    fn extract_jwt_token_missing_header_yields_missing_header_variant() {
        let err =
            extract_jwt_token(None).expect_err("absent header must surface MissingHeader rejection");
        assert_eq!(err, WsTokenRejectReason::MissingHeader);
        assert_eq!(err.message(), "Missing Sec-WebSocket-Protocol header");
    }

    #[test]
    fn extract_jwt_token_non_ascii_header_yields_not_ascii_variant() {
        // 0xC3 is an extended-ASCII byte that HeaderValue::from_bytes
        // accepts but HeaderValue::to_str rejects as non-visible-ASCII.
        let header = HeaderValue::from_bytes(b"jolt-jwt, \xC3\x28")
            .expect("HeaderValue::from_bytes accepts extended-ASCII bytes");
        let err = extract_jwt_token(Some(&header))
            .expect_err("non-ASCII header must surface NotAscii rejection");
        assert_eq!(err, WsTokenRejectReason::NotAscii);
        assert_eq!(
            err.message(),
            "Sec-WebSocket-Protocol header is not valid ASCII"
        );
    }

    #[test]
    fn extract_jwt_token_wrong_marker_yields_missing_jolt_jwt_prefix_variant() {
        // First subprotocol is not "jolt-jwt" (case-sensitive per RFC 6455
        // §11.5). Both an outright wrong marker AND a wrong-case marker
        // share the rejection variant — the tests pin both shapes since
        // case-sensitivity is a documented decision (module rustdoc 3).
        let header = HeaderValue::from_str(&format!("graphql-ws, {SAMPLE_JWT}")).unwrap();
        let err = extract_jwt_token(Some(&header)).unwrap_err();
        assert_eq!(err, WsTokenRejectReason::MissingJoltJwtPrefix);

        let header_uppercase = HeaderValue::from_str(&format!("JOLT-JWT, {SAMPLE_JWT}")).unwrap();
        let err_uppercase = extract_jwt_token(Some(&header_uppercase)).unwrap_err();
        assert_eq!(
            err_uppercase,
            WsTokenRejectReason::MissingJoltJwtPrefix,
            "subprotocol literal match must be case-sensitive (RFC 6455 §11.5)"
        );
        assert_eq!(
            err.message(),
            "Invalid Sec-WebSocket-Protocol format: expected 'jolt-jwt, <token>'"
        );
    }

    #[test]
    fn extract_jwt_token_solo_marker_yields_missing_jolt_jwt_prefix_variant() {
        // Header is exactly "jolt-jwt" with no comma + token follow-up. The
        // helper requires exactly two comma-separated parts; one part (even
        // the right marker) is rejected as a malformed shape.
        let header = HeaderValue::from_static("jolt-jwt");
        let err = extract_jwt_token(Some(&header)).unwrap_err();
        assert_eq!(err, WsTokenRejectReason::MissingJoltJwtPrefix);
    }

    #[test]
    fn extract_jwt_token_empty_token_after_marker_yields_empty_token_variant() {
        // Header is "jolt-jwt," (or "jolt-jwt, ") — structurally parseable
        // (two comma-separated parts) but the token slot is empty after
        // whitespace trim. Distinct from MissingHeader and from
        // MissingJoltJwtPrefix because BOTH parts are present, just one is
        // empty.
        let header_no_trailing = HeaderValue::from_static("jolt-jwt,");
        let err1 = extract_jwt_token(Some(&header_no_trailing)).unwrap_err();
        assert_eq!(err1, WsTokenRejectReason::EmptyToken);

        let header_trailing_space = HeaderValue::from_static("jolt-jwt, ");
        let err2 = extract_jwt_token(Some(&header_trailing_space)).unwrap_err();
        assert_eq!(
            err2,
            WsTokenRejectReason::EmptyToken,
            "whitespace-only token slot must trim to empty and reject"
        );
        assert_eq!(err1.message(), "Empty WebSocket JWT token");
    }

    #[test]
    fn extract_jwt_token_more_than_two_subprotocols_yields_malformed_variant() {
        // Three comma-separated subprotocols: ambiguous which is the token.
        // JWTs cannot contain commas (they are dot-separated base64url
        // chunks), so any extra comma means the client offered another
        // subprotocol alongside the auth pair.
        let header =
            HeaderValue::from_str(&format!("jolt-jwt, {SAMPLE_JWT}, graphql-ws")).unwrap();
        let err = extract_jwt_token(Some(&header)).unwrap_err();
        assert_eq!(err, WsTokenRejectReason::MalformedSubprotocols);
        assert_eq!(
            err.message(),
            "Invalid Sec-WebSocket-Protocol: more than two subprotocols offered"
        );
    }

    #[test]
    fn ws_jwt_token_newtype_exposes_borrowed_str() {
        // Pins the public surface of the WsJwtToken handle that JOLT-RS-076
        // will stash into request extensions: construction is via the tuple
        // ctor, and `.as_str()` borrows the underlying token string. The
        // newtype's distinct TypeId is the load-bearing property (075
        // module rustdoc decision 7); this test pins the borrowed-str
        // accessor that callers will use to read the token back.
        let handle = WsJwtToken(SAMPLE_JWT.to_owned());
        assert_eq!(handle.as_str(), SAMPLE_JWT);
        // Equality/Clone derived; pin the round-trip too so a future Debug
        // implementation change doesn't accidentally drop the derive.
        let cloned = handle.clone();
        assert_eq!(handle, cloned);
    }
}

mod auth_ws_jwt {
    //! PRD-mandated verification for JOLT-RS-076: "Integration test: WS
    //! connect with invalid token → 401 response, no upgrade."
    //!
    //! The closest contract test we can write at this point in the port is a
    //! tower-Service-level test: the layer's job is to either (a) short-
    //! circuit with a 401 (preventing the inner WS upgrade handler from
    //! running) or (b) delegate to the inner service (allowing the upgrade
    //! to proceed). Verifying both paths through the tower::Service surface
    //! pins the same contract a real WS-connect integration test would —
    //! the inner service in these tests stands in for the upgrade handler
    //! that JOLT-RS-077 will land. A real `axum::extract::ws::WebSocketUpgrade`-
    //! based integration test is deferred until the WebSocketHandler trait
    //! lands (see spec_rust.md phase 4).
    //!
    //! Module is named `auth_ws_jwt` so
    //! `cargo test -p jolt-core -- tests::auth_ws_jwt` filters cleanly to
    //! this slice; matches the established `auth_bearer` / `auth_jwt` /
    //! `auth_websocket` naming convention.
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::body::{to_bytes, Body};
    use axum::extract::Request as AxumRequest;
    use axum::http::header::{CONTENT_TYPE, SEC_WEBSOCKET_PROTOCOL, WWW_AUTHENTICATE};
    use axum::http::{HeaderValue, Method as HttpMethod, StatusCode};
    use axum::response::Response;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use tower::{Layer, ServiceExt};

    use crate::auth_websocket::{WsJwtToken, JOLT_JWT_SUBPROTOCOL};
    use crate::request_ext::RequestExt;
    use crate::{AuthWsJwtLayer, JwtClaims, JwtConfig};

    /// Inner service that panics if invoked. Used by every rejection-path
    /// test so a regression that accidentally delegates past a failed auth
    /// precheck (instead of short-circuiting and preventing the upgrade)
    /// crashes loudly rather than passing silently. Mirrors the
    /// `forbid_inner` helper in the sibling `auth_jwt` test module.
    fn forbid_inner() -> impl tower::Service<
        AxumRequest,
        Response = Response,
        Error = Infallible,
        Future = std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Response, Infallible>> + Send>,
        >,
    > + Clone {
        tower::service_fn(|_req: AxumRequest| {
            Box::pin(async move {
                panic!(
                    "AuthWsJwtService must short-circuit on rejection and never call inner \
                     (the upgrade must NOT proceed when auth fails)"
                );
                #[allow(unreachable_code)]
                Ok::<Response, Infallible>(Response::new(Body::empty()))
            })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<Response, Infallible>> + Send>,
                >
        })
    }

    fn now_secs() -> usize {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is sane")
            .as_secs() as usize
    }

    fn sign_hs256(secret: &[u8], claims: &JwtClaims) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            claims,
            &EncodingKey::from_secret(secret),
        )
        .expect("HS256 encode with static secret never fails")
    }

    /// Build a WebSocket-shaped request whose `Sec-WebSocket-Protocol` header
    /// carries the given value. The request also carries the upgrade-handshake
    /// headers so a future end-to-end test against an actual WS upgrade
    /// handler doesn't have to mint a different request shape — but 076's
    /// auth precheck only inspects `Sec-WebSocket-Protocol`, so the upgrade
    /// headers are inert for these tests.
    fn ws_request_with_protocol(
        protocol: HeaderValue,
        request_ext: Arc<RequestExt>,
    ) -> AxumRequest {
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/ws")
            .header("connection", "Upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header(SEC_WEBSOCKET_PROTOCOL, protocol)
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&request_ext));
        req
    }

    #[tokio::test]
    async fn invalid_token_short_circuits_with_401_and_no_upgrade() {
        // PRD-mandated half: WS connect with invalid token → 401 response,
        // no upgrade. Pins (a) the 401 status, (b) the dedicated body for
        // the expired-token rejection path, (c) the finished latch flip,
        // (d) the inner-never-invoked guarantee (forbid_inner panics if the
        // upgrade is allowed to proceed).
        let secret = b"jolt-rs-076-auth-ws-jwt-test-secret";
        // exp = 1000 → 1970; well in the past.
        let claims = JwtClaims {
            sub: "alice".to_owned(),
            exp: 1_000,
            iat: None,
            extra: serde_json::Map::new(),
        };
        let token = sign_hs256(secret, &claims);

        let layer = AuthWsJwtLayer::new(JwtConfig::new(secret.to_vec(), Algorithm::HS256));
        let svc = layer.layer(forbid_inner());

        let request_ext = Arc::new(RequestExt::new());
        let header = HeaderValue::from_str(&format!("{JOLT_JWT_SUBPROTOCOL}, {token}"))
            .expect("subprotocol header is ASCII");
        let req = ws_request_with_protocol(header, Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "invalid (expired) WS token must surface 401 and prevent the upgrade"
        );
        // No `WWW-Authenticate: Bearer` challenge on a WS upgrade rejection —
        // see auth_ws_jwt module docs decision 2.
        assert!(
            resp.headers().get(WWW_AUTHENTICATE).is_none(),
            "WS auth rejection must NOT carry a WWW-Authenticate challenge"
        );
        // No `Sec-WebSocket-Protocol` echo on the rejection — see decision 3.
        assert!(
            resp.headers().get(SEC_WEBSOCKET_PROTOCOL).is_none(),
            "WS auth rejection must NOT echo back any selected subprotocol"
        );
        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .expect("401 must carry Content-Type")
            .to_str()
            .expect("Content-Type is ASCII");
        assert!(
            content_type.starts_with("text/plain"),
            "401 body is text/plain; got {content_type}"
        );
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("401 body is UTF-8");
        assert_eq!(
            body_text, "Token has expired",
            "expired-token rejection uses the dedicated body shape from JwtDecodeError::Expired"
        );

        assert!(
            request_ext.is_finished(),
            "AuthWsJwtService must flip RequestExt::mark_finished on rejection"
        );
        assert!(
            request_ext.take_response().is_none(),
            "the 401 is returned directly from call(), not stashed in RequestExt"
        );
    }

    #[tokio::test]
    async fn missing_protocol_header_short_circuits_with_401() {
        // No `Sec-WebSocket-Protocol` header at all → MissingHeader rejection
        // surfaces with the dedicated body string from
        // WsTokenRejectReason::MissingHeader.
        let layer = AuthWsJwtLayer::new(JwtConfig::new(
            b"jolt-rs-076-auth-ws-jwt-test-secret".to_vec(),
            Algorithm::HS256,
        ));
        let svc = layer.layer(forbid_inner());

        let request_ext = Arc::new(RequestExt::new());
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/ws")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("body is UTF-8");
        assert_eq!(
            body_text, "Missing Sec-WebSocket-Protocol header",
            "no-header rejection mirrors WsTokenRejectReason::MissingHeader::message()"
        );
        assert!(request_ext.is_finished());
    }

    #[tokio::test]
    async fn malformed_protocol_header_short_circuits_with_401() {
        // Header is well-formed ASCII but doesn't have the canonical
        // `jolt-jwt, <token>` shape (wrong marker subprotocol) → the
        // extraction-side MissingJoltJwtPrefix body surfaces.
        let layer = AuthWsJwtLayer::new(JwtConfig::new(
            b"jolt-rs-076-auth-ws-jwt-test-secret".to_vec(),
            Algorithm::HS256,
        ));
        let svc = layer.layer(forbid_inner());

        let request_ext = Arc::new(RequestExt::new());
        let header = HeaderValue::from_static("graphql-ws, eyJhbGciOiJIUzI1NiJ9.x.y");
        let req = ws_request_with_protocol(header, Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("body is UTF-8");
        assert_eq!(
            body_text,
            "Invalid Sec-WebSocket-Protocol format: expected 'jolt-jwt, <token>'",
            "wrong-marker rejection mirrors WsTokenRejectReason::MissingJoltJwtPrefix::message()"
        );
        assert!(request_ext.is_finished());
    }

    #[tokio::test]
    async fn invalid_signature_token_short_circuits_with_401() {
        // Token signed with a DIFFERENT secret than the validator carries
        // → the decode-side InvalidSignature body surfaces. Confirms the
        // layer composes the 075 extractor with the 072 decoder, not just
        // the format check alone.
        let claims = JwtClaims {
            sub: "alice".to_owned(),
            exp: now_secs() + 3600,
            iat: None,
            extra: serde_json::Map::new(),
        };
        let token = sign_hs256(b"signed-with-this-secret", &claims);

        let layer = AuthWsJwtLayer::new(JwtConfig::new(
            b"verify-with-DIFFERENT-secret".to_vec(),
            Algorithm::HS256,
        ));
        let svc = layer.layer(forbid_inner());

        let request_ext = Arc::new(RequestExt::new());
        let header = HeaderValue::from_str(&format!("{JOLT_JWT_SUBPROTOCOL}, {token}")).unwrap();
        let req = ws_request_with_protocol(header, Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body_bytes = to_bytes(resp.into_body(), u32::MAX as usize).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).expect("body is UTF-8");
        assert_eq!(body_text, "Invalid token signature");
        assert!(request_ext.is_finished());
    }

    #[tokio::test]
    async fn valid_token_allows_upgrade_and_stashes_extension_keys() {
        // Mirror image of the rejection-path test: WS connect with a valid
        // token → the inner service runs (i.e. the upgrade is allowed to
        // proceed in real use), and BOTH WsJwtToken and JwtClaims land in
        // request extensions for 077's WS handler to read.
        let secret = b"jolt-rs-076-auth-ws-jwt-test-secret";
        let minted_sub = "user-076-valid";
        let claims = JwtClaims {
            sub: minted_sub.to_owned(),
            exp: now_secs() + 3600,
            iat: Some(now_secs()),
            extra: serde_json::Map::new(),
        };
        let token = sign_hs256(secret, &claims);

        let layer = AuthWsJwtLayer::new(JwtConfig::new(secret.to_vec(), Algorithm::HS256));

        // Side-channel capture mirrors the auth_jwt test module's pattern:
        // keep the assertion focused on the extension-key contract rather
        // than on serialization-through-HTTP.
        let captured: Arc<std::sync::Mutex<Option<(WsJwtToken, JwtClaims)>>> =
            Arc::new(std::sync::Mutex::new(None));
        let captured_for_inner = Arc::clone(&captured);
        let inner = tower::service_fn(move |req: AxumRequest| {
            let captured = Arc::clone(&captured_for_inner);
            async move {
                let token = req
                    .extensions()
                    .get::<WsJwtToken>()
                    .expect(
                        "AuthWsJwtLayer must stash WsJwtToken into extensions on a valid token",
                    )
                    .clone();
                let claims = req
                    .extensions()
                    .get::<JwtClaims>()
                    .expect(
                        "AuthWsJwtLayer must stash JwtClaims into extensions on a valid token",
                    )
                    .clone();
                *captured.lock().unwrap() = Some((token, claims));
                Ok::<Response, Infallible>(
                    Response::builder()
                        .status(StatusCode::SWITCHING_PROTOCOLS)
                        .body(Body::empty())
                        .unwrap(),
                )
            }
        });
        let svc = layer.layer(inner);

        let request_ext = Arc::new(RequestExt::new());
        let header = HeaderValue::from_str(&format!("{JOLT_JWT_SUBPROTOCOL}, {token}")).unwrap();
        let req = ws_request_with_protocol(header, Arc::clone(&request_ext));

        let resp = svc.oneshot(req).await.unwrap();
        // The inner stand-in returns 101 Switching Protocols to model the
        // upgrade outcome. The layer didn't short-circuit, so the inner ran.
        assert_eq!(
            resp.status(),
            StatusCode::SWITCHING_PROTOCOLS,
            "valid WS token must allow the inner upgrade handler to run"
        );

        let observed = captured
            .lock()
            .unwrap()
            .clone()
            .expect("inner service must have observed both extension keys");
        let (observed_token, observed_claims) = observed;
        assert_eq!(
            observed_token.as_str(),
            token,
            "WsJwtToken in extensions must carry the verbatim token bytes"
        );
        assert_eq!(
            observed_claims.sub, minted_sub,
            "JwtClaims sub must round-trip through the extension key"
        );
        assert!(
            !request_ext.is_finished(),
            "happy path must NOT mark the request finished"
        );
    }

    #[tokio::test]
    async fn fresh_request_ext_is_injected_when_no_upstream_layer_set_one() {
        // Mirrors AuthJwtService's preserve-or-inject contract: even when no
        // upstream Arc<RequestExt> is present, the layer injects a fresh one
        // before flipping mark_finished. Without this, a malformed-header
        // request reaching AuthWsJwtLayer standalone would have no observable
        // handle on the finished latch.
        let layer = AuthWsJwtLayer::new(JwtConfig::new(
            b"jolt-rs-076-auth-ws-jwt-test-secret".to_vec(),
            Algorithm::HS256,
        ));
        let svc = layer.layer(forbid_inner());

        // No RequestExt, no WS subprotocol header — the missing-header
        // branch fires.
        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/ws")
            .body(Body::empty())
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "rejection must still produce 401 even without an upstream RequestExt"
        );
    }
}

mod early_termination {
    //! PRD-mandated verification for JOLT-RS-078: "Each middleware layer
    //! (auth, cors, body, query, log) checks
    //! `req.extensions().get::<RequestExt>()?.is_finished()` before
    //! proceeding. If finished, return current response unchanged."
    //!
    //! Each test pre-flips the `finished` latch on a caller-supplied
    //! [`Arc<RequestExt>`](crate::RequestExt), wires the layer with an inner
    //! stub that returns a unique 200 marker body, and asserts:
    //!   1. The layer delegated straight to inner (the stub's marker body is
    //!      surfaced verbatim — no 401/400/204/CORS-header decoration).
    //!   2. The layer's own work-side mutation did NOT fire (no extension
    //!      key inserted, no header injected, no body buffered).
    //!
    //! The contract is uniform across all six layers exposed at the
    //! `jolt_core` crate root: AuthBearerLayer, AuthJwtLayer, AuthWsJwtLayer,
    //! CorsLayer, ParseBodyLayer, ParseBodyStringLayer, and ParseQueryLayer.
    //! The future logging layer (phase15 069/070) will follow the same shape
    //! when it lands.
    //!
    //! Marker-body-and-header strategy: the inner stub returns
    //! `Response::new(Body::from("__marker__"))` plus a unique header value
    //! per test so a layer that *did* fire would produce a different shape
    //! (different status code, different body, additional headers). Reading
    //! the response back and asserting the marker round-trips proves the
    //! layer's own logic was bypassed.
    //!
    //! Each test also asserts the layer's normal work-side extension key
    //! (BearerToken / JwtClaims / QueryParams / parsed-T) is ABSENT from
    //! request extensions on the inner-side. Since `service_fn` consumes the
    //! request, the assertion happens INSIDE the inner stub via a side-channel
    //! `Arc<Mutex<...>>` that captures whether the work-side key was set.

    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::extract::Request as AxumRequest;
    use axum::http::header::{
        ACCESS_CONTROL_ALLOW_ORIGIN, AUTHORIZATION, CONTENT_TYPE, HeaderName,
        SEC_WEBSOCKET_PROTOCOL,
    };
    use axum::http::{HeaderValue, Method as HttpMethod, StatusCode};
    use axum::response::Response;
    use jolt_utils::jwt::JwtConfig;
    use jsonwebtoken::Algorithm;
    use tower::{Layer, ServiceBuilder, ServiceExt};

    use crate::auth_bearer::BearerToken;
    use crate::auth_websocket::WsJwtToken;
    use crate::parse_query::QueryParams;
    use crate::{
        AuthBearerLayer, AuthJwtLayer, AuthWsJwtLayer, CorsConfig, CorsLayer, Endpoint,
        EndpointFuture, EndpointRegistry, Method, ParseBodyLayer, ParseBodyStringLayer,
        ParseQueryLayer, Request, RequestExt, Router,
    };
    use jolt_utils::jwt::JwtClaims;

    const MARKER_BODY: &str = "__inner_marker__";
    static MARKER_HEADER: HeaderName = HeaderName::from_static("x-inner-marker");

    /// Build an inner stub service that:
    ///   - Returns 200 with a marker body and a marker header so a layer that
    ///     decorated/short-circuited would produce a different response.
    ///   - Captures (via a side-channel `Arc<Mutex<bool>>`) whether the
    ///     given work-side extension key `T` was present at inner-call time.
    ///     Used to assert the layer DID NOT do its own work (which is what
    ///     would normally insert that key).
    fn marker_inner_capturing<T>(
        captured: Arc<Mutex<bool>>,
    ) -> impl tower::Service<
        AxumRequest,
        Response = Response,
        Error = Infallible,
        Future = impl Send + 'static,
    > + Clone
           + Send
           + 'static
    where
        T: Clone + Send + Sync + 'static,
    {
        tower::service_fn(move |req: AxumRequest| {
            let captured = Arc::clone(&captured);
            async move {
                let saw_extension = req.extensions().get::<T>().is_some();
                *captured
                    .lock()
                    .expect("inner-stub capture mutex poisoned") = saw_extension;
                let mut resp = Response::new(Body::from(MARKER_BODY));
                resp.headers_mut()
                    .insert(MARKER_HEADER.clone(), HeaderValue::from_static("present"));
                Ok::<Response, Infallible>(resp)
            }
        })
    }

    /// Inner stub for layers whose work-side key is none we want to assert
    /// against (e.g. CorsLayer doesn't insert anything; just confirms the
    /// marker round-trips). Returns 200 with the marker body + header.
    fn marker_inner_only() -> impl tower::Service<
        AxumRequest,
        Response = Response,
        Error = Infallible,
        Future = impl Send + 'static,
    > + Clone
           + Send
           + 'static {
        tower::service_fn(|_req: AxumRequest| async move {
            let mut resp = Response::new(Body::from(MARKER_BODY));
            resp.headers_mut()
                .insert(MARKER_HEADER.clone(), HeaderValue::from_static("present"));
            Ok::<Response, Infallible>(resp)
        })
    }

    async fn read_body(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("response body should buffer cleanly in tests");
        String::from_utf8(bytes.to_vec()).expect("marker body must be UTF-8")
    }

    fn pre_finished_request_ext() -> Arc<RequestExt> {
        let ext = Arc::new(RequestExt::new());
        ext.mark_finished();
        ext
    }

    #[tokio::test]
    async fn auth_bearer_skips_when_finished() {
        // Pre-flipped finished latch. The request carries an EXPLICITLY
        // malformed Authorization header that AuthBearerLayer would normally
        // reject as 401 — a regression that ignored the early-termination
        // check would surface that 401 instead of the inner's 200 marker.
        let captured: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let svc =
            AuthBearerLayer::new().layer(marker_inner_capturing::<BearerToken>(Arc::clone(&captured)));

        let ext = pre_finished_request_ext();
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/x")
            // Malformed header: NO 'Bearer ' scheme prefix. AuthBearerLayer's
            // normal path returns 401 with "Invalid Authorization header
            // format: expected 'Bearer <token>'".
            .header(AUTHORIZATION, "NotABearerHeader")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "AuthBearerLayer must skip auth check when RequestExt is already finished"
        );
        assert_eq!(
            resp.headers()
                .get(&MARKER_HEADER)
                .and_then(|v| v.to_str().ok()),
            Some("present"),
            "inner stub's marker header must round-trip on the skip path"
        );
        assert_eq!(read_body(resp).await, MARKER_BODY);
        assert!(
            !*captured.lock().unwrap(),
            "AuthBearerLayer must NOT insert BearerToken when skipping (auth check did not run)"
        );
    }

    #[tokio::test]
    async fn auth_jwt_skips_when_finished() {
        // Pre-flipped finished latch. AuthJwtLayer would normally reject a
        // request lacking a BearerToken extension as 401 (MissingBearerToken).
        // The early-termination check must bypass that decode path.
        let captured: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let layer = AuthJwtLayer::new(JwtConfig::new(b"secret".to_vec(), Algorithm::HS256));
        let svc = layer.layer(marker_inner_capturing::<JwtClaims>(Arc::clone(&captured)));

        let ext = pre_finished_request_ext();
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/x")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(read_body(resp).await, MARKER_BODY);
        assert!(
            !*captured.lock().unwrap(),
            "AuthJwtLayer must NOT insert JwtClaims when skipping"
        );
    }

    #[tokio::test]
    async fn auth_ws_jwt_skips_when_finished() {
        // Pre-flipped finished latch. AuthWsJwtLayer would normally reject a
        // request lacking the Sec-WebSocket-Protocol header as 401
        // (MissingHeader). The early-termination check must bypass.
        let captured: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let layer = AuthWsJwtLayer::new(JwtConfig::new(b"secret".to_vec(), Algorithm::HS256));
        let svc = layer.layer(marker_inner_capturing::<WsJwtToken>(Arc::clone(&captured)));

        let ext = pre_finished_request_ext();
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/ws")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(read_body(resp).await, MARKER_BODY);
        assert!(
            !*captured.lock().unwrap(),
            "AuthWsJwtLayer must NOT insert WsJwtToken when skipping"
        );
    }

    #[tokio::test]
    async fn cors_skips_when_finished_on_options() {
        // Pre-flipped finished latch on an OPTIONS request. The layer's
        // normal OPTIONS path produces a 204 preflight; the skip-when-finished
        // path must instead pass through to inner (which returns the 200
        // marker). Also confirms the layer does NOT inject CORS headers when
        // skipping.
        let config = CorsConfig {
            allow_origins: vec!["*".to_string()],
            ..Default::default()
        };
        let layer = CorsLayer::new(config);
        let svc = layer.layer(marker_inner_only());

        let ext = pre_finished_request_ext();
        let mut req = AxumRequest::builder()
            .method(HttpMethod::OPTIONS)
            .uri("/api/test")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "CorsLayer must skip preflight branch when RequestExt is finished — got the inner 200, not the 204 preflight"
        );
        assert!(
            resp.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
            "CorsLayer must NOT inject CORS headers on the skip path"
        );
        assert_eq!(read_body(resp).await, MARKER_BODY);
    }

    #[tokio::test]
    async fn cors_skips_when_finished_on_non_options() {
        // Pre-flipped finished latch on a GET. The layer's normal non-OPTIONS
        // path delegates to inner and INJECTS CORS headers on the response;
        // the skip path must NOT inject those headers.
        let config = CorsConfig {
            allow_origins: vec!["*".to_string()],
            expose_headers: vec!["x-something".to_string()],
            ..Default::default()
        };
        let layer = CorsLayer::new(config);
        let svc = layer.layer(marker_inner_only());

        let ext = pre_finished_request_ext();
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/test")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            resp.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
            "CorsLayer must NOT inject Allow-Origin on the skip path"
        );
        assert_eq!(read_body(resp).await, MARKER_BODY);
    }

    #[tokio::test]
    async fn parse_body_skips_when_finished() {
        // Pre-flipped finished latch on a request whose body is INVALID JSON.
        // The layer's normal path would return 400 ("Invalid JSON: ..."); the
        // early-termination skip path must delegate to inner and produce the
        // marker 200.
        let captured: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let layer = ParseBodyLayer::<serde_json::Value>::new();
        let svc =
            layer.layer(marker_inner_capturing::<serde_json::Value>(Arc::clone(&captured)));

        let ext = pre_finished_request_ext();
        let mut req = AxumRequest::builder()
            .method(HttpMethod::POST)
            .uri("/x")
            .header(CONTENT_TYPE, "application/json")
            // Deliberately invalid JSON so the normal path's 400 fires if the
            // skip check is broken.
            .body(Body::from("this is not json"))
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "ParseBodyLayer must skip body parse when finished — got the inner 200, not the 400"
        );
        assert_eq!(read_body(resp).await, MARKER_BODY);
        assert!(
            !*captured.lock().unwrap(),
            "ParseBodyLayer must NOT insert parsed-T extension on the skip path"
        );
    }

    #[tokio::test]
    async fn parse_body_string_skips_when_finished() {
        // Pre-flipped finished latch on a request whose body is INVALID UTF-8.
        // The layer's normal path would return 400 ("Invalid UTF-8: ..."); the
        // skip path must delegate to inner.
        let captured: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let layer = ParseBodyStringLayer::new();
        let svc = layer.layer(marker_inner_capturing::<String>(Arc::clone(&captured)));

        let ext = pre_finished_request_ext();
        // 0xFF is not valid UTF-8. Without the skip, parse_body_string returns
        // 400; with the skip, the inner stub's 200 surfaces.
        let mut req = AxumRequest::builder()
            .method(HttpMethod::POST)
            .uri("/x")
            .body(Body::from(vec![0xFFu8, 0xFE, 0xFD]))
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(read_body(resp).await, MARKER_BODY);
        assert!(
            !*captured.lock().unwrap(),
            "ParseBodyStringLayer must NOT insert String extension on the skip path"
        );
    }

    #[tokio::test]
    async fn parse_query_skips_when_finished() {
        // Pre-flipped finished latch. ParseQueryLayer normally inserts a
        // QueryParams extension key unconditionally; the skip path must NOT
        // insert it. (The layer doesn't fail, so this slice of the contract
        // is purely "the layer's own work-side mutation did not fire" rather
        // than "the layer didn't surface its own error".)
        let captured: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let svc = ParseQueryLayer::new()
            .layer(marker_inner_capturing::<QueryParams>(Arc::clone(&captured)));

        let ext = pre_finished_request_ext();
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/x?foo=bar&baz=qux")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(read_body(resp).await, MARKER_BODY);
        assert!(
            !*captured.lock().unwrap(),
            "ParseQueryLayer must NOT insert QueryParams when skipping"
        );
    }

    #[tokio::test]
    async fn auth_bearer_does_not_skip_when_not_finished() {
        // Negative half: with an Arc<RequestExt> present but `finished == false`,
        // the layer's normal logic must run (here, fail with 401 on the
        // malformed Authorization header). Confirms the skip predicate is
        // gated on the latch state and not triggered by mere presence of an
        // Arc<RequestExt>.
        let svc = AuthBearerLayer::new().layer(marker_inner_only());

        let ext = Arc::new(RequestExt::new()); // NOT finished
        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/x")
            .header(AUTHORIZATION, "NotABearerHeader")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "AuthBearerLayer's skip path must be gated on finished == true; a non-finished latch must NOT skip"
        );
        assert!(
            ext.is_finished(),
            "AuthBearerLayer must flip the latch on its rejection path (which is what the skip check observes for downstream layers)"
        );
    }

    #[tokio::test]
    async fn skip_path_is_observable_to_outer_layer() {
        // End-to-end skip-propagation test: stack ParseBodyLayer (outer) →
        // AuthBearerLayer (inner of parse) → marker stub. Pre-finish the
        // RequestExt; the OUTER ParseBodyLayer should skip its body parse,
        // delegate to AuthBearerLayer which ALSO skips its auth check, and
        // the inner stub's 200 marker surfaces unchanged. Without the skip
        // contract, ParseBodyLayer would buffer + reject the (invalid) body
        // OR AuthBearerLayer would reject the (missing) header — either way
        // the marker would not round-trip.
        let captured: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let inner = marker_inner_capturing::<BearerToken>(Arc::clone(&captured));
        let auth = AuthBearerLayer::new().layer(inner);
        let stack = ParseBodyLayer::<serde_json::Value>::new().layer(auth);

        let ext = pre_finished_request_ext();
        let mut req = AxumRequest::builder()
            .method(HttpMethod::POST)
            .uri("/x")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from("not json at all"))
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = stack.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "stacked layers must all skip; inner stub's 200 must surface verbatim"
        );
        assert_eq!(read_body(resp).await, MARKER_BODY);
        assert!(
            !*captured.lock().unwrap(),
            "neither layer must run its work-side path on the skip propagation"
        );
    }

    #[tokio::test]
    async fn skip_path_works_without_any_request_ext() {
        // Negative half for the skip predicate: a request with NO
        // Arc<RequestExt> in extensions must NOT skip (the predicate's
        // get::<Arc<RequestExt>>() lookup returns None, so the layer falls
        // through to its normal path). Wire AuthBearerLayer with a malformed
        // header; the layer must run its rejection branch (401) rather than
        // delegating to inner.
        let svc = AuthBearerLayer::new().layer(marker_inner_only());

        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/x")
            .header(AUTHORIZATION, "NotABearerHeader")
            .body(Body::empty())
            .unwrap();
        // Deliberately NO request_ext insertion here.

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "no RequestExt in extensions → predicate is None → fall through to normal logic (401 on malformed header)"
        );
    }

    // Sec-WebSocket-Protocol header reference kept in scope so the
    // auth_ws_jwt skip test can mirror the helper-based shape used by other
    // suite members. The test above (auth_ws_jwt_skips_when_finished) does
    // not actually emit this header — that is the point: a request without
    // it would normally be rejected as MissingHeader; the skip path bypasses
    // that rejection.
    #[allow(dead_code)]
    fn _ws_protocol_header_ref() -> HeaderName {
        SEC_WEBSOCKET_PROTOCOL
    }

    // ---------------------------------------------------------------------------
    // JOLT-RS-081: end-to-end early-termination integration tests
    //
    // These tests exercise the full dispatch chain (middleware → Router →
    // handler) and verify that when a layer marks the request finished, the
    // stashed response propagates correctly and the endpoint handler is never
    // invoked. They complement the JOLT-RS-078 layer-skip tests (above) by
    // wiring real Router + EndpointRegistry rather than stub inner services.
    //
    // Four scenarios per the PRD:
    //   1. single-layer finish  — one layer marks finished, response propagates
    //   2. multi-layer finish   — outer layers still decorate the propagated resp
    //   3. finish+before+handler — handler is skipped when finished (first check)
    //   4. finish+after+handler  — handler is NOT reached even on path+method match
    // ---------------------------------------------------------------------------

    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn single_layer_finish_propagates_response() {
        // Stack: AuthBearerLayer → Router (with a tracking ProtectedEndpoint).
        // Sending a GET /protected with NO Authorization header causes
        // AuthBearerLayer to mark_finished + return 401 directly (without
        // delegating to inner). Router's handler must never be invoked.
        let invoked = Arc::new(AtomicBool::new(false));

        struct ProtectedEndpoint {
            invoked: Arc<AtomicBool>,
        }

        impl Endpoint for ProtectedEndpoint {
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
                    Response::new(Body::from("should-never-run"))
                })
            }
        }

        let mut registry = EndpointRegistry::new();
        registry.register(ProtectedEndpoint {
            invoked: Arc::clone(&invoked),
        });
        let router = Router::new(registry);
        let svc = ServiceBuilder::new()
            .layer(AuthBearerLayer::new())
            .service(router);

        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/protected")
            .body(Body::empty())
            .unwrap();
        let resp = svc.oneshot(req).await.unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "AuthBearerLayer must reject missing Authorization with 401"
        );
        let body = read_body(resp).await;
        assert!(
            body == "Missing Authorization header",
            "AuthBearerLayer's 401 body must survive; got: {body}"
        );
        assert!(
            !invoked.load(Ordering::Relaxed),
            "handler must not be invoked when single layer finishes"
        );
    }

    #[tokio::test]
    async fn multi_layer_finish_propagates_through_outer_layers() {
        // Stack (outer → inner): CorsLayer → AuthBearerLayer → Router.
        // Request: GET /protected with Origin header but NO Authorization.
        // AuthBearerLayer rejects with 401 + marks_finished. CorsLayer, on
        // the response path, must still inject Access-Control-Allow-Origin
        // (JOLT-RS-057 non-OPTIONS contract) into the propagated 401.
        // Router's handler must never be invoked.
        use axum::http::header::{ORIGIN, WWW_AUTHENTICATE};

        let invoked = Arc::new(AtomicBool::new(false));

        struct ProtectedEndpoint {
            invoked: Arc<AtomicBool>,
        }

        impl Endpoint for ProtectedEndpoint {
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
                    Response::new(Body::from("should-never-run"))
                })
            }
        }

        let mut registry = EndpointRegistry::new();
        registry.register(ProtectedEndpoint {
            invoked: Arc::clone(&invoked),
        });
        let router = Router::new(registry);

        let cors = CorsLayer::new(CorsConfig {
            allow_origins: vec!["*".to_string()],
            ..Default::default()
        });
        let svc = ServiceBuilder::new()
            .layer(cors)
            .layer(AuthBearerLayer::new())
            .service(router);

        let req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/protected")
            .header(ORIGIN, "https://example.com")
            .body(Body::empty())
            .unwrap();
        let resp = svc.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers()
                .get(WWW_AUTHENTICATE)
                .and_then(|v| v.to_str().ok()),
            Some("Bearer"),
            "WWW-Authenticate must survive propagation through outer CorsLayer"
        );
        assert_eq!(
            resp.headers()
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|v| v.to_str().ok()),
            Some("*"),
            "CorsLayer must inject ACAO header on the propagated 401"
        );
        assert!(
            !invoked.load(Ordering::Relaxed),
            "handler must not be invoked in multi-layer finish"
        );
    }

    #[tokio::test]
    async fn finish_before_handler_skips_handler() {
        // Router's first finished check: when a caller-supplied
        // Arc<RequestExt> is already finished AND has a stashed response
        // BEFORE the registry walk, Router short-circuits and returns the
        // stashed response immediately. This test pins the early-out branch
        // (the check at the top of Router::call, before the registry walk).
        let invoked = Arc::new(AtomicBool::new(false));

        struct ProtectedEndpoint {
            invoked: Arc<AtomicBool>,
        }

        impl Endpoint for ProtectedEndpoint {
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
                    Response::new(Body::from("should-never-run"))
                })
            }
        }

        let mut registry = EndpointRegistry::new();
        registry.register(ProtectedEndpoint {
            invoked: Arc::clone(&invoked),
        });
        let router = Router::new(registry);

        let ext = Arc::new(RequestExt::new());
        ext.set_response(
            axum::response::Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::from("blocked by guard"))
                .unwrap(),
        );
        ext.mark_finished();

        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/protected")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(read_body(resp).await, "blocked by guard");
        assert!(
            !invoked.load(Ordering::Relaxed),
            "handler must not be invoked when finished flag is set before dispatch"
        );
    }

    #[tokio::test]
    async fn finish_after_path_match_still_skips_handler() {
        // Router's SECOND finished check (the guard immediately before
        // `endpoint.handler(jolt_req).await` in Router::call). Even when
        // the path + method match a registered endpoint, the handler must
        // NOT be invoked if RequestExt is finished.
        //
        // Distinct from `finish_before_handler_skips_handler` because THAT
        // test exercises the first check (before registry walk). This test
        // pins the second check independently: a regression that removed
        // only the second check while keeping the first would silently begin
        // dispatching requests that matched a route but whose RequestExt
        // was finished by an in-band middleware tier.
        let invoked = Arc::new(AtomicBool::new(false));

        struct ProtectedEndpoint {
            invoked: Arc<AtomicBool>,
        }

        impl Endpoint for ProtectedEndpoint {
            fn path(&self) -> &str {
                "/api/users"
            }
            fn method(&self) -> Method {
                Method::Get
            }
            fn handler(&self, _req: Request) -> EndpointFuture {
                let invoked = Arc::clone(&self.invoked);
                Box::pin(async move {
                    invoked.store(true, Ordering::Relaxed);
                    Response::new(Body::from("should-never-run"))
                })
            }
        }

        let mut registry = EndpointRegistry::new();
        registry.register(ProtectedEndpoint {
            invoked: Arc::clone(&invoked),
        });
        let router = Router::new(registry);

        let ext = Arc::new(RequestExt::new());
        ext.set_response(
            axum::response::Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::from("second-check block"))
                .unwrap(),
        );
        ext.mark_finished();

        let mut req = AxumRequest::builder()
            .method(HttpMethod::GET)
            .uri("/api/users")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(Arc::clone(&ext));

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "second finished check must block handler even on path+method match"
        );
        assert_eq!(read_body(resp).await, "second-check block");
        assert!(
            !invoked.load(Ordering::Relaxed),
            "handler must not be invoked when finished — second-check guard failed"
        );
    }
}

mod websocket {
    //! PRD-mandated verification for JOLT-RS-121: "Write WS trait unit test:
    //! mock WebSocketHandler, simulate open → message → close, verify callbacks
    //! fire in order."
    //!
    //! The test drives the full lifecycle sequence on a mock handler that
    //! records every callback invocation into a shared `Vec<&'static str>`
    //! and then asserts the recorded order matches the documented contract:
    //! `set_claims` → `on_open` → `on_ready` → `on_message` (0..N) →
    //! `on_close` → `on_shutdown`.
    //!
    //! `set_claims` is a sync method (not async) that fires before `on_open`;
    //! the rest are async. The mock records `set_claims` by wrapping the actual
    //! handler in a driver function that calls it manually, then drives the
    //! async lifecycle.

    use std::sync::{Arc, Mutex};

    use crate::{WebSocketHandler, WebSocketSender, WsMessage};

    /// A mock handler that records every lifecycle callback invocation in a
    /// shared `Vec<&'static str>`. Public fields let the test driver inspect
    /// state: `claims_set` confirms `set_claims` fired, `last_message` shows
    /// what message arrived, and `close_shutdown_order` records whether close
    /// arrived before shutdown (inner ordering within the same async tick).
    struct MockLifecycleHandler {
        calls: Arc<Mutex<Vec<&'static str>>>,
        claims_set: Arc<Mutex<bool>>,
        last_message: Arc<Mutex<Option<WsMessage>>>,
        close_shutdown_order: Arc<Mutex<Vec<&'static str>>>,
    }

    impl MockLifecycleHandler {
        fn new(
            calls: Arc<Mutex<Vec<&'static str>>>,
            claims_set: Arc<Mutex<bool>>,
            last_message: Arc<Mutex<Option<WsMessage>>>,
            close_shutdown_order: Arc<Mutex<Vec<&'static str>>>,
        ) -> Self {
            Self {
                calls,
                claims_set,
                last_message,
                close_shutdown_order,
            }
        }
    }

    impl WebSocketHandler for MockLifecycleHandler {
        fn set_claims(&mut self, _claims: jolt_utils::jwt::JwtClaims) {
            *self.claims_set.lock().unwrap() = true;
            self.calls.lock().unwrap().push("set_claims");
        }

        async fn on_open(&mut self, _sender: WebSocketSender) {
            self.calls.lock().unwrap().push("on_open");
        }

        async fn on_ready(&mut self, _sender: WebSocketSender) {
            self.calls.lock().unwrap().push("on_ready");
        }

        async fn on_message(&mut self, msg: WsMessage, _sender: WebSocketSender) {
            self.calls.lock().unwrap().push("on_message");
            *self.last_message.lock().unwrap() = Some(msg);
        }

        async fn on_close(&mut self) {
            self.calls.lock().unwrap().push("on_close");
            self.close_shutdown_order.lock().unwrap().push("on_close");
        }

        async fn on_shutdown(&mut self) {
            self.calls.lock().unwrap().push("on_shutdown");
            self.close_shutdown_order.lock().unwrap().push("on_shutdown");
        }
    }

    #[tokio::test]
    async fn lifecycle_callbacks_fire_in_documented_order_on_normal_connection() {
        // PRD-mandated verification: mock handler, simulate open → ready →
        // message → close → shutdown, verify callbacks fire in the order
        // set_claims → on_open → on_ready → on_message → on_close →
        // on_shutdown.
        let calls = Arc::new(Mutex::new(Vec::new()));
        let claims_set = Arc::new(Mutex::new(false));
        let last_message = Arc::new(Mutex::new(None));
        let close_shutdown_order = Arc::new(Mutex::new(Vec::new()));

        let mut handler = MockLifecycleHandler::new(
            Arc::clone(&calls),
            Arc::clone(&claims_set),
            Arc::clone(&last_message),
            Arc::clone(&close_shutdown_order),
        );

        let (sender, _rx) = WebSocketSender::channel();

        // 1. set_claims — sync, fires before any async callback.
        handler.set_claims(jolt_utils::jwt::JwtClaims {
            sub: "test-user".to_owned(),
            exp: 0,
            iat: None,
            extra: serde_json::Map::new(),
        });
        assert!(*claims_set.lock().unwrap(), "set_claims must have fired");

        // 2. on_open — fires once after claims, before messages.
        handler.on_open(sender.clone()).await;

        // 3. on_ready — fires after on_open, before message loop.
        handler.on_ready(sender.clone()).await;

        // 4. on_message — fires per inbound frame. Drive a few messages to
        //    pin the N-times contract.
        handler
            .on_message(WsMessage::Text("hello".to_string()), sender.clone())
            .await;
        handler
            .on_message(
                WsMessage::Binary(vec![1, 2, 3]),
                sender.clone(),
            )
            .await;

        {
            let last = last_message.lock().unwrap();
            assert!(
                matches!(last.as_ref(), Some(WsMessage::Binary(b)) if b == &vec![1, 2, 3]),
                "last_message must be the most recent on_message arg"
            );
        }

        // 5. on_close — fires once, no sender.
        handler.on_close().await;

        // 6. on_shutdown — fires after on_close.
        handler.on_shutdown().await;

        // Assert the full recorded order.
        let recorded = calls.lock().unwrap();
        let expected: &[&str] = &[
            "set_claims",
            "on_open",
            "on_ready",
            "on_message",
            "on_message",
            "on_close",
            "on_shutdown",
        ];
        assert_eq!(
            &recorded[..],
            expected,
            "lifecycle callbacks must fire in the documented order: \
             set_claims → on_open → on_ready → on_message (N times) → \
             on_close → on_shutdown"
        );

        // Pins that close fires before shutdown within the same handler
        // instance (not just the same recorded vec).
        let inner_order = close_shutdown_order.lock().unwrap();
        assert_eq!(
            &inner_order[..],
            &["on_close", "on_shutdown"],
            "on_close must fire before on_shutdown"
        );
    }

    #[tokio::test]
    async fn default_no_op_impls_run_to_completion_without_panicking() {
        // Pins that the default implementations (no-ops) can be called from
        // a unit struct handler without panicking. Each of the four async
        // methods (open / ready / message / close / shutdown) must complete.
        struct NoOpHandler;
        impl WebSocketHandler for NoOpHandler {}

        let (sender, _rx) = WebSocketSender::channel();
        let mut handler = NoOpHandler;

        handler.on_open(sender.clone()).await;
        handler.on_ready(sender.clone()).await;
        handler
            .on_message(WsMessage::Text("ignored".to_string()), sender.clone())
            .await;
        handler.on_close().await;
        handler.on_shutdown().await;
    }

    #[tokio::test]
    async fn partial_override_leaves_untouched_callbacks_at_noop_defaults() {
        // A handler that overrides only on_open and on_close must still
        // accept on_ready, on_message, and on_shutdown calls via the default
        // no-op impls. The recorded calls must show only the overridden
        // methods firing with content.
        let calls = Arc::new(Mutex::new(Vec::new()));

        struct PartialHandler {
            calls: Arc<Mutex<Vec<&'static str>>>,
        }
        impl WebSocketHandler for PartialHandler {
            async fn on_open(&mut self, _sender: WebSocketSender) {
                self.calls.lock().unwrap().push("on_open");
            }
            async fn on_close(&mut self) {
                self.calls.lock().unwrap().push("on_close");
            }
        }

        let (sender, _rx) = WebSocketSender::channel();
        let mut handler = PartialHandler {
            calls: Arc::clone(&calls),
        };

        handler.on_open(sender.clone()).await;
        handler.on_ready(sender.clone()).await; // default no-op
        handler
            .on_message(WsMessage::Text("ignored".to_string()), sender.clone())
            .await; // default no-op
        handler.on_close().await;
        handler.on_shutdown().await; // default no-op

        let recorded = calls.lock().unwrap();
        assert_eq!(
            &recorded[..],
            &["on_open", "on_close"],
            "only overridden methods fire; defaults are silent no-ops"
        );
    }

    #[tokio::test]
    async fn lifecycle_with_zero_messages_still_fires_open_ready_close_shutdown() {
        // The "0..N messages" contract means the read loop may never deliver
        // a frame (e.g. the client connects and immediately closes). The
        // handler must still receive on_open → on_ready → on_close →
        // on_shutdown without panicking.
        let calls = Arc::new(Mutex::new(Vec::new()));

        struct ZeroMessageHandler {
            calls: Arc<Mutex<Vec<&'static str>>>,
        }
        impl WebSocketHandler for ZeroMessageHandler {
            async fn on_open(&mut self, _sender: WebSocketSender) {
                self.calls.lock().unwrap().push("on_open");
            }
            async fn on_ready(&mut self, _sender: WebSocketSender) {
                self.calls.lock().unwrap().push("on_ready");
            }
            async fn on_close(&mut self) {
                self.calls.lock().unwrap().push("on_close");
            }
            async fn on_shutdown(&mut self) {
                self.calls.lock().unwrap().push("on_shutdown");
            }
        }

        let (sender, _rx) = WebSocketSender::channel();
        let mut handler = ZeroMessageHandler {
            calls: Arc::clone(&calls),
        };

        handler.on_open(sender.clone()).await;
        handler.on_ready(sender.clone()).await;
        // No on_message calls.
        handler.on_close().await;
        handler.on_shutdown().await;

        let recorded = calls.lock().unwrap();
        assert_eq!(
            &recorded[..],
            &["on_open", "on_ready", "on_close", "on_shutdown"],
            "zero-message lifecycle must still fire open → ready → close → shutdown"
        );
    }
}
