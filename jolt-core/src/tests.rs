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
}
