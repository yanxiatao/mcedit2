//! Bilingual core UI labels. Switching language never mutates saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_switch_changes_labels() {
        assert_eq!(Language::Chinese.pick("打开世界", "Open world"), "打开世界");
        assert_eq!(
            Language::English.pick("打开世界", "Open world"),
            "Open world"
        );
    }
}
