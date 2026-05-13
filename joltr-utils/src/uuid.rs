pub fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn uuid_v7() -> String {
    uuid::Uuid::now_v7().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_v4_is_valid_format() {
        let id = uuid_v4();
        let parsed = uuid::Uuid::parse_str(&id).expect("uuid_v4 output must be a valid UUID");
        assert_eq!(
            parsed.get_version_num(),
            4,
            "uuid_v4 must produce version 4 UUID"
        );
    }

    #[test]
    fn uuid_v7_is_valid_format() {
        let id = uuid_v7();
        let parsed = uuid::Uuid::parse_str(&id).expect("uuid_v7 output must be a valid UUID");
        assert_eq!(
            parsed.get_version_num(),
            7,
            "uuid_v7 must produce version 7 UUID"
        );
    }

    #[test]
    fn uuid_v4_produces_unique_values() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = uuid_v4();
            assert!(
                seen.insert(id),
                "uuid_v4 must produce unique values within 1000 generations"
            );
        }
    }

    #[test]
    fn uuid_v7_produces_unique_values() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = uuid_v7();
            assert!(
                seen.insert(id),
                "uuid_v7 must produce unique values within 1000 generations"
            );
        }
    }
}
