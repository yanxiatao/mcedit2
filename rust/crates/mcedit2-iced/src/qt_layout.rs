//! Qt 4.8 QMainWindow::saveState 的只读解码及原生侧栏转换。
//! 格式依据 qt/qt 的 4.8 分支 qmainwindow、qdockarealayout、qtoolbararealayout、
//! qstring.cpp；原程序调用 saveState()，应用版本为 0。转换不实例化 Qt 控件。
use crate::{
    workspace_ui::{
        layout::{self, Side},
        panels::Order,
        Pane, ToolsPane,
    },
    App,
};

#[derive(Clone, Debug)]
struct Dock {
    name: String,
    flags: u8,
    geometry: [i32; 4],
    area: usize,
    parent: usize,
}
impl Dock {
    fn pane(&self) -> Option<Pane> {
        // 来自原 editorapp.py / editorsession.py 的真实 objectName。
        match self.name.as_str() {
            "HistoryWidget" => Some(Pane::History),
            "LibraryWidget" => Some(Pane::Library),
            "inspector" => Some(Pane::Inspect),
            "ToolOptionsWidget" => Some(Pane::Edit),
            _ => None,
        }
    }
    fn visible(&self) -> bool {
        self.flags & 1 != 0
    }
}
#[derive(Clone, Copy, Debug)]
enum Item {
    Dock(usize),
    Group(usize),
}
#[derive(Clone, Debug)]
struct Group {
    current: Option<usize>,
    tabbed: bool,
    horizontal: bool,
    items: Vec<Item>,
}
#[derive(Clone, Debug)]
struct Area {
    side: Side,
    size: [i32; 2],
    group: usize,
}
#[derive(Clone, Debug)]
struct Toolbar {
    name: String,
    side: Side,
    flags: u8,
    position: i32,
    size: i32,
    floating: bool,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Saved {
    docks: Vec<Dock>,
    groups: Vec<Group>,
    areas: Vec<Area>,
    center: Option<[i32; 2]>,
    toolbars: Vec<Toolbar>,
}
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(size).ok_or("Qt 布局长度溢出")?;
        let bytes = self
            .bytes
            .get(self.at..end)
            .ok_or_else(|| format!("Qt 布局在字节 {} 提前结束", self.at))?;
        self.at = end;
        Ok(bytes)
    }
    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn integer(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize, String> {
        usize::try_from(self.integer()?).map_err(|_| "Qt 布局数量不能为负数".into())
    }
    fn pair(&mut self) -> Result<[i32; 2], String> {
        Ok([self.integer()?, self.integer()?])
    }
    fn geometry(&mut self) -> Result<[i32; 4], String> {
        Ok([
            self.integer()?,
            self.integer()?,
            self.integer()?,
            self.integer()?,
        ])
    }
    fn string(&mut self) -> Result<String, String> {
        let size = u32::from_be_bytes(self.take(4)?.try_into().unwrap());
        if size == u32::MAX {
            return Ok(String::new());
        }
        if size % 2 != 0 {
            return Err("Qt 布局 QString 的字节长度必须为偶数".into());
        }
        let units = self
            .take(size as usize)?
            .chunks_exact(2)
            .map(|s| u16::from_be_bytes([s[0], s[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&units).map_err(|e| format!("Qt 布局面板名称不是有效 UTF-16：{e}"))
    }
    fn side(&mut self) -> Result<Side, String> {
        match self.integer()? {
            0 => Ok(Side::Left),
            1 => Ok(Side::Right),
            2 => Ok(Side::Top),
            3 => Ok(Side::Bottom),
            value => Err(format!("Qt 布局停靠区编号无效：{value}")),
        }
    }
    fn header(&mut self, saved: &mut Saved) -> Result<(usize, usize), String> {
        let tabbed = match self.byte()? {
            0xfa => true,
            0xfc => false,
            value => return Err(format!("Qt 布局组标记无效：0x{value:02x}")),
        };
        let current = if tabbed { self.integer()? } else { -1 };
        let horizontal = match self.byte()? {
            1 => true,
            2 => false,
            value => return Err(format!("Qt 布局组方向无效：{value}")),
        };
        let count = self.count()?;
        if current < -1 || (current >= 0 && current as usize >= count) {
            return Err("Qt 布局活动标签超出实际组范围".into());
        }
        let group = saved.groups.len();
        saved.groups.push(Group {
            current: (current >= 0).then_some(current as usize),
            tabbed,
            horizontal,
            items: Vec::new(),
        });
        Ok((group, count))
    }
    fn group(&mut self, saved: &mut Saved, area: usize) -> Result<usize, String> {
        let (root, count) = self.header(saved)?;
        // 使用显式栈读取任意嵌套分栏，不让外部 INI 的嵌套深度占用调用栈。
        let mut stack = vec![(root, count)];
        while let Some((group, remaining)) = stack.last_mut() {
            if *remaining == 0 {
                stack.pop();
                continue;
            }
            *remaining -= 1;
            let parent = *group;
            match self.byte()? {
                0xfb => {
                    let name = self.string()?;
                    let flags = self.byte()?;
                    if flags & !3 != 0 {
                        return Err("Qt 停靠面板状态位无效".into());
                    }
                    let geometry = self.geometry()?;
                    let index = saved.docks.len();
                    saved.docks.push(Dock {
                        name,
                        flags,
                        geometry,
                        area,
                        parent,
                    });
                    saved.groups[parent].items.push(Item::Dock(index));
                }
                0xfc => {
                    self.geometry()?; // 分栏位置、尺寸、最小值和最大值；不当作浮窗矩形。
                    let (child, count) = self.header(saved)?;
                    saved.groups[parent].items.push(Item::Group(child));
                    stack.push((child, count));
                }
                value => return Err(format!("Qt 布局成员标记无效：0x{value:02x}")),
            }
        }
        Ok(root)
    }
}
impl Saved {
    pub fn floating(&self) -> Result<Option<LegacyFloats>, String> {
        use crate::floating_panels::{Geometry, Panel, Saved as Floating};
        let mut entries = std::collections::BTreeMap::new();
        for dock in self.docks.iter().filter(|dock| dock.flags & 2 != 0) {
            let panel = match dock.name.as_str() {
                "ToolOptionsWidget" => Panel::Options,
                "inspector" => Panel::Inspector,
                "HistoryWidget" => Panel::History,
                "LibraryWidget" => Panel::Library,
                "MinimapWidget" => Panel::Minimap,
                "TextureAtlasWidget" => Panel::Atlas,
                _ => continue,
            };
            let [x, y, width, height] = dock.geometry;
            let geometry = Geometry {
                size: [width as f32, height as f32],
                position: Some([x as f32, y as f32]),
                maximized: false,
                open: dock.visible(),
                pinned: false,
            };
            if entries.insert(panel, geometry).is_some() {
                return Err(format!("旧布局含多个对应同一原生面板的浮窗：{}", dock.name));
            }
        }
        Floating(entries.clone()).validate()?;
        Ok((!entries.is_empty()).then_some(LegacyFloats(entries)))
    }
    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        let mut reader = Reader { bytes, at: 0 };
        if reader.integer()? != 0xff || reader.integer()? != 0 {
            return Err("旧面板状态不是原程序使用的 Qt 4.8 / 应用版本 0 格式".into());
        }
        let mut result = Self::default();
        let mut has_toolbars = false;
        while reader.at < bytes.len() {
            match reader.byte()? {
                0xfd => {
                    if result.center.is_some() {
                        return Err("Qt 布局重复保存停靠区".into());
                    }
                    let count = reader.count()?;
                    if count > 4 {
                        return Err("Qt 布局停靠区超过四个方向".into());
                    }
                    for _ in 0..count {
                        let side = reader.side()?;
                        if result.areas.iter().any(|area| area.side == side) {
                            return Err("Qt 布局停靠方向重复".into());
                        }
                        let size = reader.pair()?;
                        let area = result.areas.len();
                        let group = reader.group(&mut result, area)?;
                        result.areas.push(Area { side, size, group });
                    }
                    result.center = Some(reader.pair()?);
                    for _ in 0..4 {
                        reader.integer()?;
                    } // 角落归属，不影响原生双栏。
                }
                0xfc => {
                    if has_toolbars {
                        return Err("Qt 布局重复保存工具栏".into());
                    }
                    has_toolbars = true;
                    for _ in 0..reader.count()? {
                        let side = reader.side()?;
                        for _ in 0..reader.count()? {
                            let name = reader.string()?;
                            let flags = reader.byte()?;
                            if flags & !3 != 0 {
                                return Err("Qt 工具栏状态位无效".into());
                            }
                            let position = reader.integer()?;
                            let size = reader.integer()?;
                            let geom0 = reader.integer()?;
                            reader.integer()?;
                            let floating = geom0 & 1 != 0;
                            result.toolbars.push(Toolbar {
                                name,
                                side,
                                flags,
                                position,
                                size,
                                floating,
                            });
                        }
                    }
                }
                value => return Err(format!("未核对的 Qt 窗口状态标记：0x{value:02x}")),
            }
        }
        Ok(result)
    }
    fn order(&self, root: usize) -> Vec<usize> {
        let mut result = Vec::new();
        let mut stack = vec![Item::Group(root)];
        while let Some(item) = stack.pop() {
            match item {
                Item::Dock(index) => result.push(index),
                Item::Group(index) => stack.extend(self.groups[index].items.iter().rev().copied()),
            }
        }
        result
    }
    fn ratio(&self, index: usize) -> Option<f32> {
        let area = &self.areas[self.docks[index].area];
        let axis = usize::from(matches!(area.side, Side::Top | Side::Bottom));
        let panel = area.size[axis];
        let center = self.center?[axis];
        (panel > 0 && center > 0)
            .then(|| f64::from(panel) / (f64::from(panel) + f64::from(center)))
            .map(|v| v as f32)
    }
    pub fn anchors(&self) -> Vec<Anchor> {
        self.docks
            .iter()
            .enumerate()
            .filter_map(|(index, dock)| {
                dock.pane().map(|pane| Anchor {
                    index,
                    label: format!(
                        "{} · {} · {}",
                        pane.label(crate::language::Language::Chinese),
                        side_label(self.areas[dock.area].side),
                        dock.name
                    ),
                })
            })
            .collect()
    }
    pub fn anchor(&self, index: usize) -> Option<Anchor> {
        self.anchors()
            .into_iter()
            .find(|anchor| anchor.index == index)
    }
    pub fn has_ratio(&self, index: usize) -> bool {
        self.docks
            .get(index)
            .is_some_and(|_| self.ratio(index).is_some())
    }
    pub fn minimap(&self) -> Result<Option<bool>, String> {
        let values = self
            .docks
            .iter()
            .filter(|dock| dock.name == "MinimapWidget")
            .collect::<Vec<_>>();
        match values.as_slice() {
            [] => Ok(None),
            [dock] => Ok(Some(dock.visible())),
            _ => Err("Qt 布局包含重名小地图，不能确定显隐来源".into()),
        }
    }
    pub fn descriptions(&self) -> Vec<String> {
        let mut rows = self
            .docks
            .iter()
            .enumerate()
            .map(|(index, dock)| {
                let area = &self.areas[dock.area];
                let group = &self.groups[dock.parent];
                format!(
                    "{} · {} · {} · {} · {}{} · {:?} · 侧栏占比 {}",
                    dock.name,
                    side_label(area.side),
                    if dock.visible() { "显示" } else { "隐藏" },
                    if dock.flags & 2 != 0 {
                        "浮动"
                    } else {
                        "停靠"
                    },
                    if group.tabbed {
                        "标签组"
                    } else {
                        "分栏组"
                    },
                    if group.horizontal {
                        "（横向）"
                    } else {
                        "（纵向）"
                    },
                    dock.geometry,
                    self.ratio(index)
                        .map(|value| format!("{:.1}%", value * 100.))
                        .unwrap_or_else(|| "未保存有效尺寸".into())
                )
            })
            .collect::<Vec<_>>();
        rows.extend(self.toolbars.iter().map(|bar| {
            format!(
                "工具栏 {} · {} · {} · {} · 位置 {} / 首选尺寸 {}{}",
                bar.name,
                side_label(bar.side),
                if bar.flags & 1 != 0 {
                    "显示"
                } else {
                    "隐藏"
                },
                if bar.flags & 2 != 0 {
                    "纵向"
                } else {
                    "横向"
                },
                bar.position,
                bar.size,
                if bar.floating { " · 浮动" } else { "" }
            )
        }));
        rows
    }
    pub fn prepare(&self, app: &App, anchor: usize, size: bool) -> Result<Prepared, String> {
        let dock = self.docks.get(anchor).ok_or("请选择实际旧面板")?;
        let mut pane = dock.pane().ok_or("所选旧面板没有对应的原生侧栏")?;
        let mut visible = dock.visible();
        let group = &self.groups[dock.parent];
        let root = if group.tabbed {
            dock.parent
        } else {
            self.areas[dock.area].group
        };
        if let Some(Item::Dock(active)) = group.current.and_then(|at| group.items.get(at)) {
            if let Some(active_pane) = self.docks[*active].pane() {
                pane = active_pane;
                visible = self.docks[*active].visible();
            }
        }
        let prefix = self
            .order(root)
            .into_iter()
            .chain(0..self.docks.len())
            .filter_map(|at| self.docks[at].pane())
            .collect::<Vec<_>>();
        let ratio = if size {
            Some(
                self.ratio(anchor)
                    .ok_or("所选旧面板没有有效尺寸，取消导入比例后可保留当前比例")?,
            )
        } else {
            None
        };
        let layout = app.workspace.layout.saved().with_legacy(
            self.areas[dock.area].side,
            ratio,
            !visible,
        )?;
        Ok(Prepared {
            layout,
            order: app.workspace.panel_tabs.with_prefix(&prefix),
            pane,
        })
    }
}
fn side_label(side: Side) -> &'static str {
    match side {
        Side::Left => "左侧",
        Side::Right => "右侧",
        Side::Top => "上方",
        Side::Bottom => "下方",
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Anchor {
    pub index: usize,
    label: String,
}
impl std::fmt::Display for Anchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}
pub(crate) struct Prepared {
    layout: layout::Saved,
    order: Order,
    pane: Pane,
}
#[derive(Clone, Debug)]
pub(crate) struct LegacyFloats(
    std::collections::BTreeMap<crate::floating_panels::Panel, crate::floating_panels::Geometry>,
);
impl LegacyFloats {
    pub fn prepare(&self, app: &App) -> Result<crate::floating_panels::Saved, String> {
        let mut saved = app.desktop.saved.floating_panels.clone();
        for (&panel, geometry) in &self.0 {
            let mut geometry = geometry.clone();
            if let Some(current) = saved.0.get(&panel) {
                geometry.pinned = current.pinned;
            }
            saved.0.insert(panel, geometry);
        }
        crate::floating_panels::validate_update(app, &saved)?;
        Ok(saved)
    }
    pub fn descriptions(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|(panel, geometry)| {
                format!(
                    "{} · {}×{} · 位置 {:?} · {}",
                    panel.label(crate::language::Language::Chinese),
                    geometry.size[0],
                    geometry.size[1],
                    geometry.position,
                    if geometry.open {
                        "打开浮窗"
                    } else {
                        "保留隐藏浮窗的位置"
                    }
                )
            })
            .collect()
    }
}
impl Prepared {
    pub fn install(self, app: &mut App) {
        crate::keyboard_bindings::interrupt(app);
        app.workspace.layout = layout::State::restore(&self.layout);
        app.desktop.saved.panels = self.layout;
        app.workspace.panel_tabs = self.order.clone();
        app.desktop.saved.panel_tabs = self.order;
        app.workspace_pane = self.pane;
        match self.pane {
            Pane::Library => app.tools_pane = ToolsPane::Library,
            Pane::History => app.tools_pane = ToolsPane::History,
            _ => (),
        }
        app.desktop.saved.tools_pane = app.tools_pane;
    }
}
