use std::collections::HashMap;
use std::sync::LazyLock;

static MIME_TABLE: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut m = HashMap::new();

    m.insert(".html", "text/html");
    m.insert(".htm", "text/html");
    m.insert(".css", "text/css");
    m.insert(".js", "application/javascript");
    m.insert(".mjs", "application/javascript");
    m.insert(".json", "application/json");
    m.insert(".png", "image/png");
    m.insert(".jpg", "image/jpeg");
    m.insert(".jpeg", "image/jpeg");
    m.insert(".gif", "image/gif");
    m.insert(".svg", "image/svg+xml");
    m.insert(".ico", "image/x-icon");
    m.insert(".webp", "image/webp");
    m.insert(".txt", "text/plain");
    m.insert(".xml", "application/xml");
    m.insert(".pdf", "application/pdf");
    m.insert(".zip", "application/zip");
    m.insert(".mp4", "video/mp4");
    m.insert(".webm", "video/webm");
    m.insert(".mp3", "audio/mpeg");
    m.insert(".wasm", "application/wasm");

    m
});

pub fn content_type_for_extension(ext: &str) -> Option<&'static str> {
    MIME_TABLE.get(ext).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_extension_returns_text_html() {
        assert_eq!(MIME_TABLE.get(".html").copied(), Some("text/html"));
    }

    #[test]
    fn css_extension_returns_text_css() {
        assert_eq!(MIME_TABLE.get(".css").copied(), Some("text/css"));
    }

    #[test]
    fn js_extension_returns_application_javascript() {
        assert_eq!(MIME_TABLE.get(".js").copied(), Some("application/javascript"));
    }

    #[test]
    fn json_extension_returns_application_json() {
        assert_eq!(MIME_TABLE.get(".json").copied(), Some("application/json"));
    }

    #[test]
    fn png_extension_returns_image_png() {
        assert_eq!(MIME_TABLE.get(".png").copied(), Some("image/png"));
    }

    #[test]
    fn unknown_extension_returns_none() {
        assert_eq!(content_type_for_extension(".xyz"), None);
    }

    #[test]
    fn content_type_for_extension_returns_correct_type() {
        assert_eq!(content_type_for_extension(".css"), Some("text/css"));
        assert_eq!(content_type_for_extension(".png"), Some("image/png"));
    }
}
