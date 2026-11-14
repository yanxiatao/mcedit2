//! 只读导入原 MCEdit2 的 Qt 4.8 INI；不实例化 Qt、不执行 QVariant 或 Python。
//! 来源：7c13c16^ 的 appsettings.py / util/settings.py / editorapp.py，及 Qt 4.8
//! https://github.com/qt/qt/blob/4.8/src/corelib/io/qsettings.cpp
//! https://github.com/qt/qt/blob/4.8/src/gui/kernel/qwidget.cpp
use super::{Geometry, Language};
use crate::workspace_ui::View;
use std::{collections::BTreeMap, io::Read, path::PathBuf};
#[path = "qt_layout.rs"]
pub(super) mod layout;

#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub path: PathBuf,
    pub recent: Option<Vec<PathBuf>>,
    pub language: Option<Language>,
    pub geometry: Option<Geometry>,
    pub dock_layout: Option<layout::Saved>,
    pub minimap_visibility: Option<bool>,
    pub floating_panels: Option<layout::LegacyFloats>,
    pub view: Option<View>,
    pub camera_perspective: Option<bool>,
    pub camera_distance: Option<crate::camera_distance::Saved>,
    pub world_browser: Option<crate::world_browser::Saved>,
    pub plugin_development: Option<bool>,
    pub plugin_enabled: Option<BTreeMap<String, bool>>,
    pub client_installations: Option<crate::client_installations::LegacySettings>,
    pub dialog_paths: Option<crate::dialog_paths::Saved>,
    pub structure_export: Option<crate::structure_exchange::export_options::LegacySettings>,
    pub nbt_search: Option<crate::nbt_search::LegacySettings>,
    pub brush: Option<crate::brush_stroke::LegacySettings>,
    pub selection: Option<crate::selection_preferences::LegacySettings>,
    pub layer_preferences: Option<crate::scene_layers::preferences::LegacySettings>,
    pub keys: BTreeMap<crate::keyboard_bindings::Command, String>,
    pub navigation: BTreeMap<crate::navigation_bindings::Command, String>,
    pub sticky: Option<bool>,
    pub notices: Vec<String>,
}
impl Plan {
    pub fn available(&self) -> bool {
        self.recent.is_some()
            || self.language.is_some()
            || self.geometry.is_some()
            || self.dock_layout.is_some()
            || self.minimap_visibility.is_some()
            || self.floating_panels.is_some()
            || self.view.is_some()
            || self.camera_perspective.is_some()
            || self.camera_distance.is_some()
            || self.world_browser.is_some()
            || self.plugin_development.is_some()
            || self.plugin_enabled.is_some()
            || self.client_installations.is_some()
            || self.dialog_paths.is_some()
            || self.structure_export.is_some()
            || self.nbt_search.is_some()
            || self.brush.is_some()
            || self.selection.is_some()
            || self.layer_preferences.is_some()
            || !self.keys.is_empty()
            || !self.navigation.is_empty()
            || self.sticky.is_some()
    }
}
#[derive(Default)]
pub(super) struct Import {
    pub busy: bool,
    pub plan: Option<Plan>,
    pub report: String,
    pub recent: bool,
    pub language: bool,
    pub geometry: bool,
    pub dock_layout: bool,
    pub layout_anchor: Option<usize>,
    pub layout_size: bool,
    pub minimap_visibility: bool,
    pub floating_panels: bool,
    pub view: bool,
    pub camera_projection: bool,
    pub camera_distance: bool,
    pub world_browser: bool,
    pub plugin_development: bool,
    pub plugin_enabled: bool,
    pub client_installations: bool,
    pub dialog_paths: bool,
    pub structure_export: bool,
    pub nbt_search: bool,
    pub brush: bool,
    pub selection: bool,
    pub layer_preferences: bool,
    pub keys: bool,
}
impl Import {
    pub fn receive(&mut self, plan: Plan) {
        self.recent = plan.recent.is_some();
        self.language = plan.language.is_some();
        self.geometry = false;
        self.dock_layout = false;
        self.layout_anchor = None;
        self.layout_size = false;
        self.minimap_visibility = plan.minimap_visibility.is_some();
        self.floating_panels = false;
        self.view = plan.view.is_some();
        self.camera_projection = plan.camera_perspective.is_some();
        self.camera_distance = plan.camera_distance.is_some();
        self.world_browser = plan.world_browser.is_some();
        self.plugin_development = plan.plugin_development.is_some();
        self.plugin_enabled = false;
        self.client_installations = plan.client_installations.is_some();
        self.dialog_paths = plan.dialog_paths.is_some();
        self.structure_export = plan.structure_export.is_some();
        self.nbt_search = plan.nbt_search.is_some();
        self.brush = plan.brush.is_some();
        self.selection = plan.selection.is_some();
        self.layer_preferences = plan.layer_preferences.is_some();
        self.keys = !plan.keys.is_empty() || !plan.navigation.is_empty() || plan.sticky.is_some();
        self.report.clear();
        self.plan = Some(plan);
    }

