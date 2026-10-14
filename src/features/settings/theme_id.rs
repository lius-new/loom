//! Pure stable serialized theme identifiers.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
const IDS: &[&str] = &["aura-dark", "aura-light", "ember", "high-contrast"];

/// A built-in theme. Stored in `settings.json` as the theme's string id; an
/// unknown id loads as the default theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThemeId(usize); // index into IDS; 0 is the default

impl ThemeId {
    pub fn all() -> impl Iterator<Item = Self> {
        (0..IDS.len()).map(Self)
    }

    pub fn parse(id: &str) -> Option<Self> {
        IDS.iter().position(|name| *name == id).map(Self)
    }

    pub fn id(self) -> &'static str {
        IDS[self.0]
    }
    pub(crate) fn index(self) -> usize {
        self.0
    }
    pub(crate) fn from_index(index: usize) -> Self {
        Self(index)
    }
}

impl Serialize for ThemeId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

impl<'de> Deserialize<'de> for ThemeId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Ok(Self::parse(&id).unwrap_or_default())
    }
}
