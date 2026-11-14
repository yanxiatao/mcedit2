//! Bilingual core UI labels. Switching language never mutates saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Language {
    English,
    Chinese,
}
impl Default for Language {
    fn default() -> Self {
        let locale = sys_locale::get_locale()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if locale.starts_with("zh") {
            Self::Chinese
        } else {
            Self::English
        }
    }
}
impl Language {
    pub(crate) fn pick(self, zh: &'static str, en: &'static str) -> &'static str {
        match self {
            Self::Chinese => zh,
            Self::English => en,
        }
    }
    pub(crate) fn toggle(self) -> Self {
        match self {
            Self::Chinese => Self::English,
            Self::English => Self::Chinese,
        }
    }
}
