use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Json<T>(pub T);

impl<T> Deref for Json<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Json<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonArray<T>(pub Vec<T>);

impl<T> Deref for JsonArray<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for JsonArray<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct User {
        name: String,
        age: u32,
    }

    #[test]
    fn json_newtype_serializes_identically_to_inner() {
        let user = User {
            name: "Alice".into(),
            age: 30,
        };
        let json_user = Json(User {
            name: "Alice".into(),
            age: 30,
        });

        let user_json = serde_json::to_string(&user).unwrap();
        let json_user_json = serde_json::to_string(&json_user).unwrap();
        assert_eq!(user_json, json_user_json);
    }

    #[test]
    fn json_newtype_deserializes_identically_to_inner() {
        let raw = r#"{"name":"Alice","age":30}"#;

        let user: User = serde_json::from_str(raw).unwrap();
        let json_user: Json<User> = serde_json::from_str(raw).unwrap();

        assert_eq!(user, *json_user);
        assert_eq!(user, json_user.0);
    }

    #[test]
    fn deref_accesses_inner() {
        let json = Json(42i32);
        assert_eq!(*json, 42);
    }

    #[test]
    fn deref_mut_allows_mutation() {
        let mut json = Json(vec![1, 2, 3]);
        json.push(4);
        assert_eq!(*json, vec![1, 2, 3, 4]);
    }

    #[test]
    fn jsonarray_deserializes_from_json_array() {
        let raw = r#"[1, 2, 3, 4, 5]"#;
        let arr: JsonArray<i32> = serde_json::from_str(raw).unwrap();
        assert_eq!(*arr, vec![1, 2, 3, 4, 5]);
        assert_eq!(arr.len(), 5);
    }

    #[test]
    fn jsonarray_serializes_to_json_array() {
        let arr = JsonArray(vec!["a".to_string(), "b".to_string()]);
        let json = serde_json::to_string(&arr).unwrap();
        assert_eq!(json, r#"["a","b"]"#);
    }

    #[test]
    fn jsonarray_deref_and_deref_mut() {
        let mut arr = JsonArray(vec![10, 20]);
        assert_eq!(*arr, vec![10, 20]);
        assert_eq!(arr[0], 10);

        arr.push(30);
        assert_eq!(*arr, vec![10, 20, 30]);

        arr[0] = 99;
        assert_eq!(*arr, vec![99, 20, 30]);
    }

    #[test]
    fn jsonarray_empty_deserialization() {
        let raw = r#"[]"#;
        let arr: JsonArray<i32> = serde_json::from_str(raw).unwrap();
        assert!(arr.is_empty());
    }

    #[test]
    fn jsonarray_round_trip() {
        let original = JsonArray(vec![Json(42), Json(99)]);
        let json = serde_json::to_string(&original).unwrap();
        let restored: JsonArray<Json<i32>> = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }
}
