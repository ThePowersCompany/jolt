/// Tri-state optional — mirrors JSON's "value / null / absent" semantics.
///
/// Used by `#[derive(PatchQuery)]` to distinguish between explicit `null`
/// (update the column to `NULL`) and field omission (skip the column).
///
/// # Serialization (JOLT-RS-163)
///
/// `Some(T)` delegates to `T::serialize`. `Null` and `NotProvided` both call
/// `serializer.serialize_none()`. The tri-state is distinguished at the
/// CONTAINING struct level via `#[serde(skip_serializing_if =
/// "Optional::is_not_provided")]`: `NotProvided` fields are skipped before
/// serialization; `Null` fields render as JSON `null`; `Some(T)` fields
/// render as the inner value.
///
/// # Deserialization (JOLT-RS-164)
///
/// Present non-null value → `Some(T)`, present null → `Null`.
/// `NotProvided` is never returned from deserialization — the containing
/// struct must use `#[serde(default)]` on `Optional<T>` fields so serde
/// invokes `Default::default()` (= `NotProvided`) when the field is absent
/// from the JSON.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Optional<T> {
    Some(T),
    Null,
    NotProvided,
}

impl<T> Optional<T> {
    pub fn is_some(&self) -> bool {
        matches!(self, Self::Some(_))
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub fn is_not_provided(&self) -> bool {
        matches!(self, Self::NotProvided)
    }

    pub fn as_ref(&self) -> Optional<&T> {
        match self {
            Self::Some(val) => Optional::Some(val),
            Self::Null => Optional::Null,
            Self::NotProvided => Optional::NotProvided,
        }
    }
}

impl<T> Default for Optional<T> {
    fn default() -> Self {
        Self::NotProvided
    }
}

impl<T: serde::Serialize> serde::Serialize for Optional<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Some(v) => v.serialize(serializer),
            Self::Null => serializer.serialize_none(),
            Self::NotProvided => serializer.serialize_none(),
        }
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for Optional<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Option::<T>::deserialize(deserializer)? {
            Some(val) => Ok(Optional::Some(val)),
            None => Ok(Optional::Null),
        }
    }
}
