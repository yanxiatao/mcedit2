//! 资源语言表决定显示名称，原始方块 ID / 属性始终独立保留。
use crate::{language::Language, resource_ids};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub(crate) struct Names {
    tables: BTreeMap<String, BTreeMap<String, Option<String>>>,
    bytes: usize,
}
pub(crate) fn locale_asset(path: &str) -> Option<(String, bool)> {
    let mut p = path.split('/');
    if p.next()? != "assets" || !resource_ids::namespace(p.next()?) || p.next()? != "lang" {
        return None;
    }
    let file = p.next()?;
    if p.next().is_some() {
        return None;
    }
    let (locale, json) = file
        .strip_suffix(".json")
        .map(|l| (l, true))
        .or_else(|| file.strip_suffix(".lang").map(|l| (l, false)))?;
    let locale = locale.to_ascii_lowercase();
    matches!(locale.as_str(), "zh_cn" | "en_us").then_some((locale, json))
}
impl Names {
    pub(crate) fn translation(&self, key: &str, language: Language) -> Option<&str> {
        let locale = match language {
            Language::Chinese => "zh_cn",
            Language::English => "en_us",
        };
        self.tables
            .get(locale)
            .and_then(|table| table.get(key))
            .and_then(Option::as_deref)
            .or_else(|| {
                self.tables
                    .get("en_us")
                    .and_then(|table| table.get(key))
                    .and_then(Option::as_deref)
            })
    }
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
    pub(crate) fn insert(&mut self, path: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        let (locale, json) = locale_asset(path).ok_or("语言资源路径无效")?;
        let text = std::str::from_utf8(bytes)
            .map_err(|e| format!("语言文件不是 UTF-8：{e}"))?
            .trim_start_matches('\u{feff}');
        let mut diagnostics = Vec::new();
        let values: BTreeMap<String, Option<String>> = if json {
            let values: BTreeMap<String, serde_json::Value> =
                serde_json::from_str(text).map_err(|e| format!("语言 JSON 无效：{e}"))?;
            values
                .into_iter()
                .map(|(key, value)| {
                    let text = match plain(&value, 0) {
                        Ok(text) => Some(text),
                        Err(error) => {
                            diagnostics.push(format!("键 {key} 无法作为静态名称解析：{error}"));
                            None
                        }
                    };
                    (key, text)
                })
                .collect()
        } else {
            let mut values = BTreeMap::new();
            for (line_no, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let (key, value) = line
                    .split_once('=')
                    .ok_or_else(|| format!("语言文件第 {} 行缺少等号", line_no + 1))?;
                if key.trim().is_empty() {
                    return Err(format!("语言文件第 {} 行键为空", line_no + 1));
                }
                values.insert(key.trim().into(), Some(value.trim().into()));
            }
            values
        };
        let table = self.tables.entry(locale).or_default();
        for (key, value) in values {
            if let Some(old) = table.get(&key) {
                self.bytes -= key.len() + old.as_ref().map_or(0, String::len) + 96;
            }
            self.bytes = self
                .bytes
                .checked_add(key.len() + value.as_ref().map_or(0, String::len) + 96)
                .ok_or("语言表大小溢出")?;
            table.insert(key, value);
        }
        Ok(diagnostics)
    }
    pub(crate) fn name(&self, id: &str, language: Language) -> Option<&str> {
        let (namespace, path) = id.split_once(':')?;
        if !resource_ids::namespace(namespace) || !resource_ids::path(path) {
            return None;
        }
        let table = self.tables.get(match language {
            Language::Chinese => "zh_cn",
            Language::English => "en_us",
        })?;
        let path = path.replace('/', ".");
        table
            .get(&format!("block.{namespace}.{path}"))
            .or_else(|| table.get(&format!("item.{namespace}.{path}")))
            .or_else(|| {
                (namespace == "minecraft")
                    .then(|| table.get(&format!("tile.{path}.name")))
                    .flatten()
            })
            .and_then(|name| name.as_deref())
            .filter(|name| !name.trim().is_empty())
    }
}

/// 富文本名称提取可直接确定的文字；动态索引 / 翻译参数需要运行时上下文。
fn plain(value: &serde_json::Value, depth: usize) -> Result<String, String> {
    if depth > 32 {
        return Err("富文本嵌套超过 32 层".into());
    }
    match value {
        serde_json::Value::String(text) => Ok(text.clone()),
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) => Ok(value.to_string()),
        serde_json::Value::Array(parts) => {
            parts.iter().try_fold(String::new(), |mut text, part| {
                text.push_str(&plain(part, depth + 1)?);
                Ok(text)
            })
        }
        serde_json::Value::Object(object) => {
            let Some(text) = object.get("text").and_then(|v| v.as_str()) else {
                return Err("富文本需要尚未提供的动态参数或翻译上下文".into());
            };
            let mut text = text.to_owned();
            if let Some(extra) = object.get("extra") {
                let extra = extra.as_array().ok_or("富文本 extra 必须是数组")?;
                for part in extra {
                    text.push_str(&plain(part, depth + 1)?);
                }
            }
            Ok(text)
        }
        serde_json::Value::Null => Err("语言值为空".into()),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Selection {
    pub key: (usize, usize),
    pub position: (i32, i32, i32),
    pub state: mcedit2_world::sections::State,
}
impl crate::App {
    pub(crate) fn block_name_info(&self) -> Option<String> {
        let selected = self.selected_block.as_ref()?;
        if self.current_key() != Some(selected.key)
            || mcedit2_world::block_lookup::parse_xyz(&self.query_coordinates).ok()
                != Some(selected.position)
        {
            return None;
        }
        let bank = self.texture_bank.as_ref()?;
        let state = bank.render_state(&selected.state);
        let name = bank.block_name(&state.name, self.language)?;
        Some(format!(
            "{}{name}\n{}",
            self.language.pick("名称：", "Name: "),
            mcedit2_world::editing::render_state(&selected.state)
        ))
    }
}

#[cfg(test)]
#[path = "block_names_tests.rs"]
mod tests;
