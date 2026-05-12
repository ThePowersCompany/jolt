use joltr_core::{SameSite, SetCookie};

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
