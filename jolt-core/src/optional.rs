/// Tri-state optional — mirrors JSON's "value / null / absent" semantics.
///
/// Used by `#[derive(PatchQuery)]` to distinguish between explicit `null`
/// (update the column to `NULL`) and field omission (skip the column).
///
/// Serialize + Deserialize are not implemented here — they belong to
/// JOLT-RS-163/164 in phase38. The three-variant shape is the load-bearing
/// part for phase27 PatchQuery codegen.
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
