use std::collections::HashMap;

use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, HeaderValue};
use joltr_core::{Cookie, Method, Request, Response, SameSite, SetCookie, StatusCode};

fn cookie(name: &str, value: &str) -> Cookie {
    Cookie {
        name: name.to_string(),
        value: value.to_string(),
    }
}

#[test]
fn set_cookie_new_renders_name_value() {
    let cookie = SetCookie::new("session", "abc123");

    assert_eq!(cookie.to_string(), "session=abc123");
    assert_eq!(cookie.to_header(), "session=abc123");
}

#[test]
fn set_cookie_builder_renders_attributes_in_stable_order() {
    let cookie = SetCookie::new("session", "abc123")
        .path("/")
        .domain("example.com")
        .secure()
        .http_only()
        .same_site(SameSite::Lax)
        .max_age(3600)
        .expires("Wed, 21 Oct 2015 07:28:00 GMT");

    assert_eq!(
        cookie.to_header(),
        "session=abc123; Path=/; Domain=example.com; Secure; HttpOnly; SameSite=Lax; Max-Age=3600; Expires=Wed, 21 Oct 2015 07:28:00 GMT"
    );
}

#[test]
fn set_cookie_same_site_variants_render_header_values() {
    assert_eq!(SameSite::Lax.to_string(), "Lax");
    assert_eq!(SameSite::Strict.to_string(), "Strict");
    assert_eq!(SameSite::None.to_string(), "None");
}

#[test]
fn set_cookie_parse_reads_supported_attributes() {
    let cookie = SetCookie::parse(
        "session=abc123; Path=/; Domain=example.com; Secure; HttpOnly; SameSite=Lax; Max-Age=3600; Expires=Wed, 21 Oct 2015 07:28:00 GMT",
    )
    .unwrap();

    assert_eq!(cookie.name, "session");
    assert_eq!(cookie.value, "abc123");
    assert_eq!(cookie.path.as_deref(), Some("/"));
    assert_eq!(cookie.domain.as_deref(), Some("example.com"));
    assert!(cookie.secure);
    assert!(cookie.http_only);
    assert_eq!(cookie.same_site, Some(SameSite::Lax));
    assert_eq!(cookie.max_age, Some(3600));
    assert_eq!(
        cookie.expires.as_deref(),
        Some("Wed, 21 Oct 2015 07:28:00 GMT")
    );
}

#[test]
fn set_cookie_parse_is_case_insensitive_for_attribute_names_and_same_site() {
    let cookie =
        SetCookie::parse("session=abc123; path=/admin; secure; httponly; samesite=none").unwrap();

    assert_eq!(cookie.path.as_deref(), Some("/admin"));
    assert!(cookie.secure);
    assert!(cookie.http_only);
    assert_eq!(cookie.same_site, Some(SameSite::None));
}

#[test]
fn set_cookie_parse_preserves_value_after_first_equals() {
    let cookie = SetCookie::parse("token=a=b=c; Path=/").unwrap();

    assert_eq!(cookie.name, "token");
    assert_eq!(cookie.value, "a=b=c");
}

#[test]
fn set_cookie_parse_rejects_invalid_required_parts() {
    assert!(SetCookie::parse("Secure").is_err());
    assert!(SetCookie::parse("session=abc; SameSite=Maybe").is_err());
    assert!(SetCookie::parse("session=abc; Max-Age=soon").is_err());
    assert!(SetCookie::parse("session=abc; Secure=true").is_err());
}

#[test]
fn cookie_parse_all_reads_semicolon_separated_name_value_pairs() {
    let cookies = Cookie::parse_all("sid=abc123; theme=dark; token=a=b=c");

    assert_eq!(
        cookies,
        vec![
            cookie("sid", "abc123"),
            cookie("theme", "dark"),
            cookie("token", "a=b=c")
        ]
    );
}

#[test]
fn cookie_parse_all_trims_pairs_and_skips_invalid_segments() {
    let cookies = Cookie::parse_all(" sid = abc123 ; Secure ; =missing ; empty= ; theme = dark ");

    assert_eq!(
        cookies,
        vec![
            cookie("sid", "abc123"),
            cookie("empty", ""),
            cookie("theme", "dark")
        ]
    );
}

#[test]
fn request_cookies_parse_cookie_header() {
    let mut headers = HeaderMap::new();
    headers.insert("Cookie", HeaderValue::from_static("sid=abc123; theme=dark"));
    let req = Request {
        method: Method::Get,
        path: "/".to_string(),
        headers,
        query_params: HashMap::new(),
        body: Vec::new(),
        cookies: Vec::new(),
        finished: false,
    };

    assert_eq!(
        req.cookies(),
        vec![cookie("sid", "abc123"), cookie("theme", "dark")]
    );
}

#[test]
fn response_set_cookie_appends_set_cookie_headers() {
    let response = Response::new(StatusCode::Ok, "ok")
        .set_cookie(SetCookie::new("sid", "abc123").path("/").http_only())
        .set_cookie(SetCookie::new("theme", "dark"));

    let values = response
        .headers
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(values, vec!["sid=abc123; Path=/; HttpOnly", "theme=dark"]);
}

#[test]
fn response_remove_cookie_appends_expiring_set_cookie_header() {
    let response = Response::new(StatusCode::Ok, "ok").remove_cookie("sid");

    let value = response.headers.get(SET_COOKIE).unwrap().to_str().unwrap();

    assert_eq!(
        value,
        "sid=; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT"
    );
}
