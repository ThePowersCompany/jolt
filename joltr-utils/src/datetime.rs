use chrono::{DateTime, Utc};

pub fn parse_rfc3339(s: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc))
}

pub fn format_rfc3339(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parse_rfc3339_z_format() {
        let dt = parse_rfc3339("2024-01-15T10:30:00Z").expect("valid RFC 3339 with Z suffix");
        let expected = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();
        assert_eq!(dt, expected);
    }

    #[test]
    fn parse_rfc3339_offset_format() {
        let dt = parse_rfc3339("2024-01-15T10:30:00+00:00").expect("valid RFC 3339 with +00:00");
        let expected = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();
        assert_eq!(dt, expected);
    }

    #[test]
    fn parse_rfc3339_with_fractional_seconds() {
        let dt = parse_rfc3339("2024-01-15T10:30:00.500Z").expect("valid RFC 3339 with fractional seconds");
        let expected = Utc
            .with_ymd_and_hms(2024, 1, 15, 10, 30, 0)
            .unwrap()
            + chrono::Duration::milliseconds(500);
        assert_eq!(dt, expected);
    }

    #[test]
    fn parse_rfc3339_with_nonzero_offset() {
        let dt = parse_rfc3339("2024-01-15T10:30:00+05:00").expect("valid RFC 3339 with +05:00");
        let expected = Utc.with_ymd_and_hms(2024, 1, 15, 5, 30, 0).unwrap();
        assert_eq!(dt, expected);
    }

    #[test]
    fn parse_rfc3339_rejects_invalid_format() {
        let result = parse_rfc3339("2024-01-15 10:30:00");
        assert!(result.is_err(), "missing 'T' separator must be rejected");
    }

    #[test]
    fn parse_rfc3339_rejects_garbage() {
        let result = parse_rfc3339("not-a-date");
        assert!(result.is_err(), "garbage input must be rejected");
    }

    #[test]
    fn format_rfc3339_produces_rfc3339_output() {
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();
        let s = format_rfc3339(&dt);
        assert_eq!(s, "2024-01-15T10:30:00+00:00");
    }

    #[test]
    fn round_trip_parse_then_format_preserves_timestamp() {
        let original = "2024-01-15T10:30:00Z";
        let dt = parse_rfc3339(original).expect("valid RFC 3339 input");
        let formatted = format_rfc3339(&dt);
        let reparsed = parse_rfc3339(&formatted).expect("formatted output must itself be valid RFC 3339");
        assert_eq!(dt, reparsed, "round-trip parse→format→parse must preserve the instant");
    }

    #[test]
    fn round_trip_format_then_parse_preserves_timestamp() {
        let dt = Utc.with_ymd_and_hms(2024, 6, 30, 23, 59, 59).unwrap();
        let s = format_rfc3339(&dt);
        let reparsed = parse_rfc3339(&s).expect("formatted output must be valid RFC 3339");
        assert_eq!(dt, reparsed, "round-trip format→parse must preserve the instant");
    }

    #[test]
    fn format_rfc3339_includes_subsecond_precision() {
        let dt = Utc
            .with_ymd_and_hms(2024, 1, 15, 10, 30, 0)
            .unwrap()
            + chrono::Duration::milliseconds(123);
        let s = format_rfc3339(&dt);
        assert!(s.starts_with("2024-01-15T10:30:00.123"));
    }
}
