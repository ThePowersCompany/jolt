/// Returns `true` if `s` resembles a valid email address.
///
/// Checks: contains exactly one `@`, non-empty local part, non-empty domain,
/// and the domain contains at least one `.`.
///
/// This is a structural check, not full RFC 5321/5322 validation. Use a
/// dedicated email-parser crate if you need quoted local-parts, comments,
/// IP-literal domains, or SMTP deliverability checks.
pub fn is_valid_email(s: &str) -> bool {
    let at_pos = match s.rfind('@') {
        Some(pos) => pos,
        None => return false,
    };

    let local = &s[..at_pos];
    let domain = &s[at_pos + 1..];

    if s.matches('@').count() != 1 {
        return false;
    }

    if local.is_empty() || domain.is_empty() {
        return false;
    }

    if !domain.contains('.') {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_emails_return_true() {
        assert!(is_valid_email("user@example.com"));
        assert!(is_valid_email("a@b.co"));
        assert!(is_valid_email("name@sub.domain.org"));
    }

    #[test]
    fn missing_at_returns_false() {
        assert!(!is_valid_email("userexample.com"));
    }

    #[test]
    fn empty_local_returns_false() {
        assert!(!is_valid_email("@example.com"));
    }

    #[test]
    fn empty_domain_returns_false() {
        assert!(!is_valid_email("user@"));
    }

    #[test]
    fn domain_without_dot_returns_false() {
        assert!(!is_valid_email("user@localhost"));
    }

    #[test]
    fn multiple_at_returns_false() {
        assert!(!is_valid_email("a@b@c.com"));
    }
}
