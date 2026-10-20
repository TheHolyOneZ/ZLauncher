use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Override {
    #[serde(default)]
    pub skip: bool,
    #[serde(default)]
    pub extra_pages: Vec<String>,
    #[serde(default)]
    pub manifests: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Overrides(HashMap<String, Override>);

impl Overrides {
    pub fn bundled() -> Self {
        Self::parse(include_str!("../overrides.toml")).expect("bundled overrides.toml is valid")
    }

    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        Ok(Self(toml::from_str(text)?))
    }

    pub fn get(&self, id: &str) -> Option<&Override> {
        self.0.get(id)
    }
}
