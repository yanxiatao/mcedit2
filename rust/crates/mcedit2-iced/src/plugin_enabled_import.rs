//! 旧 PluginRef.filename 偏好及其默认启用语义，映射当前真实 Python 文件或包，不导入代码。
use super::*;

pub(crate) struct Plan {
    root: PathBuf,
    changes: BTreeMap<String, bool>,
    missing: Vec<String>,
}
fn entry<'a>(
    catalog: &'a Catalog,
    name: &str,
) -> Option<&'a mcedit2_plugins::catalog::CatalogEntry> {
    catalog.entries.iter().find(|entry| {
        entry.source.kind == PluginKind::Python
            && entry.source.path.file_name().and_then(|name| name.to_str()) == Some(name)
    })
}
pub(crate) fn target_count(app: &App) -> usize {
    app.plugins.catalog.as_ref().map_or(0, |catalog| {
        catalog
            .entries
            .iter()
            .filter(|entry| entry.source.kind == PluginKind::Python)
            .count()
    })
}
pub(crate) fn descriptions(app: &App, values: &BTreeMap<String, bool>) -> Vec<String> {
    let mut rows = Vec::new();
    if let Some(catalog) = &app.plugins.catalog {
        for entry in catalog
            .entries
            .iter()
            .filter(|entry| entry.source.kind == PluginKind::Python)
        {
            let name = entry
                .source
                .path
                .file_name()
                .expect("已扫描来源是文件或包")
                .to_string_lossy();
            let enabled = values.get(name.as_ref()).copied().unwrap_or(true);
            rows.push(format!(
                "{name}：{}{}",
                app.language.pick(
                    if enabled { "启用" } else { "禁用" },
                    if enabled { "Enabled" } else { "Disabled" }
                ),
                if values.contains_key(name.as_ref()) {
                    ""
                } else {
                    app.language.pick("（原版默认）", " (legacy default)")
                }
            ));
        }
    }
    for name in values.keys().filter(|name| {
        app.plugins
            .catalog
            .as_ref()
            .is_none_or(|catalog| entry(catalog, name).is_none())
    }) {
        rows.push(format!(
            "{name}：{}",
            app.language.pick(
                "当前目录没有同名 Python 插件，不导入",
                "No Python plugin with this filename in the current catalog; skipped"
            )
        ));
    }
    rows
}
pub(crate) fn prepare(app: &App, values: &BTreeMap<String, bool>) -> Result<Plan, String> {
    let catalog = app
        .plugins
        .catalog
        .as_ref()
        .ok_or("请先在插件页扫描要导入状态的实际目录")?;
    let mut changes = BTreeMap::new();
    let mut missing = Vec::new();
    for source in catalog
        .entries
        .iter()
        .filter(|entry| entry.source.kind == PluginKind::Python)
    {
        let name = source
            .source
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("插件文件名无法按旧 UTF-8 文本键匹配")?;
        // 原 PluginRef.enabled 明确使用 enabledPlugins.get(self.filename, True)。
        changes.insert(
            source.source.id.clone(),
            values.get(name).copied().unwrap_or(true),
        );
    }
    missing.extend(
        values
            .keys()
            .filter(|name| entry(catalog, name).is_none())
            .cloned(),
    );
    if changes.is_empty() {
        return Err("当前目录没有实际 Python 插件，未导入启用状态".into());
    }
    Ok(Plan {
        root: catalog.root.clone(),
        changes,
        missing,
    })
}
pub(crate) fn apply(app: &mut App, plan: Plan) -> Result<(Task<Message>, String), String> {
    if app
        .plugins
        .catalog
        .as_ref()
        .is_none_or(|catalog| catalog.root != plan.root)
    {
        return Err("目标插件目录已改变，请重新核对旧状态导入".into());
    }
    let task = super::apply_enabled(app, &plan.changes)?;
    let mut report = format!(
        "插件启用状态已按真实文件合并 {} 项，未找到 {} 项。",
        plan.changes.len(),
        plan.missing.len()
    );
    if !plan.missing.is_empty() {
        report.push_str(&format!(" 未导入：{}", plan.missing.join("、")));
    }
    Ok((task, report))
}