    pub(crate) fn has_selection(&self) -> bool {
        let Some(plan) = &self.plan else {
            return false;
        };
        [
            self.recent && plan.recent.is_some(),
            self.language && plan.language.is_some(),
            self.geometry && plan.geometry.is_some(),
            self.dock_layout && plan.dock_layout.is_some() && self.layout_anchor.is_some(),
            self.minimap_visibility && plan.minimap_visibility.is_some(),
            self.floating_panels && plan.floating_panels.is_some(),
            self.view && plan.view.is_some(),
            self.camera_projection && plan.camera_perspective.is_some(),
            self.camera_distance && plan.camera_distance.is_some(),
            self.world_browser && plan.world_browser.is_some(),
            self.plugin_development && plan.plugin_development.is_some(),
            self.plugin_enabled && plan.plugin_enabled.is_some(),
            self.client_installations && plan.client_installations.is_some(),
            self.dialog_paths && plan.dialog_paths.is_some(),
            self.structure_export && plan.structure_export.is_some(),
            self.nbt_search && plan.nbt_search.is_some(),
            self.brush && plan.brush.is_some(),
            self.selection && plan.selection.is_some(),
            self.layer_preferences && plan.layer_preferences.is_some(),
            self.keys
                && (!plan.keys.is_empty() || !plan.navigation.is_empty() || plan.sticky.is_some()),
        ]
        .into_iter()
        .any(|selected| selected)
    }
}
fn hex(byte: u8) -> Option<u16> {
    match byte {
        b'0'..=b'9' => Some(u16::from(byte - b'0')),
        b'a'..=b'f' => Some(u16::from(byte - b'a' + 10)),
        b'A'..=b'F' => Some(u16::from(byte - b'A' + 10)),
        _ => None,
    }
}
fn key(bytes: &[u8]) -> Result<String, String> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => {
                out.push(u16::from(b'/'));
                at += 1;
            }
            b'%' => {
                let unicode = bytes.get(at + 1) == Some(&b'U');
                let count = if unicode { 4 } else { 2 };
                let start = at + if unicode { 2 } else { 1 };
                let digits = bytes
                    .get(start..start + count)
                    .ok_or("INI 键的百分号编码不完整")?;
                let mut code = 0u16;
                for &digit in digits {
                    code = code * 16 + hex(digit).ok_or("INI 键的百分号编码无效")?;
                }
                out.push(code);
                at = start + count;
            }
            byte => {
                out.push(u16::from(byte));
                at += 1;
            }
        }
    }
    String::from_utf16(&out).map_err(|e| format!("INI 键包含无效 UTF-16：{e}"))
}
/// Qt INI 的 scalar 字符串。无 BOM 使用 Qt 4 默认 Latin-1；转义的数值是 UTF-16 单元。
fn units(bytes: &[u8], utf8: bool) -> Result<Vec<u16>, String> {
    let mut out = Vec::new();
    let mut at = 0;
    let mut quoted = false;
    let mut was_quoted = false;
    while at < bytes.len() && matches!(bytes[at], b' ' | b'\t') {
        at += 1;
    }
    while at < bytes.len() {
        match bytes[at] {
            b'"' => {
                quoted = !quoted;
                was_quoted = true;
                at += 1;
                if !quoted {
                    while at < bytes.len() && matches!(bytes[at], b' ' | b'\t') {
                        at += 1;
                    }
                }
            }
            b',' if !quoted => return Err("该旧设置需要单个值，实际为 Qt 字符串列表".into()),
            b'\\' => {
                at += 1;
                let escape = *bytes.get(at).ok_or("INI 转义缺少字符")?;
                at += 1;
                let code = match escape {
                    b'a' => 7,
                    b'b' => 8,
                    b'f' => 12,
                    b'n' => 10,
                    b'r' => 13,
                    b't' => 9,
                    b'v' => 11,
                    b'"' | b'\'' | b'?' | b'\\' => u16::from(escape),
                    b'x' | b'0'..=b'7' => {
                        let radix = if escape == b'x' { 16u32 } else { 8 };
                        let mut value = if escape == b'x' {
                            0
                        } else {
                            u32::from(escape - b'0')
                        };
                        let mut count = usize::from(escape != b'x');
                        while let Some(&digit) = bytes.get(at) {
                            let digit = if radix == 16 {
                                hex(digit).map(u32::from)
                            } else {
                                (b'0'..=b'7')
                                    .contains(&digit)
                                    .then(|| u32::from(digit - b'0'))
                            };
                            let Some(digit) = digit else {
                                break;
                            };
                            value = value
                                .checked_mul(radix)
                                .and_then(|v| v.checked_add(digit))
                                .filter(|v| *v <= u16::MAX as u32)
                                .ok_or("INI 字符转义超出 UTF-16 范围")?;
                            count += 1;
                            at += 1;
                        }
                        if count == 0 {
                            return Err("INI 数值转义没有数字".into());
                        }
                        value as u16
                    }
                    _ => return Err(format!("未核实的 INI 转义 \\{}", char::from(escape))),
                };
                out.push(code);
            }
            _ => {
                let start = at;
                while at < bytes.len() && !matches!(bytes[at], b'"' | b'\\' | b',') {
                    at += 1;
                }
                if utf8 {
                    out.extend(
                        std::str::from_utf8(&bytes[start..at])
                            .map_err(|e| format!("INI UTF-8 无效：{e}"))?
                            .encode_utf16(),
                    );
                } else {
                    out.extend(bytes[start..at].iter().map(|b| u16::from(*b)));
                }
            }
        }
    }
    if quoted {
        return Err("INI 引号未闭合".into());
    }
    if !was_quoted {
        while out.last().is_some_and(|c| matches!(*c, 32 | 9)) {
            out.pop();
        }
    }
    Ok(out)
}
struct Ini {
    values: BTreeMap<String, Vec<u8>>,
    utf8: bool,
}
impl Ini {
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let (utf8, bytes) = bytes
            .strip_prefix(&[0xef, 0xbb, 0xbf])
            .map_or((false, bytes), |bytes| (true, bytes));
        let mut values = BTreeMap::new();
        let mut group = String::new();
        let mut line = Vec::new();
        let mut at = 0;
        let mut quoted = false;
        let mut escaped = false;
        let mut comment = false;
        while at <= bytes.len() {
            if at == bytes.len() && escaped {
                return Err("INI 在未完成的转义后结束".into());
            }
            let byte = bytes.get(at).copied().unwrap_or(b'\n');
            at += 1;
            if escaped && matches!(byte, b'\n' | b'\r') {
                line.pop();
                escaped = false;
                if bytes
                    .get(at)
                    .is_some_and(|b| matches!(*b, b'\n' | b'\r') && *b != byte)
                {
                    at += 1;
                }
                continue;
            }
            if matches!(byte, b'\n' | b'\r') {
                if quoted {
                    return Err("INI 的引用值在行尾未闭合".into());
                }
                let row = line.trim_ascii();
                if !row.is_empty() {
                    if row[0] == b'[' {
                        let close = row
                            .iter()
                            .position(|b| *b == b']')
                            .ok_or("INI 分组缺少 ]")?;
                        if !row[close + 1..].trim_ascii().is_empty() {
                            return Err("INI 分组后有额外内容".into());
                        }
                        let name = row[1..close].trim_ascii();
                        group = if name.eq_ignore_ascii_case(b"General") {
                            String::new()
                        } else if name.eq_ignore_ascii_case(b"%General") {
                            "General/".into()
                        } else {
                            format!("{}/", key(name)?)
                        };
                    } else {
                        let split = row
                            .iter()
                            .position(|b| *b == b'=')
                            .ok_or("INI 设置行缺少 =")?;
                        let name = format!("{group}{}", key(row[..split].trim_ascii())?);
                        if name.is_empty() {
                            return Err("INI 设置键为空".into());
                        }
                        values.insert(name, row[split + 1..].to_vec());
                    }
                }
                line.clear();
                escaped = false;
                comment = false;
                continue;
            }
            if comment {
                continue;
            }
            if byte == b';' && !quoted && !escaped {
                comment = true;
                continue;
            }
            line.push(byte);
            if !escaped && byte == b'"' {
                quoted = !quoted;
            }
            escaped = byte == b'\\' && !escaped;
        }
        Ok(Self { values, utf8 })
    }
    fn boolean(&mut self, name: &str) -> Result<Option<bool>, String> {
        self.text(name)?
            .map(|value| match value.as_str() {
                "true" | "1" => Ok(true),
                "false" | "0" => Ok(false),
                _ => Err(format!("{name} 不是有效布尔值：{value}")),
            })
            .transpose()
    }
    fn integer(&mut self, name: &str) -> Result<Option<u16>, String> {
        self.text(name)?
            .map(|value| {
                value
                    .trim()
                    .parse::<u16>()
                    .map_err(|error| format!("{name} 需要非负整数，实际为 {value}：{error}"))
            })
            .transpose()
    }
    fn text(&mut self, name: &str) -> Result<Option<String>, String> {
        let Some(bytes) = self.values.remove(name) else {
            return Ok(None);
        };
        let value = String::from_utf16(&units(&bytes, self.utf8)?)
            .map_err(|e| format!("{name} 不是有效文本：{e}"))?;
        if value == "@Invalid()" {
            return Ok(None);
        }
        if let Some(value) = value.strip_prefix("@@") {
            Ok(Some(format!("@{value}")))
        } else if value.starts_with('@') {
            Err(format!("{name} 不是字符串设置"))
        } else {
            Ok(Some(value))
        }
    }
    /// Qt 4.8 QSettings 固定 Qt_4_0 流：QVariant Vector3D=84，三个大端 Double，
    /// 该流版本没有 Qt_4_2 才加入的 is_null 字节。不实例化任意 QVariant 类型。
    fn brush_size(&mut self) -> Result<Option<[u16; 3]>, String> {
        let Some(value) = self.values.remove("editortools/brush/size") else {
            return Ok(None);
        };
        let value = units(&value, self.utf8)?;
        if value.iter().copied().eq("@Invalid()".encode_utf16()) {
            return Ok(None);
        }
        let prefix = "@Variant(".encode_utf16().collect::<Vec<_>>();
        if !value.starts_with(&prefix) || value.last() != Some(&u16::from(b')')) {
            return Err("旧笔刷尺寸不是 Qt QVector3D QVariant".into());
        }
        let bytes = value[prefix.len()..value.len() - 1]
            .iter()
            .map(|value| u8::try_from(*value).map_err(|_| "旧笔刷尺寸含非字节字符"))
            .collect::<Result<Vec<_>, _>>()?;
        if bytes.len() != 28 || bytes[..4] != 84_u32.to_be_bytes() {
            return Err("旧笔刷尺寸不是已核实的 Qt 4.8 / Qt_4_0 QVector3D 编码".into());
        }
        let mut size = [0; 3];
        for (axis, chunk) in bytes[4..].chunks_exact(8).enumerate() {
            let value = f64::from_be_bytes(chunk.try_into().unwrap());
            if !value.is_finite() || value.fract() != 0. || !(1. ..=512.).contains(&value) {
                return Err(format!(
                    "旧笔刷轴 {} 的尺寸必须为 1..512 整数，实际为 {value}",
                    axis + 1
                ));
            }
            size[axis] = value as u16;
        }
        Ok(Some(size))
    }
    fn byte_array(&mut self, name: &str) -> Result<Option<Vec<u8>>, String> {
        let Some(value) = self.values.remove(name) else {
            return Ok(None);
        };
        let value = units(&value, self.utf8)?;
        if value.iter().copied().eq("@Invalid()".encode_utf16()) {
            return Ok(None);
        }
        let prefix = "@ByteArray(".encode_utf16().collect::<Vec<_>>();
        if !value.starts_with(&prefix) || value.last() != Some(&u16::from(b')')) {
            return Err(format!("{name} 不是 Qt QByteArray"));
        }
        let bytes = value[prefix.len()..value.len() - 1]
            .iter()
            .map(|v| u8::try_from(*v).map_err(|_| format!("{name} 含非字节字符")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(bytes))
    }
    fn geometry(&mut self) -> Result<Option<(Geometry, i32, bool)>, String> {
        let Some(bytes) = self
            .byte_array("mainwindow/geometry")?
            .filter(|bytes| !bytes.is_empty())
        else {
            return Ok(None);
        };
        if bytes.len() != 46
            || bytes[..4] != 0x01d9_d0cbu32.to_be_bytes()
            || bytes[4..8] != [0, 1, 0, 0]
        {
            return Err("仅支持已核实的 Qt 4.8 窗口几何 1.0 格式".into());
        }
        let number =
            |at| i32::from_be_bytes(bytes[at..at + 4].try_into().expect("已核对固定几何长度"));
        let (left, top, right, bottom) = (number(24), number(28), number(32), number(36));
        let width = i64::from(right) - i64::from(left) + 1;
        let height = i64::from(bottom) - i64::from(top) + 1;
        if width <= 0 || height <= 0 || !matches!(bytes[44], 0 | 2) || !matches!(bytes[45], 0 | 4) {
            return Err("Qt 普通窗口矩形或状态无效".into());
        }
        Ok(Some((
            Geometry {
                size: [width as f32, height as f32],
                position: Some([left as f32, top as f32]),
                maximized: bytes[44] != 0,
            },
            number(40),
            bytes[45] != 0,
        )))
    }
}
fn keybindings(
    ini: &mut Ini,
    notices: &mut Vec<String>,
) -> Result<BTreeMap<crate::keyboard_bindings::Command, String>, String> {
    use crate::keyboard_bindings::Command;
    let mut result = BTreeMap::new();
    for (command, action, default) in [
        (Command::Forward, "forward", u32::from(b'W')),
        (Command::Backward, "backward", u32::from(b'S')),
        (Command::Left, "left", u32::from(b'A')),
        (Command::Right, "right", u32::from(b'D')),
        (Command::Up, "up", 0x20),
        (Command::Down, "down", u32::from(b'C')),
    ] {
        let prefix = format!("keybindings/worldview/camera/move/{action}");
        let key = ini.text(&format!("{prefix}/key"))?;
        let modifiers = ini.text(&format!("{prefix}/modifiers"))?;
        let button = ini.text(&format!("{prefix}/button"))?;
        if key.is_none() && modifiers.is_none() && button.is_none() {
            continue;
        }
        let read = |value: Option<String>, name: &str, default| {
            value
                .map(|value| {
                    value
                        .parse::<u32>()
                        .map_err(|e| format!("{prefix}/{name} 不是有效 Qt 整数：{e}"))
                })
                .transpose()
                .map(|value| value.unwrap_or(default))
        };
        let key = read(key, "key", default)?;
        let modifiers = read(modifiers, "modifiers", 0)?;
        let button = read(button, "button", 0)?;
        match crate::navigation_bindings::legacy_flight_binding(button, key, modifiers) {
            Ok(binding) => {
                result.insert(command, binding);
            }
            Err(error) => notices.push(format!("旧移动动作 {action} 未映射：{error}")),
        }
    }
    Ok(result)
}
fn navigation_bindings(
    ini: &mut Ini,
    notices: &mut Vec<String>,
) -> Result<BTreeMap<crate::navigation_bindings::Command, String>, String> {
    use crate::navigation_bindings::{self, Command};
    let mut result = BTreeMap::new();
    for (command, name, button) in [
        (Command::Look, "worldview/camera/holdToTurn", 2),
        (Command::MoveCamera, "worldview/camera/holdToMove", 4),
        (Command::Pan, "worldview/general/holdToMove", 2),
        (Command::ZoomIn, "worldview.general.zoom_in", 0x100),
        (Command::ZoomOut, "worldview.general.zoom_out", 0x200),
    ] {
        let prefix = format!("keybindings/{name}");
        let values = [
            ini.text(&format!("{prefix}/button"))?,
            ini.text(&format!("{prefix}/key"))?,
            ini.text(&format!("{prefix}/modifiers"))?,
        ];
        if values.iter().all(Option::is_none) {
            continue;
        }
        let mut numbers = [button, 0, 0];
        for (index, value) in values.into_iter().enumerate() {
            if let Some(value) = value {
                numbers[index] = value
                    .parse()
                    .map_err(|e| format!("{prefix} 的 Qt 数值无效：{e}"))?;
            }
        }
        match navigation_bindings::legacy_binding(command, numbers[0], numbers[1], numbers[2]) {
            Ok(value) => {
                result.insert(command, value);
            }
            Err(error) => notices.push(format!("旧视图控制 {name} 未映射：{error}")),
        }
    }
    Ok(result)
}
fn read_ini(path: &std::path::Path) -> Result<Ini, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .map_err(|e| format!("{}：{e}", path.display()))?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("旧设置超过 8 MiB".into());
    }
    Ini::parse(&bytes)
}
/// MultiMC 的 InstanceDir 是相对启动器配置目录的路径；复用同一 Qt INI 解码器。
pub(crate) fn launcher_instances(path: &std::path::Path) -> Result<PathBuf, String> {
    let mut ini = read_ini(path)?;
    let value = ini
        .text("InstanceDir")?
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{} 未声明 InstanceDir", path.display()))?;
    let instances = PathBuf::from(value);
    Ok(if instances.is_absolute() {
        instances
    } else {
        path.parent()
            .ok_or("启动器配置文件没有父目录")?
            .join(instances)
    })
}
/// 原 MultiMC 实例的名称和准确客户端版本；不从目录名或其他实例推断。
pub(crate) fn launcher_instance(path: &std::path::Path) -> Result<(String, String), String> {
    let mut ini = read_ini(path)?;
    let version = ini
        .text("IntendedVersion")?
        .filter(|value| crate::auto_assets::is_simple_version(value))
        .ok_or_else(|| format!("{} 未声明有效 IntendedVersion", path.display()))?;
    let name = ini.text("name")?.unwrap_or_default();
    Ok((name, version))
}
fn client_installations(
    ini: &mut Ini,
) -> Result<Option<crate::client_installations::LegacySettings>, String> {
    use crate::client_installations::{Installation, Kind, LegacySettings};
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Install {
        name: String,
        path: PathBuf,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Launcher {
        #[serde(rename = "configFile")]
        config_file: PathBuf,
    }
    let entries = ini
        .text("minecraft_installs/installations")?
        .map(|text| {
            let entries: Vec<Install> =
                serde_json::from_str(&text).map_err(|e| format!("旧客户端安装列表无效：{e}"))?;
            Ok::<_, String>(
                entries
                    .into_iter()
                    .map(|entry| Installation {
                        name: entry.name,
                        path: entry.path,
                        kind: Kind::Minecraft,
                    })
                    .collect(),
            )
        })
        .transpose()?;
    let launchers = ini
        .text("minecraft_installs/multimc_installs")?
        .map(|text| {
            let entries: Vec<Launcher> =
                serde_json::from_str(&text).map_err(|e| format!("旧 MultiMC 配置列表无效：{e}"))?;
            entries
                .into_iter()
                .map(|entry| {
                    let name = entry
                        .config_file
                        .parent()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .ok_or_else(|| {
                            format!(
                                "旧 MultiMC 配置路径没有安装目录名：{}",
                                entry.config_file.display()
                            )
                        })?
                        .to_owned();
                    Ok(Installation {
                        name,
                        path: entry.config_file,
                        kind: Kind::MultiMc,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?;
    let settings = LegacySettings {
        entries,
        launchers,
        selected: ini
            .text("minecraft_installs/current_install_path")?
            .map(|p| (!p.is_empty()).then(|| PathBuf::from(p))),
        version: ini.text("minecraft_installs/current_version")?,
        resource_pack: ini.text("minecraft_installs/current_resource_pack")?,
        allow_snapshots: ini.boolean("minecraft_installs/allow_snapshots_")?,
    };
    if !settings.present() {
        return Ok(None);
    }
    settings.merge(&Default::default())?;
    Ok(Some(settings))
}
pub(super) fn read(path: PathBuf) -> Result<Plan, String> {
    let mut ini = read_ini(&path)?;
    let dock_layout = ini
        .byte_array("mainwindow/state")?
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| layout::Saved::read(&bytes))
        .transpose()?;
    let minimap_visibility = dock_layout
        .as_ref()
        .map(layout::Saved::minimap)
        .transpose()?
        .flatten();
    let floating_panels = dock_layout
        .as_ref()
        .map(layout::Saved::floating)
        .transpose()?
        .flatten();
    let client_installations = client_installations(&mut ini)?;
    let plugin_development = ini.boolean("editor/developer_mode")?;
    let plugin_enabled = ini
        .text("plugins/enabled_plugins")?
        .map(|text| {
            serde_json::from_str::<BTreeMap<String, bool>>(&text)
                .map_err(|error| format!("旧插件启用状态 JSON 无效：{error}"))
        })
        .transpose()?;
    use crate::entity_visibility::Layer as ModelLayer;
    use mcedit2_world::scene_annotations::{CommandLayer, Kind};
    let mob_spawns = ini.boolean("layertoggleoptions/Places Where Creepers Can Spawn_visible")?;
    let layer_preferences = crate::scene_layers::preferences::LegacySettings::new(
        [
            (
                Kind::Entity,
                ini.boolean("layertoggleoptions/MonsterLocations_visible")?,
            ),
            (
                Kind::BlockEntity,
                ini.boolean("layertoggleoptions/TileEntityLocations_visible")?,
            ),
            (
                Kind::BlockTick,
                ini.boolean("layertoggleoptions/TileTicks_visible")?,
            ),
            (
                Kind::Heightmap,
                ini.boolean("layertoggleoptions/HeightMap_visible")?,
            ),
            (
                Kind::ChunkSection,
                ini.boolean("layertoggleoptions/ChunkSections_visible")?,
            ),
            (Kind::SpawnDark, mob_spawns),
            (Kind::SpawnNight, mob_spawns),
        ],
        [
            (
                ModelLayer::Blocks,
                ini.boolean("layertoggleoptions/Blocks_visible")?,
            ),
            (
                ModelLayer::Items,
                ini.boolean("layertoggleoptions/Items_visible")?,
            ),
            (
                ModelLayer::ItemFrames,
                ini.boolean("layertoggleoptions/ItemFrames_visible")?,
            ),
            (
                ModelLayer::BlockEntities,
                ini.boolean("layertoggleoptions/TileEntities_visible")?,
            ),
        ],
        [
            (
                CommandLayer::Colors,
                ini.boolean("layertoggleoptions/CommandBlockColors_visible")?,
            ),
            (
                CommandLayer::Locations,
                ini.boolean("layertoggleoptions/CommandBlockLocations_visible")?,
            ),
        ],
    );
    // 原 Namespace 会再次添加 export_dialog/，实际磁盘键不是单层前缀。
    let author = ini.text("export_dialog/export_dialog/structure/author")?;
    let excluded = ini
        .text("export_dialog/export_dialog/structure/excluded_blocks")?
        .map(|value| {
            serde_json::from_str::<Vec<String>>(&value)
                .map_err(|error| format!("旧结构导出排除列表无效：{error}"))
        })
        .transpose()?;
    let structure_export = (author.is_some() || excluded.is_some())
        .then_some(crate::structure_exchange::export_options::LegacySettings { author, excluded });
    let nbt_search = crate::nbt_search::LegacySettings {
        name: ini.text("findreplace/nbt/nameField")?,
        value: ini.text("findreplace/nbt/valueField")?,
        entities: ini.boolean("findreplace/nbt/searchEntitiesChecked")?,
        block_entities: ini.boolean("findreplace/nbt/searchTileEntitiesChecked")?,
        chunks: ini.boolean("findreplace/nbt/searchChunksChecked")?,
        entity_ids: ini.text("findreplace/nbt/entityIDField")?,
        be_ids: ini.text("findreplace/nbt/tileEntityIDField")?,
        replace_name: ini.boolean("findreplace/nbt/replaceNameEnabled")?,
        new_name: ini.text("findreplace/nbt/replaceNameField")?,
        replace_value: ini.boolean("findreplace/nbt/replaceValueEnabled")?,
        new_value: ini.text("findreplace/nbt/replaceValueField")?,
    };
    let nbt_search = nbt_search.present().then_some(nbt_search);
    let legacy_players = ini.boolean("findreplace/nbt/searchPlayersChecked")?;
    let legacy_replacement_type = ini.integer("findreplace/nbt/replaceValueTagType")?;
    let brush_mode = ini
        .text("editortools/brush/mode")?
        .map(|value| match value.as_str() {
            "fill" => Ok(crate::brush_stroke::Mode::Fill),
            "replace" => Ok(crate::brush_stroke::Mode::Replace),
            "biome" => Ok(crate::brush_stroke::Mode::Biome),
            _ => Err(format!("未知的旧笔刷模式：{value}")),
        })
        .transpose()?;
    let selection_shape = ini
        .text("editortools/brush/shape")?
        .map(|value| {
            use mcedit2_world::editing::SelectionShape;
            match value.as_str() {
                "Square" => Ok(SelectionShape::Box),
                "Round" => Ok(SelectionShape::Round),
                "Diamond" => Ok(SelectionShape::Diamond),
                "Cylinder" => Ok(SelectionShape::Cylinder),
                "ParabolicDome" => Ok(SelectionShape::ParabolicDome),
                "Chunk" => Ok(SelectionShape::Chunks),
                _ => Err(format!("未知的旧共享工具形状：{value}")),
            }
        })
        .transpose()?;
    // ShapeWidget 共用旧键；Chunk 只由 SelectionTool 的 addShapes 注册。
    let brush_shape = selection_shape
        .filter(|shape| *shape != mcedit2_world::editing::SelectionShape::Chunks)
        .map(|shape| shape.brush());
    let classic = ini.boolean("select_tool/classic_selection")?;
    let sticky_selection = ini.boolean("select_tool/sticky_selection")?;
    let selection = (classic.is_some() || sticky_selection.is_some() || selection_shape.is_some())
        .then_some(crate::selection_preferences::LegacySettings {
            classic,
            sticky: sticky_selection,
            shape: selection_shape,
        });
    let brush_size = ini.brush_size()?;
    let brush = (brush_mode.is_some() || brush_shape.is_some() || brush_size.is_some()).then_some(
        crate::brush_stroke::LegacySettings {
            mode: brush_mode,
            shape: brush_shape,
            size: brush_size,
        },
    );
    let mut notices = Vec::new();
    if legacy_players.is_some() {
        notices.push(
            "旧 NBT 玩家搜索未接入执行器，该勾选不导入；玩家文件继续由独立文档入口处理。".into(),
        );
    }
    if legacy_replacement_type.is_some() {
        notices.push(
            "旧 NBT 替换类型选项未驱动实际替换，不映射为原生查询类型；旧替换文本按原节点类型转换。"
                .into(),
        );
    }
    if selection_shape == Some(mcedit2_world::editing::SelectionShape::Chunks) {
        notices.push("旧 Chunk 形状仅用于选框；笔刷形状保持当前值。".into());
    }
    let language = match ini.text("app_language")? {
        None => None,
        Some(value) => match value.as_str() {
            "en" | "en_US" | "en_GB" => Some(Language::English),
            "zh" | "zh_CN" => Some(Language::Chinese),
            _ => {
                notices.push(format!("语言 {value} 没有对应的当前界面翻译"));
                None
            }
        },
    };
    let recent = ini
        .text("open_world_dialog/recent_files")?
        .map(|text| -> Result<Vec<PathBuf>, String> {
            let paths: Vec<String> =
                serde_json::from_str(&text).map_err(|e| format!("最近存档 JSON 无效：{e}"))?;
            if paths.len() > super::RECENT_LIMIT {
                return Err("旧最近存档超过原版 15 项".into());
            }
            let mut result = Vec::new();
            for text in paths {
                let mut root = PathBuf::from(&text);
                if !root.is_absolute() {
                    return Err(format!("旧最近存档路径不是本机绝对路径：{text}"));
                }
                if matches!(
                    root.file_name().and_then(|s| s.to_str()),
                    Some("level.dat" | "level.dat_old")
                ) {
                    root.pop();
                }
                if root
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| matches!(s, "schematic" | "schem" | "nbt" | "zip"))
                {
                    notices.push(format!(
                        "旧最近文件保留其实际路径，打开时识别格式：{}",
                        root.display()
                    ));
                }
                if !result.contains(&root) {
                    result.push(root);
                }
            }
            Ok(result)
        })
        .transpose()?;
    let folders = ini.text("world_list/saves_folders")?;
    let current_folder = ini.text("world_list/current_saves_folder")?;
    let last_folder = ini.text("world_list/last_chosen_saves_folder")?;
    let world_browser = if folders.is_some() || current_folder.is_some() || last_folder.is_some() {
        let paths: Vec<String> = match folders {
            Some(json) => {
                serde_json::from_str(&json).map_err(|e| format!("旧 saves 目录 JSON 无效：{e}"))?
            }
            None => Vec::new(),
        };
        let mut roots = Vec::new();
        for path in paths {
            let path = PathBuf::from(path);
            if !roots.contains(&path) {
                roots.push(path);
            }
        }
        let current = current_folder
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let last_chosen = last_folder
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        if let Some(current) = &current {
            if !roots.contains(current) {
                roots.push(current.clone());
            }
        }
        let saved = crate::world_browser::Saved {
            roots,
            current,
            last_chosen,
        };
        saved
            .validate()
            .map_err(|e| format!("旧 saves 目录无效：{e}"))?;
        Some(saved)
    } else {
        None
    };
    let geometry = ini.geometry()?.map(|(geometry, screen, fullscreen)| {
        notices.push(format!(
            "窗口来自旧屏幕编号 {screen}；位置与大小按源普通窗口矩形导入，请按当前显示器确认"
        ));
        if fullscreen {
            notices.push("旧全屏状态不迁移，使用其普通窗口矩形".into());
        }
        geometry
    });
    let view = match ini.text("editorsession/currentview")? {
        None => None,
        Some(value) => Some(match value.as_str() {
            "Cam" => View::Perspective,
            "Over" => View::Overhead,
            "2D" => View::Slices,
            _ => return Err(format!("未核实的旧视图标识：{value}")),
        }),
    };
    let keys = keybindings(&mut ini, &mut notices)?;
    let navigation = navigation_bindings(&mut ini, &mut notices)?;
    let sticky = ini.boolean("worldview/camera/sticky_mouselook")?;
    let camera_perspective = ini.boolean("worldview/camera/perspective")?;
    let radius = ini.integer("worldview/camera/view_distance")?;
    let maximum = ini.integer("worldview/camera/max_view_distance")?;
    let camera_distance = if radius.is_some() || maximum.is_some() {
        // camera.py 明确声明的两个默认值；不是由当前原生设置猜测旧设置。
        let distance = crate::camera_distance::Saved {
            radius: radius.unwrap_or(12),
            maximum: maximum.unwrap_or(64),
        };
        distance
            .validate()
            .map_err(|error| format!("旧相机视距无效：{error}"))?;
        Some(distance)
    } else {
        None
    };
    let dialog_paths = crate::dialog_paths::Saved::legacy(
        ini.text("open_world_dialog/starting_dir")?
            .filter(|path| !path.is_empty())
            .map(PathBuf::from),
        ini.text("import_dialog/starting_dir")?
            .filter(|path| !path.is_empty())
            .map(PathBuf::from),
    )?;
    if ini.boolean("editor/enable_lighting")?.is_some() {
        // editorapp.enableLightingChanged 写入 relight 模块属性；默认 setMethod("cython")
        // 直接转交 with_cython.updateLightsByCoord，后者未读取该属性。
        notices.push(
            "旧“光照更新”开关不导入：它未接到原版默认重照实现；这不是场景光照显示设置。".into(),
        );
    }
    if ini.boolean("plugins/auto_reload")?.is_some() {
        notices.push(
            "旧 plugins/auto_reload 只有声明、未接入执行逻辑；窗口激活重载按旧开发者模式选项导入。"
                .into(),
        );
    }
    for key in ini.values.keys() {
        notices.push(format!("未映射的旧设置：{key}"));
    }
    Ok(Plan {
        path,
        recent,
        language,
        geometry,
        dock_layout,
        minimap_visibility,
        floating_panels,
        view,
        camera_perspective,
        camera_distance,
        world_browser,
        plugin_development,
        plugin_enabled,
        client_installations,
        dialog_paths,
        structure_export,
        nbt_search,
        brush,
        selection,
        layer_preferences,
        keys,
        navigation,
        sticky,
        notices,
    })
}
pub(super) fn pick() -> Result<Option<Plan>, String> {
    let selected = rfd::FileDialog::new()
        .set_title("导入旧 MCEdit2 设置 / Import legacy MCEdit2 preferences")
        .add_filter("MCEdit2 INI", &["ini"])
        .pick_file();
    selected.map(read).transpose()
}
