//! Canonical, non-traversing Minecraft Java resource locations.
//!
//! Java identifiers can contain dots (for example mod.name:machine.port).
//! Namespaces and paths use different character sets. ZIP member names are
//! never interpreted as operating-system paths or passed to filesystem writes.

/// A Minecraft resource namespace, including names with periods.
pub(crate) fn namespace(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-' | b'.')
        })
}

/// Canonical resource path: slash-separated lowercase segments. A dot in
/// a filename is valid, but . and .. segments (including ZIP traversal),
/// backslashes, empty segments, absolute paths and upper-case are refused.
pub(crate) fn path(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'/' | b'_' | b'-' | b'.')
        })
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub(crate) fn qualified(value: &str, fallback_namespace: &str) -> Option<String> {
    let (ns, name) = value.split_once(':').unwrap_or((fallback_namespace, value));
    if !namespace(ns) || !path(name) {
        return None;
    }
    Some(format!("{ns}:{name}"))
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minecraft_resource_ids_allow_modded_dots_but_refuse_traversal() {
        assert!(namespace("create.addon"));
        assert!(namespace("example-mod_2"));
        assert!(path("machine/cable.port"));
        assert_eq!(
            qualified("create.addon:block/gearbox.v2", "minecraft"),
            Some("create.addon:block/gearbox.v2".into())
        );
        assert_eq!(
            qualified("block/log", "minecraft"),
            Some("minecraft:block/log".into())
        );
        for namespace_value in ["..", ".", "UPPER", "mod:name", "", "mod/name"] {
            assert!(!namespace(namespace_value), "{namespace_value}");
        }
        for bad in [
            "",
            "/root",
            "folder/",
            "folder//block",
            "a/./b",
            "a/../b",
            "../block",
            "folder\\block",
            "Upper/stone",
            "c:stone",
        ] {
            assert!(!path(bad), "{bad}");
        }
        assert_eq!(qualified("mod:../secret", "minecraft"), None);
        assert_eq!(qualified("mod:stone", ".."), Some("mod:stone".into()));
        assert_eq!(qualified("stone", ".."), None);
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Sources {
            minecraft_resource_ids_allow_modded_dots_but_refuse_traversal();
        }
    }
}
