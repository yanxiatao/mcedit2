//! 独立结构的最近记录只保存已确认来源和页面，不保存编辑、候选或正在运行的工具。
use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegacySource {
    pub(super) version: SourceVersion,
    sha256: String,
}
impl LegacySource {
    pub(super) fn capture(document: &Document) -> Self {
        let content = document.content();
        Self {
            version: SourceVersion {
                data_version: content.data_version,
                release: content.source_release.clone(),
            },
            sha256: document.source_file().sha256.clone(),
        }
    }
    pub(super) fn verify(&self, path: &Path) -> Result<(), String> {
        mcedit2_world::clipboard::SourceFileProof {
            path: path.to_path_buf(),
            sha256: self.sha256.clone(),
        }
        .verify()
        .map_err(|error| {
            format!("最近记录的旧结构来源已改变，请在文件页重新选择版本后读取：{error}")
        })
    }
    pub(super) fn matches(&self, document: &Document) -> Result<(), String> {
        if document.source_file().sha256 != self.sha256 {
            return Err("旧结构在来源核对与读取之间已改变；尚未采用最近记录的版本解释".into());
        }
        Ok(())
    }
    pub(super) fn label(&self) -> String {
        match (&self.version.release, self.version.data_version) {
            (Some(release), _) => release.clone(),
            (None, Some(version)) => format!("DataVersion {version}"),
            (None, None) => "已确认无版本".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    pub format: Format,
    source: Option<LegacySource>,
    page: Page,
    view: crate::workspace_ui::View,
    axis: Axis,
}
impl Saved {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some(source) = &self.source {
            if self.format != Format::Legacy
                || source.sha256.len() != 64
                || !source.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err("最近结构的来源版本证明无效".into());
            }
        }
        Ok(())
    }
    fn capture(entry: &Entry) -> Option<Self> {
        let document = entry.document.as_ref()?;
        Some(Self {
            format: document.format(),
            source: (document.format() == Format::Legacy).then(|| LegacySource::capture(document)),
            page: entry.page,
            view: entry.scene.mode,
            axis: entry.axis,
        })
    }
    pub(super) fn restore(&self, entry: &mut Entry) {
        entry.recent_source = self.source.clone();
        entry.page = self.page;
        entry.scene.mode = self.view;
        entry.axis = self.axis;
    }
    pub(crate) fn label(&self, language: crate::language::Language) -> String {
        let mut label = format!("{} · {}", language.pick("结构", "Structure"), self.format);
        if let Some(source) = &self.source {
            label.push_str(" · ");
            match source.version.data_version {
                Some(version) => label.push_str(&format!("DataVersion {version}")),
                None => label.push_str(language.pick("已确认无版本", "Confirmed unversioned")),
            }
        }
        label
    }
}

pub(crate) fn capture(app: &App, id: u64) -> Option<(PathBuf, String, Saved)> {
    let entry = app.structures.files.get(&id)?;
    let saved = Saved::capture(entry)?;
    Some((
        entry.document.as_ref()?.path().to_path_buf(),
        entry.label(),
        saved,
    ))
}

/// 页面变化只更新仍在最近列表中的记录；移除或清空不会由下一次重绘重新添加。
pub(crate) fn remember(app: &mut App) {
    for recent in &mut app.desktop.saved.recent {
        if recent.structure.is_none() {
            continue;
        }
        if let Some(entry) = app.structures.files.values().find(|entry| {
            entry
                .document
                .as_ref()
                .is_some_and(|document| document.path() == recent.root)
        }) {
            recent.structure = Saved::capture(entry);
        }
    }
}
