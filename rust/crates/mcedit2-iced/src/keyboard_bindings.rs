//! 工作区命令与透视飞行的单一按键表；编辑草稿显式应用，文本控件保留自己的输入。
use crate::{workspace_tabs, workspace_ui, App, Message};
use iced::widget::{button, checkbox, column, container, row, scrollable, text, text_input};
use iced::{
    keyboard::{
        key::{Named, Physical},
        Event, Key, Modifiers,
    },
    Element, Length, Task,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[path = "binding_capture.rs"]
mod capture;
#[path = "editor_commands.rs"]
mod editing;
#[path = "tool_commands.rs"]
mod tools;
pub(crate) use capture::Target as CaptureTarget;
pub(crate) use editing::document_available as editing_document_available;
pub(crate) use tools::active as tool_active;
pub(crate) use tools::menu_available as tool_available;
pub(crate) use tools::show_edit as show_edit_tool;
pub(crate) use tools::toolbar as tool_bar;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Command {
    Open,
    Save,
    Close,
    Next,
    Previous,
    MoveLeft,
    MoveRight,
    SelectAll,
    Clear,
    Copy,
    Cut,
    Paste,
    DeleteSelection,
    DeleteBlocks,
    DeleteEntities,
    FillSelection,
    ReplaceBlocks,
    ShowLibrary,
    ExportStructure,
    ImportStructure,
    ToolSelect,
    ToolBrush,
    ToolClone,
    ToolMove,
    ToolFlood,
    ToolBlock,
    ToolEntity,
    ToolChunk,
    ShowPlugins,
    Undo,
    Redo,
    Perspective,
    Isometric,
    Overhead,
    Slices,
    Quad,
    Preferences,
    TogglePanel,
    Forward,
    Backward,
    Left,
    Right,
    Up,
    Down,
    Fast,
}
impl Command {
    pub(crate) const ALL: [Self; 45] = [
        Self::Open,
        Self::Save,
        Self::Close,
        Self::Next,
        Self::Previous,
        Self::MoveLeft,
        Self::MoveRight,
        Self::SelectAll,
        Self::Clear,
        Self::Copy,
        Self::Cut,
        Self::Paste,
        Self::DeleteSelection,
        Self::DeleteBlocks,
        Self::DeleteEntities,
        Self::FillSelection,
        Self::ReplaceBlocks,
        Self::ShowLibrary,
        Self::ExportStructure,
        Self::ImportStructure,
        Self::ToolSelect,
        Self::ToolBrush,
        Self::ToolClone,
        Self::ToolMove,
        Self::ToolFlood,
        Self::ToolBlock,
        Self::ToolEntity,
        Self::ToolChunk,
        Self::ShowPlugins,
        Self::Undo,
        Self::Redo,
        Self::Perspective,
        Self::Isometric,
        Self::Overhead,
        Self::Slices,
        Self::Quad,
        Self::Preferences,
        Self::TogglePanel,
        Self::Forward,
        Self::Backward,
        Self::Left,
        Self::Right,
        Self::Up,
        Self::Down,
        Self::Fast,
    ];
    pub(crate) fn label(self, l: crate::language::Language) -> &'static str {
        let (zh, en) = match self {
            Self::Open => ("打开世界 / 文件", "Open world / file"),
            Self::Save => ("保存当前文档…", "Save current document…"),
            Self::Close => ("关闭当前标签", "Close current tab"),
            Self::Next => ("下一个标签", "Next tab"),
            Self::Previous => ("上一个标签", "Previous tab"),
            Self::MoveLeft => ("标签左移", "Move tab left"),
            Self::MoveRight => ("标签右移", "Move tab right"),
            Self::SelectAll => ("全选当前维度 / 结构", "Select entire dimension / structure"),
            Self::Clear => ("取消选区", "Clear selection"),
            Self::Copy => ("复制选区", "Copy selection"),
            Self::Cut => ("剪切选区", "Cut selection"),
            Self::Paste => ("粘贴选区", "Paste selection"),
            Self::DeleteSelection => ("删除选区内容", "Delete selection content"),
            Self::DeleteBlocks => ("仅删除选区方块", "Delete selection blocks"),
            Self::DeleteEntities => (
                "删除选区实体与掉落物",
                "Delete selection entities and items",
            ),
            Self::FillSelection => ("打开选区填充", "Open selection fill"),
            Self::ReplaceBlocks => ("打开方块批量替换", "Open block replacement"),
            Self::ShowLibrary => ("显示资料库", "Show library"),
            Self::ExportStructure => ("打开结构导出选项", "Open structure export options"),
            Self::ImportStructure => ("打开原位结构导入", "Open in-place structure import"),
            Self::ToolSelect => ("工具：选框", "Tool: selection"),
            Self::ToolBrush => ("工具：连续笔刷", "Tool: continuous brush"),
            Self::ToolClone => ("工具：克隆选区", "Tool: clone selection"),
            Self::ToolMove => ("工具：移动选区", "Tool: move selection"),
            Self::ToolFlood => ("工具：连通填充", "Tool: flood fill"),
            Self::ToolBlock => ("工具：方块检查", "Tool: inspect block"),
            Self::ToolEntity => ("工具：实体检查", "Tool: inspect entity"),
            Self::ToolChunk => ("工具：区块检查", "Tool: inspect chunk"),
            Self::ShowPlugins => ("打开插件与生成器", "Open plugins and generators"),
            Self::Undo => ("撤销当前编辑", "Undo current edit"),
            Self::Redo => ("重做当前编辑", "Redo current edit"),
            Self::Perspective => ("自由相机视图", "Free camera view"),
            Self::Isometric => ("等轴视图", "Isometric view"),
            Self::Overhead => ("俯视视图", "Overhead view"),
            Self::Slices => ("切片视图", "Slices view"),
            Self::Quad => ("四视图", "Quad view"),
            Self::Preferences => ("按键设置", "Keyboard settings"),
            Self::TogglePanel => ("显示 / 隐藏当前侧栏", "Toggle current panel"),
            Self::Forward => ("飞行：前进", "Fly: forward"),
            Self::Backward => ("飞行：后退", "Fly: backward"),
            Self::Left => ("飞行：向左", "Fly: left"),
            Self::Right => ("飞行：向右", "Fly: right"),
            Self::Up => ("飞行：上升", "Fly: up"),
            Self::Down => ("飞行：下降", "Fly: down"),
            Self::Fast => ("飞行：加速", "Fly: faster"),
        };
        l.pick(zh, en)
    }
    fn movement(self) -> Option<char> {
        match self {
            Self::Forward => Some('w'),
            Self::Backward => Some('s'),
            Self::Left => Some('a'),
            Self::Right => Some('d'),
            Self::Up => Some(' '),
            Self::Down => Some('q'),
            Self::Fast => Some('!'),
            _ => None,
        }
    }
    fn defaults(self) -> Vec<Chord> {
        let command = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        let values: Vec<String> = match self {
            Self::Open => vec![format!("{command}+O")],
            Self::Save => vec![format!("{command}+S")],
            Self::Close => vec![format!("{command}+W")],
            Self::Next => vec![format!("{command}+Tab")],
            Self::Previous => vec![format!("{command}+Shift+Tab")],
            Self::MoveLeft => vec![format!("{command}+Shift+PageUp")],
            Self::MoveRight => vec![format!("{command}+Shift+PageDown")],
            Self::SelectAll => vec![format!("{command}+A")],
            Self::Clear => vec![format!("{command}+D")],
            Self::Copy => vec![format!("{command}+C")],
            Self::Cut => vec![format!("{command}+X")],
            Self::Paste => vec![format!("{command}+V")],
            Self::DeleteSelection => vec!["Delete".into()],
            Self::DeleteBlocks => vec!["Shift+Delete".into()],
            Self::DeleteEntities => vec!["Shift+Alt+Delete".into()],
            Self::FillSelection => vec![format!("{command}+Shift+F")],
            Self::ReplaceBlocks => vec![if cfg!(target_os = "macos") {
                format!("{command}+Alt+F")
            } else {
                format!("{command}+H")
            }],
            Self::ShowLibrary => vec![format!("{command}+Shift+L")],
            Self::ExportStructure => vec![format!("{command}+Shift+E")],
            Self::ImportStructure => vec![format!("{command}+Shift+D")],
            Self::ToolSelect
            | Self::ToolBrush
            | Self::ToolClone
            | Self::ToolMove
            | Self::ToolFlood
            | Self::ToolBlock
            | Self::ToolEntity
            | Self::ToolChunk
            | Self::ShowPlugins => Vec::new(),
            Self::Undo => vec![format!("{command}+Z")],
            Self::Redo => vec![format!("{command}+Shift+Z"), format!("{command}+Y")],
            Self::Perspective => vec!["Alt+1".into()],
            Self::Isometric => vec!["Alt+2".into()],
            Self::Overhead => vec!["Alt+3".into()],
            Self::Slices => vec!["Alt+4".into()],
            Self::Quad => vec!["Alt+5".into()],
            Self::Preferences => vec![format!("{command}+Comma")],
            Self::TogglePanel => vec!["F9".into()],
            Self::Forward => vec!["W".into()],
            Self::Backward => vec!["S".into()],
            Self::Left => vec!["A".into()],
            Self::Right => vec!["D".into()],
            Self::Up => vec!["Space".into()],
            Self::Down => vec!["Q".into(), "Shift".into()],
            Self::Fast => vec!["Ctrl".into()],
        };
        values
            .iter()
            .map(|value| Chord::parse(value).expect("内置按键有效"))
            .collect()
    }
    pub(crate) fn view(view: workspace_ui::View) -> Self {
        match view {
            workspace_ui::View::Perspective => Self::Perspective,
            workspace_ui::View::Isometric => Self::Isometric,
            workspace_ui::View::Overhead => Self::Overhead,
            workspace_ui::View::Slices => Self::Slices,
            workspace_ui::View::Quad => Self::Quad,
        }
    }
}
fn named(value: &str) -> Option<Named> {
    macro_rules! names {($($name:ident),*)=>{match value {$ (stringify!($name)=>Some(Named::$name),)* _=>None}};}
    names!(
        Tab,
        Enter,
        Space,
        Escape,
        Backspace,
        Delete,
        Insert,
        Home,
        End,
        PageUp,
        PageDown,
        ArrowUp,
        ArrowDown,
        ArrowLeft,
        ArrowRight,
        Shift,
        Control,
        Alt,
        Meta,
        CapsLock,
        NumLock,
        ScrollLock,
        Pause,
        PrintScreen,
        BrowserBack,
        BrowserForward,
        BrowserStop,
        BrowserRefresh,
        ContextMenu,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F10,
        F11,
        F12,
        F13,
        F14,
        F15,
        F16,
        F17,
        F18,
        F19,
        F20,
        F21,
        F22,
        F23,
        F24,
        F25,
        F26,
        F27,
        F28,
        F29,
        F30,
        F31,
        F32,
        F33,
        F34,
        F35
    )
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Code {
    Character(String),
    Named(String),
}
impl Code {
    fn read(key: &Key) -> Option<Self> {
        match key {
            Key::Named(Named::Super) => Some(Self::Named("Meta".into())),
            Key::Character(v) => Some(Self::Character(v.to_lowercase())),
            Key::Named(n) if named(&format!("{n:?}")).is_some() => {
                Some(Self::Named(format!("{n:?}")))
            }
            _ => None,
        }
    }
    fn modifier(&self) -> u8 {
        match self {
            Self::Named(v) => match v.as_str() {
                "Control" => 1,
                "Alt" => 2,
                "Shift" => 4,
                "Meta" => 8,
                _ => 0,
            },
            _ => 0,
        }
    }
    fn label(&self) -> String {
        match self {
            Self::Character(v) => match v.as_str() {
                "+" => "Plus".into(),
                ";" => "Semicolon".into(),
                "," => "Comma".into(),
                _ => v.to_ascii_uppercase(),
            },
            Self::Named(v) => match v.as_str() {
                "Control" => "Ctrl".into(),
                "Meta" => "Cmd".into(),
                _ => v.clone(),
            },
        }
    }
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Character(v)
                if v.chars().count() == 1
                    && !v.chars().any(char::is_control)
                    && !v.chars().any(char::is_whitespace)
                    && v.to_lowercase() == *v =>
            {
                Ok(())
            }
            Self::Named(v) if named(v).is_some() && v != "Escape" => Ok(()),
            _ => Err("按键名称无效；Escape 保留为取消操作".into()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Chord {
    key: Code,
    modifiers: u8,
}
impl Chord {
    pub(crate) fn is_modifier(&self) -> bool {
        self.key.modifier() != 0
    }
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        Self::parse_with_cancel(value, false)
    }
    pub(crate) fn parse_action(value: &str) -> Result<Self, String> {
        Self::parse_with_cancel(value, true)
    }
    fn parse_with_cancel(value: &str, action: bool) -> Result<Self, String> {
        let tokens = value.trim().split('+').map(str::trim).collect::<Vec<_>>();
        let (key, mods) = tokens.split_last().ok_or("按键为空")?;
        let modifiers = modifier_tokens(mods)?;
        let key = match key.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Code::Named("Control".into()),
            "cmd" | "meta" | "win" | "super" => Code::Named("Meta".into()),
            "plus" => Code::Character("+".into()),
            "semicolon" => Code::Character(";".into()),
            "comma" => Code::Character(",".into()),
            "return" => Code::Named("Enter".into()),
            "esc" => Code::Named("Escape".into()),
            _ if key.chars().count() == 1 => Code::Character(key.to_lowercase()),
            _ => Code::Named(key.to_string()),
        };
        if !(action && key == Code::Named("Escape".into())) {
            key.validate()?;
        }
        if modifiers & key.modifier() != 0 {
            return Err("按键不能同时作为自身的修饰键".into());
        }
        Ok(Self { key, modifiers })
    }
    pub(crate) fn label(&self) -> String {
        let mut parts = Vec::new();
        for (bit, name) in [(1, "Ctrl"), (2, "Alt"), (4, "Shift"), (8, "Cmd")] {
            if self.modifiers & bit != 0 {
                parts.push(name.to_owned());
            }
        }
        parts.push(self.key.label());
        parts.join("+")
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.key.validate()?;
        if self.modifiers > 15 || self.modifiers & self.key.modifier() != 0 {
            return Err("按键修饰位无效".into());
        }
        Ok(())
    }
    pub(crate) fn event(key: &Key, mods: Modifiers) -> Option<Self> {
        let key = Code::read(key)?;
        let modifiers = modifiers(mods) & !key.modifier();
        Some(Self { key, modifiers })
    }
    pub(crate) fn action_event(event: &Event) -> Vec<Self> {
        let Event::KeyPressed {
            key,
            modified_key,
            modifiers,
            ..
        } = event
        else {
            return Vec::new();
        };
        let mut chords = Vec::new();
        if let Some(chord) = Self::event(key, *modifiers) {
            chords.push(chord);
        }
        if let Some(chord) = Self::event(modified_key, *modifiers) {
            if !chords.contains(&chord) {
                chords.push(chord.clone());
            }
            if key != modified_key
                && modifiers.shift()
                && matches!(modified_key, Key::Character(value) if value.chars().all(|character| !character.is_alphabetic()))
            {
                let unshifted = Self {
                    modifiers: chord.modifiers & !4,
                    ..chord
                };
                if !chords.contains(&unshifted) {
                    chords.push(unshifted);
                }
            }
        }
        chords
    }
}
pub(crate) fn modifiers(value: Modifiers) -> u8 {
    u8::from(value.control())
        | u8::from(value.alt()) * 2
        | u8::from(value.shift()) * 4
        | u8::from(value.logo()) * 8
}
pub(crate) fn modifier_tokens(parts: &[&str]) -> Result<u8, String> {
    let mut result = 0;
    for part in parts {
        let bit = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => 1,
            "alt" => 2,
            "shift" => 4,
            "cmd" | "meta" | "win" | "super" => 8,
            _ => return Err(format!("未知修饰键 {part}")),
        };
        if result & bit != 0 {
            return Err(format!("修饰键 {part} 重复"));
        }
        result |= bit;
    }
    Ok(result)
}
pub(crate) fn modifier_label(flags: u8) -> String {
    [(1, "Ctrl+"), (2, "Alt+"), (4, "Shift+"), (8, "Cmd+")]
        .into_iter()
        .filter(|(bit, _)| flags & bit != 0)
        .map(|(_, name)| name)
        .collect()
}
fn describe(values: &[Chord]) -> String {
    values
        .iter()
        .map(Chord::label)
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    overrides: BTreeMap<Command, Vec<Chord>>,
    #[serde(default)]
    mouse: BTreeMap<Command, Vec<crate::navigation_bindings::Binding>>,
    #[serde(default)]
    pub navigation: crate::navigation_bindings::Saved,
}
impl Saved {
    pub(crate) fn merge_legacy(
        &self,
        values: &BTreeMap<Command, String>,
        navigation: &BTreeMap<crate::navigation_bindings::Command, String>,
        sticky: Option<bool>,
    ) -> Result<Self, String> {
        let mut saved = self.clone();
        saved.navigation = saved.navigation.merge(navigation)?;
        if let Some(sticky) = sticky {
            saved.navigation.sticky = sticky;
        }
        for (&command, text) in values {
            let (chords, mouse) = parse_inputs(command, text)?;
            saved.overrides.insert(command, chords);
            saved.mouse.remove(&command);
            if !mouse.is_empty() {
                saved.mouse.insert(command, mouse);
            }
        }
        saved.validate()?;
        Ok(saved)
    }
    fn active(&self) -> BTreeMap<Command, Vec<Chord>> {
        Command::ALL
            .into_iter()
            .map(|cmd| {
                (
                    cmd,
                    self.overrides
                        .get(&cmd)
                        .cloned()
                        .unwrap_or_else(|| cmd.defaults()),
                )
            })
            .collect()
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.navigation.validate()?;
        let navigation = self.navigation.compile();
        let mut mouse_inputs = BTreeMap::new();
        for (&command, bindings) in &self.mouse {
            if command.movement().is_none() {
                return Err("工作区命令不能绑定鼠标".into());
            }
            for binding in bindings {
                if !matches!(binding, crate::navigation_bindings::Binding::Mouse(..)) {
                    return Err("鼠标飞行记录必须是实际鼠标按钮".into());
                }
                binding.validate_hold()?;
                if let Some(other) = navigation.conflicts_binding(binding, true) {
                    return Err(format!(
                        "{} 同时绑定了“{}”和“{}”",
                        binding.label(),
                        command.label(crate::language::Language::Chinese),
                        other.label(crate::language::Language::Chinese)
                    ));
                }
                if let Some(previous) = mouse_inputs.insert(binding, command) {
                    return Err(format!(
                        "{} 同时绑定了“{}”和“{}”",
                        binding.label(),
                        previous.label(crate::language::Language::Chinese),
                        command.label(crate::language::Language::Chinese)
                    ));
                }
            }
        }
        let mut used = BTreeMap::new();
        for (command, values) in self.active() {
            for value in values {
                value.key.validate()?;
                if value.modifiers > 15 || value.modifiers & value.key.modifier() != 0 {
                    return Err("按键修饰位无效".into());
                }
                if command.movement().is_none() && value.key.modifier() != 0 {
                    return Err("工作区命令需要普通键，不能只绑定修饰键".into());
                }
                if let Some(other) = navigation.conflicts(&value, command.movement().is_some()) {
                    return Err(format!(
                        "{} 同时绑定了“{}”和“{}”",
                        value.label(),
                        command.label(crate::language::Language::Chinese),
                        other.label(crate::language::Language::Chinese)
                    ));
                }
                if let Some(previous) = used.insert(value.clone(), command) {
                    return Err(format!(
                        "{} 同时绑定了“{}”和“{}”",
                        value.label(),
                        previous.label(crate::language::Language::Chinese),
                        command.label(crate::language::Language::Chinese)
                    ));
                }
            }
        }
        Ok(())
    }
}
struct Held {
    physical: Physical,
    key: Code,
    movement: char,
    modifier: u8,
}
struct MouseHeld {
    button: iced::mouse::Button,
    movement: char,
}
fn parse_inputs(
    command: Command,
    text: &str,
) -> Result<(Vec<Chord>, Vec<crate::navigation_bindings::Binding>), String> {
    use crate::navigation_bindings::Binding;
    let mut keys = Vec::new();
    let mut mouse = Vec::new();
    if text.trim().is_empty() {
        return Ok((keys, mouse));
    }
    for token in text.split(';') {
        match Binding::parse(token.trim())? {
            Binding::Key(key) => keys.push(key),
            binding if command.movement().is_some() => {
                binding.validate_hold()?;
                mouse.push(binding);
            }
            _ => return Err("工作区命令需要键盘组合，鼠标绑定用于飞行与视图导航".into()),
        }
    }
    Ok((keys, mouse))
}
fn movement_state(app: &mut App) {
    app.spectator_keys = app
        .shortcuts
        .held
        .iter()
        .map(|held| held.movement)
        .chain(app.shortcuts.mouse_held.iter().map(|held| held.movement))
        .collect();
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owner {
    World(usize, usize),
    Structure(u64),
}
pub(crate) fn owner(app: &App) -> Option<Owner> {
    if app.structures.active.is_some() {
        crate::structure_workspace::active(app).map(Owner::Structure)
    } else {
        app.current_key()
            .map(|(world, dimension)| Owner::World(world, dimension))
    }
}
pub(crate) struct State {
    pub saved: Saved,
    active: BTreeMap<Command, Vec<Chord>>,
    pub open: bool,
    draft: BTreeMap<Command, String>,
    report: String,
    held: Vec<Held>,
    mouse_held: Vec<MouseHeld>,
    input_owner: Option<Owner>,
    pub mouse_flight: std::sync::Arc<BTreeMap<crate::navigation_bindings::Binding, Command>>,
    pub navigation: crate::navigation_bindings::State,
    draft_navigation: BTreeMap<crate::navigation_bindings::Command, String>,
    draft_sticky: bool,
    capture: Option<capture::Recording>,
}
impl Default for State {
    fn default() -> Self {
        Self::restore(Saved::default())
    }
}
impl State {
    /// 固定菜单助记键只使用未被用户分配给命令、飞行或导航的按键。
    pub(crate) fn binds_key(&self, event: &Event) -> bool {
        let Event::KeyPressed {
            key,
            modifiers: mods,
            ..
        } = event
        else {
            return false;
        };
        let Some(key) = Code::read(key) else {
            return false;
        };
        let chord = Chord {
            modifiers: modifiers(*mods) & !key.modifier(),
            key,
        };
        self.active.values().any(|values| values.contains(&chord))
            || self.navigation.config.conflicts(&chord, false).is_some()
    }
    pub(crate) fn restore(saved: Saved) -> Self {
        Self {
            active: saved.active(),
            mouse_flight: std::sync::Arc::new(
                saved
                    .mouse
                    .iter()
                    .flat_map(|(&cmd, bindings)| {
                        bindings.iter().cloned().map(move |binding| (binding, cmd))
                    })
                    .collect(),
            ),
            navigation: crate::navigation_bindings::State::new(&saved.navigation),
            saved,
            open: false,
            draft: BTreeMap::new(),
            report: String::new(),
            held: Vec::new(),
            mouse_held: Vec::new(),
            input_owner: None,
            draft_navigation: BTreeMap::new(),
            draft_sticky: true,
            capture: None,
        }
    }
    pub(crate) fn hint(&self, command: Command) -> String {
        let mut values = self.active[&command]
            .iter()
            .map(Chord::label)
            .collect::<Vec<_>>();
        if let Some(mouse) = self.saved.mouse.get(&command) {
            values.extend(mouse.iter().map(crate::navigation_bindings::Binding::label));
        }
        values.join("; ")
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Invoke(Command),
    Capture(CaptureTarget, bool),
    CaptureCancel(u64),
    CaptureClear(u64),
    Captured(u64, Result<crate::navigation_bindings::Binding, String>),
    Open,
    Cancel,
    Edit(Command, String),
    Reset(Command),
    ResetAll,
    Apply,
    ClearMovement,
    Modifiers(Modifiers),
    EditNavigation(crate::navigation_bindings::Command, String),
    ResetNavigation(crate::navigation_bindings::Command),
    Sticky(bool),
    MouseDown(iced::mouse::Button, Command, u64, Option<Owner>),
    MouseUp(iced::mouse::Button),
}
/// 打开独占对话框或失焦时结束手势；笔刷使用代次拒绝继续先前尚未松开的笔画。
pub(crate) fn interrupt(app: &mut App) {
    interrupt_view_gesture(app);
    app.spectator_keys.clear();
    app.shortcuts.held.clear();
    app.shortcuts.mouse_held.clear();
    app.shortcuts.input_owner = None;
    app.spectator_last_frame = None;
    crate::structure_workspace::sync_input(app);
}
/// 导航接管选框与笔画，已经按住的飞行键继续生效。
pub(crate) fn interrupt_view_gesture(app: &mut App) {
    crate::plugin_ui::cancel_gesture(app);
    crate::placement::cancel_gesture(app);
    crate::flood_fill::cancel_gesture(app);
    app.input_epoch = app.input_epoch.checked_add(1).expect("输入手势代次溢出");
    app.workspace.dragging = None;
    app.shortcuts.navigation.clear();
    app.selection_edit
        .pointer
        .lock()
        .expect("选框手势锁被毒化")
        .cancel();
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Invoke(command) => return dispatch(app, command),
        Action::Capture(target, append) => {
            if !app.shortcuts.open {
                return Task::none();
            }
            interrupt(app);
            app.shortcuts.capture = Some(capture::Recording {
                epoch: app.input_epoch,
                target,
                append,
            });
            app.shortcuts.report.clear();
        }
        Action::CaptureCancel(epoch) => {
            if app
                .shortcuts
                .capture
                .is_some_and(|recording| recording.epoch == epoch)
            {
                app.shortcuts.capture = None;
                app.shortcuts.report.clear();
                interrupt(app);
            }
        }
        Action::CaptureClear(epoch) => {
            if let Some(recording) = app
                .shortcuts
                .capture
                .filter(|recording| recording.epoch == epoch)
            {
                recording.target.write(&mut app.shortcuts, String::new());
                app.shortcuts.capture = None;
                app.shortcuts.report = app
                    .language
                    .pick(
                        "已清空此项草稿；点击应用后停用绑定。",
                        "Draft cleared; Apply to disable this binding.",
                    )
                    .into();
                interrupt(app);
            }
        }
        Action::Captured(epoch, value) => {
            let Some(recording) = app
                .shortcuts
                .capture
                .filter(|recording| recording.epoch == epoch && app.shortcuts.open)
            else {
                return Task::none();
            };
            match value.and_then(|value| {
                recording
                    .target
                    .combine(&app.shortcuts, value, recording.append)
            }) {
                Ok(value) => {
                    recording.target.write(&mut app.shortcuts, value);
                    app.shortcuts.capture = None;
                    app.shortcuts.report = app
                        .language
                        .pick(
                            "已录入草稿；点击应用后生效。",
                            "Binding recorded in draft; Apply to activate it.",
                        )
                        .into();
                    interrupt(app);
                }
                Err(error) => {
                    log::error!("[binding-capture] {error}");
                    app.shortcuts.report = error;
                }
            }
        }
        Action::MouseDown(button, command, epoch, input) => {
            if epoch != app.input_epoch || input != owner(app) || !enabled(app) {
                return Task::none();
            }
            let movement = command.movement().expect("鼠标飞行由移动命令生成");
            app.shortcuts
                .mouse_held
                .push(MouseHeld { button, movement });
            app.shortcuts.input_owner = input;
            movement_state(app);
        }
        Action::MouseUp(button) => {
            app.shortcuts
                .mouse_held
                .retain(|held| held.button != button);
            movement_state(app);
        }
        Action::Sticky(value) => app.shortcuts.draft_sticky = value,
        Action::Modifiers(modifiers) => app.shortcuts.navigation.modifiers = modifiers,
        Action::EditNavigation(command, value) => {
            app.shortcuts.draft_navigation.insert(command, value);
            app.shortcuts.report.clear();
        }
        Action::ResetNavigation(command) => {
            app.shortcuts.draft_navigation.insert(
                command,
                crate::navigation_bindings::Config::default_hint(command),
            );
            app.shortcuts.report.clear();
        }
        Action::ClearMovement => interrupt(app),
        Action::Open => {
            interrupt(app);
            app.shortcuts.open = true;
            app.shortcuts.capture = None;
            app.shortcuts.draft_sticky = app.shortcuts.saved.navigation.sticky;
            app.shortcuts.draft = Command::ALL
                .into_iter()
                .map(|cmd| (cmd, app.shortcuts.hint(cmd)))
                .collect();
            app.shortcuts.report.clear();
            app.shortcuts.draft_navigation = crate::navigation_bindings::Command::ALL
                .into_iter()
                .map(|cmd| (cmd, app.shortcuts.navigation.config.hint(cmd)))
                .collect();
        }
        Action::Cancel => {
            app.shortcuts.open = false;
            app.shortcuts.capture = None;
            app.shortcuts.report.clear();
        }
        Action::Edit(cmd, value) => {
            app.shortcuts.draft.insert(cmd, value);
            app.shortcuts.report.clear();
        }
        Action::Reset(cmd) => {
            app.shortcuts.draft.insert(cmd, describe(&cmd.defaults()));
            app.shortcuts.report.clear();
        }
        Action::ResetAll => {
            app.shortcuts.capture = None;
            app.shortcuts.draft_sticky = true;
            app.shortcuts.draft_navigation = crate::navigation_bindings::Command::ALL
                .into_iter()
                .map(|cmd| (cmd, crate::navigation_bindings::Config::default_hint(cmd)))
                .collect();
            app.shortcuts.draft = Command::ALL
                .into_iter()
                .map(|cmd| (cmd, describe(&cmd.defaults())))
                .collect();
            app.shortcuts.report.clear();
        }
        Action::Apply => {
            let parsed = (|| {
                let mut saved = Saved::default();
                saved.navigation = crate::navigation_bindings::Saved::parse(
                    &app.shortcuts.draft_navigation,
                    app.shortcuts.draft_sticky,
                )?;
                for cmd in Command::ALL {
                    let value = &app.shortcuts.draft[&cmd];
                    let (values, mouse) = parse_inputs(cmd, value)
                        .map_err(|e| format!("{}：{e}", cmd.label(app.language)))?;
                    if !mouse.is_empty() {
                        saved.mouse.insert(cmd, mouse);
                    }
                    if values != cmd.defaults() {
                        saved.overrides.insert(cmd, values);
                    }
                }
                saved.validate()?;
                Ok::<_, String>(saved)
            })();
            match parsed {
                Ok(saved) => {
                    let modifiers = app.shortcuts.navigation.modifiers;
                    app.shortcuts = State::restore(saved);
                    app.shortcuts.navigation.modifiers = modifiers;
                    app.spectator_keys.clear();
                }
                Err(error) => {
                    log::error!("[keyboard] {error}");
                    app.shortcuts.report = error;
                }
            }
        }
    }
    Task::none()
}
pub(crate) fn flight_active(app: &App) -> bool {
    !app.floating_panels.focused()
        && !crate::world_save::busy(app)
        && !app.about.open
        && !app.drag_drop.engaged()
        && (crate::navigation_bindings::perspective_active(app)
            || crate::structure_workspace::perspective_active(app))
}
fn enabled(app: &App) -> bool {
    flight_active(app)
}
pub(crate) fn sync(app: &mut App) {
    crate::navigation_bindings::sync(app);
    if !enabled(app) || app.spectator_keys.is_empty() || app.shortcuts.input_owner != owner(app) {
        app.shortcuts.held.clear();
        app.shortcuts.mouse_held.clear();
        app.shortcuts.input_owner = None;
        app.spectator_keys.clear();
    }
    if !enabled(app) {
        app.spectator_keys.clear();
    }
    if crate::navigation_bindings::perspective_active(app)
        && crate::has_flight_input(&app.spectator_keys)
    {
        app.spectator_last_frame
            .get_or_insert_with(std::time::Instant::now);
    } else {
        app.spectator_last_frame = None;
    }
    crate::structure_workspace::sync_input(app);
}
pub(crate) fn movement(app: &mut App, event: Event) -> Task<Message> {
    match event {
        Event::KeyReleased {
            key, physical_key, ..
        } => {
            let code = Code::read(&key);
            app.shortcuts.held.retain(|held| {
                // Iced 对虚拟 / 未识别物理键仍提供逻辑键；释放不依赖当时的修饰位。
                !(held.physical == physical_key
                    && (matches!(physical_key, Physical::Code(_))
                        || code.as_ref() == Some(&held.key)))
            });
        }
        Event::KeyPressed {
            key: Key::Named(Named::Escape),
            ..
        } => app.shortcuts.held.clear(),
        Event::KeyPressed {
            key,
            physical_key,
            modifiers: mods,
            repeat: false,
            ..
        } if enabled(app) => {
            if let Some(code) = Code::read(&key) {
                let flags = modifiers(mods) & !code.modifier();
                let held_modifiers = app
                    .shortcuts
                    .held
                    .iter()
                    .fold(0, |mask, held| mask | held.modifier);
                let candidates = Command::ALL
                    .into_iter()
                    .filter_map(|cmd| cmd.movement().map(|movement| (cmd, movement)));
                let found = candidates
                    .clone()
                    .find(|(cmd, _)| {
                        app.shortcuts.active[cmd]
                            .iter()
                            .any(|chord| chord.key == code && chord.modifiers == flags)
                    })
                    .or_else(|| {
                        candidates.into_iter().find(|(cmd, _)| {
                            app.shortcuts.active[cmd].iter().any(|chord| {
                                chord.key == code && chord.modifiers == (flags & !held_modifiers)
                            })
                        })
                    });
                if let Some((_, movement)) = found {
                    app.shortcuts.input_owner = owner(app);
                    app.shortcuts.held.retain(|held| {
                        held.physical != physical_key
                            || matches!(physical_key, Physical::Unidentified(_)) && held.key != code
                    });
                    let modifier = code.modifier();
                    app.shortcuts.held.push(Held {
                        physical: physical_key,
                        key: code,
                        movement,
                        modifier,
                    });
                }
            }
        }
        _ => (),
    }
    movement_state(app);
    sync(app);
    Task::none()
}
pub(crate) fn label(app: &App, command: Command, zh: &'static str, en: &'static str) -> String {
    let name = app.language.pick(zh, en);
    let keys = app.shortcuts.hint(command);
    if keys.is_empty() {
        name.into()
    } else {
        format!("{name} · {keys}")
    }
}
pub(crate) fn flight_hint(app: &App) -> String {
    let keys = |cmd| {
        let value = app.shortcuts.hint(cmd);
        if value.is_empty() {
            app.language.pick("未绑定", "Unbound").into()
        } else {
            value
        }
    };
    let (forward, left, backward, right, up, down, fast) = (
        keys(Command::Forward),
        keys(Command::Left),
        keys(Command::Backward),
        keys(Command::Right),
        keys(Command::Up),
        keys(Command::Down),
        keys(Command::Fast),
    );
    match app.language {
        crate::language::Language::Chinese => format!(
            "飞行 {forward}/{left}/{backward}/{right} · 上升 {up} · 下降 {down} · 加速 {fast}"
        ),
        crate::language::Language::English => format!(
            "Fly {forward}/{left}/{backward}/{right} · Rise {up} · Descend {down} · Faster {fast}"
        ),
    }
}
pub(crate) fn panel_command(app: &App, event: &Event) -> Option<Command> {
    let Event::KeyPressed {
        key,
        modifiers: mods,
        repeat: false,
        ..
    } = event
    else {
        return None;
    };
    let code = Code::read(key)?;
    let flags = modifiers(*mods) & !code.modifier();
    Command::ALL
        .into_iter()
        .filter(|cmd| cmd.movement().is_none())
        .find(|cmd| {
            app.shortcuts.active[cmd]
                .iter()
                .any(|chord| chord.key == code && chord.modifiers == flags)
        })
}
pub(crate) fn keyboard(app: &mut App, event: Event) -> Task<Message> {
    if crate::world_save::busy(app)
        || app.about.open
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.main_menu.is_some()
    {
        return Task::none();
    }
    if matches!(
        event,
        Event::KeyPressed {
            key: Key::Named(Named::Escape),
            ..
        }
    ) {
        interrupt_view_gesture(app);
    }
    if crate::plugin_ui::shortcut_pending(app) {
        if let Some(task) = crate::plugin_ui::shortcut_keyboard(app, &event) {
            return task;
        }
    }
    if let Event::KeyPressed {
        key,
        modifiers: mods,
        repeat: false,
        ..
    } = &event
    {
        if let Some(code) = Code::read(key) {
            let flags = modifiers(*mods) & !code.modifier();
            if let Some(command) = Command::ALL
                .into_iter()
                .filter(|cmd| cmd.movement().is_none())
                .find(|cmd| {
                    app.shortcuts.active[cmd]
                        .iter()
                        .any(|chord| chord.key == code && chord.modifiers == flags)
                })
            {
                return dispatch(app, command);
            }
        }
    }
    if let Some(task) = crate::plugin_ui::shortcut_keyboard(app, &event) {
        return task;
    }
    keyboard_continuation(app, event)
}
pub(crate) fn keyboard_continuation(app: &mut App, event: Event) -> Task<Message> {
    if let Some(task) = crate::structure_workspace::navigation_key(app, &event) {
        return task;
    }
    if let Some(task) = crate::navigation_bindings::keyboard(app, &event) {
        return task;
    }
    movement(app, event)
}
fn dispatch(app: &mut App, command: Command) -> Task<Message> {
    if crate::world_save::busy(app) || app.about.open {
        return Task::none();
    }
    use workspace_tabs::{Action as World, CloseTarget};
    use Command::*;
    if let Some(task) = editing::dispatch(app, command) {
        return task;
    }
    if app.desktop.show_recent {
        return match command {
            Open => crate::file_input::update(app, crate::file_input::Action::Pick),
            Preferences => update(app, Action::Open),
            Next => workspace_tabs::update(app, World::Cycle(false)),
            Previous => workspace_tabs::update(app, World::Cycle(true)),
            _ => Task::none(),
        };
    }
    if let Some(id) = crate::structure_workspace::active(app) {
        return match command {
            Open => crate::file_input::update(app, crate::file_input::Action::Pick),
            Preferences => update(app, Action::Open),
            Save => workspace_tabs::update(app, World::Save),
            Close => workspace_tabs::update(app, World::Close(CloseTarget::Structure(id))),
            Next => workspace_tabs::update(app, World::Cycle(false)),
            Previous => workspace_tabs::update(app, World::Cycle(true)),
            MoveLeft => workspace_tabs::update(app, World::Move(true)),
            MoveRight => workspace_tabs::update(app, World::Move(false)),
            Undo => workspace_tabs::history(app, false),
            Redo => workspace_tabs::history(app, true),
            DeleteSelection => crate::deletion_ui::update(app, crate::deletion_ui::Action::Quick),
            TogglePanel => {
                let kind = workspace_ui::layout::active(app);
                workspace_ui::layout::update(app, workspace_ui::layout::Action::Toggle(kind))
            }
            Copy | Cut | Paste | Clear | SelectAll => crate::structure_workspace::update(
                app,
                id,
                match command {
                    Copy => crate::structure_workspace::Action::Work(
                        crate::structure_workspace::Work::Copy(false),
                    ),
                    Cut => crate::structure_workspace::Action::Work(
                        crate::structure_workspace::Work::Copy(true),
                    ),
                    Paste => crate::structure_workspace::Action::Paste,
                    Clear => crate::structure_workspace::Action::ClearSelection,
                    SelectAll => crate::structure_workspace::Action::SelectAll,
                    _ => unreachable!(),
                },
            ),
            Perspective | Isometric | Overhead | Slices | Quad => {
                crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::SceneMode(match command {
                        Perspective => workspace_ui::View::Perspective,
                        Isometric => workspace_ui::View::Isometric,
                        Overhead => workspace_ui::View::Overhead,
                        Slices => workspace_ui::View::Slices,
                        Quad => workspace_ui::View::Quad,
                        _ => unreachable!(),
                    }),
                )
            }
            Forward | Backward | Left | Right | Up | Down | Fast => {
                unreachable!("持续移动由释放键跟踪处理")
            }
            DeleteBlocks | DeleteEntities | FillSelection | ReplaceBlocks | ShowLibrary
            | ExportStructure | ImportStructure | ToolSelect | ToolBrush | ToolClone | ToolMove
            | ToolFlood | ToolBlock | ToolEntity | ToolChunk | ShowPlugins => {
                unreachable!("编辑操作由公共文档路由处理")
            }
        };
    }
    if app.current_key().is_none() && !matches!(command, Open | Preferences) {
        return Task::none();
    }
    let file = crate::file_ui::active_document_id(app);
    if let Some(file) = file {
        if matches!(
            command,
            DeleteSelection
                | SelectAll
                | Clear
                | Copy
                | Cut
                | Paste
                | Perspective
                | Isometric
                | Overhead
                | Slices
                | Quad
        ) {
            return Task::none();
        }
        use crate::nbt_documents::Action as Document;
        let action = match command {
            Close => Some(Document::Close(file)),
            Next => Some(Document::Cycle(false)),
            Previous => Some(Document::Cycle(true)),
            MoveLeft => Some(Document::Move(true)),
            MoveRight => Some(Document::Move(false)),
            _ => None,
        };
        if let Some(action) = action {
            interrupt(app);
            return crate::nbt_documents::update(app, action);
        }
    }
    match command {
        Open => crate::file_input::update(app, crate::file_input::Action::Pick),
        Save => workspace_tabs::update(app, World::Save),
        Close => app.current.map_or_else(Task::none, |world| {
            workspace_tabs::update(app, World::Close(CloseTarget::World(world)))
        }),
        Next => workspace_tabs::update(app, World::Cycle(false)),
        Previous => workspace_tabs::update(app, World::Cycle(true)),
        MoveLeft => workspace_tabs::update(app, World::Move(true)),
        MoveRight => workspace_tabs::update(app, World::Move(false)),
        SelectAll => crate::update_inner(
            app,
            Message::SelectionAll(crate::selection_edit::ExtentAction::Start),
        ),
        Clear => crate::update_inner(
            app,
            Message::SelectionEdit(crate::selection_edit::Action::Clear),
        ),
        Copy => crate::selection_clipboard::update(app, crate::selection_clipboard::Action::Copy),
        Cut => crate::selection_clipboard::update(app, crate::selection_clipboard::Action::Cut),
        Paste => crate::selection_clipboard::update(app, crate::selection_clipboard::Action::Paste),
        DeleteSelection => crate::deletion_ui::update(app, crate::deletion_ui::Action::Quick),
        Undo => workspace_tabs::history(app, false),
        Redo => workspace_tabs::history(app, true),
        Perspective | Isometric | Overhead | Slices | Quad => crate::update_inner(
            app,
            Message::WorkspaceView(match command {
                Perspective => workspace_ui::View::Perspective,
                Isometric => workspace_ui::View::Isometric,
                Overhead => workspace_ui::View::Overhead,
                Slices => workspace_ui::View::Slices,
                Quad => workspace_ui::View::Quad,
                _ => unreachable!(),
            }),
        ),
        Preferences => update(app, Action::Open),
        TogglePanel => {
            let kind = workspace_ui::layout::active(app);
            workspace_ui::layout::update(app, workspace_ui::layout::Action::Toggle(kind))
        }
        Forward | Backward | Left | Right | Up | Down | Fast => {
            unreachable!("持续移动由释放键跟踪处理")
        }
        DeleteBlocks | DeleteEntities | FillSelection | ReplaceBlocks | ShowLibrary
        | ExportStructure | ImportStructure | ToolSelect | ToolBrush | ToolClone | ToolMove
        | ToolFlood | ToolBlock | ToolEntity | ToolChunk | ShowPlugins => {
            unreachable!("编辑操作由公共文档路由处理")
        }
    }
}
/// Qt 4.8 Key / KeyboardModifier 的明确数值映射；鼠标绑定由导入方单独列出。
pub(crate) fn legacy_key(key: u32, flags: u32) -> Result<String, String> {
    if flags & !0x1e00_0000 != 0 {
        return Err("旧按键包含当前未映射的修饰位".into());
    }
    if key == 0 {
        return Ok(String::new());
    }
    let mut modifiers = u8::from(flags & 0x0400_0000 != 0)
        | u8::from(flags & 0x0800_0000 != 0) * 2
        | u8::from(flags & 0x0200_0000 != 0) * 4
        | u8::from(flags & 0x1000_0000 != 0) * 8;
    let name = match key {
        0x20 => Some("Space"),
        0x0100_0000 => Some("Escape"),
        0x0100_0001 => Some("Tab"),
        0x0100_0002 => {
            modifiers |= 4;
            Some("Tab")
        }
        0x0100_0003 => Some("Backspace"),
        0x0100_0004 | 0x0100_0005 => Some("Enter"),
        0x0100_0006 => Some("Insert"),
        0x0100_0007 => Some("Delete"),
        0x0100_0008 => Some("Pause"),
        0x0100_0009 => Some("PrintScreen"),
        0x0100_0010 => Some("Home"),
        0x0100_0011 => Some("End"),
        0x0100_0012 => Some("ArrowLeft"),
        0x0100_0013 => Some("ArrowUp"),
        0x0100_0014 => Some("ArrowRight"),
        0x0100_0015 => Some("ArrowDown"),
        0x0100_0016 => Some("PageUp"),
        0x0100_0017 => Some("PageDown"),
        0x0100_0020 => Some("Shift"),
        0x0100_0021 => Some("Control"),
        0x0100_0022 => Some("Meta"),
        0x0100_0023 => Some("Alt"),
        0x0100_0024 => Some("CapsLock"),
        0x0100_0025 => Some("NumLock"),
        0x0100_0026 => Some("ScrollLock"),
        _ => None,
    };
    let code = if let Some(name) = name {
        Code::Named(name.into())
    } else if (0x0100_0030..=0x0100_0052).contains(&key) {
        Code::Named(format!("F{}", key - 0x0100_0030 + 1))
    } else {
        Code::Character(
            char::from_u32(key)
                .ok_or_else(|| format!("旧 Qt 按键 0x{key:x} 没有对应的 Unicode 字符或已核实名称"))?
                .to_lowercase()
                .collect(),
        )
    };
    code.validate()?;
    modifiers &= !code.modifier();
    Ok(Chord {
        key: code,
        modifiers,
    }
    .label())
}
pub(crate) fn modal<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    if !app.shortcuts.open {
        return base;
    }
    let l = app.language;
    let mut bindings = column![].spacing(7);
    for command in Command::ALL {
        if command == Command::ToolSelect {
            bindings = bindings.push(
                text(l.pick(
                    "编辑工具（默认不占用按键，可录入绑定）",
                    "Editing tools (unbound by default; record a binding)",
                ))
                .size(18),
            );
        } else if command == Command::Undo {
            bindings =
                bindings.push(text(l.pick("历史、视图与窗口", "History, views & window")).size(18));
        } else if command == Command::Forward {
            bindings = bindings.push(text(l.pick("相机移动", "Camera movement")).size(18));
        }
        bindings = bindings.push(
            row![
                text(command.label(l)).width(Length::Fixed(205.)),
                text_input("Ctrl+Shift+K; F6", &app.shortcuts.draft[&command])
                    .on_input(move |v| Message::KeyboardBindings(Action::Edit(command, v)))
                    .width(Length::Fill),
                button(l.pick("默认", "Default"))
                    .on_press(Message::KeyboardBindings(Action::Reset(command))),
                button(l.pick("录入", "Record")).on_press(Message::KeyboardBindings(
                    Action::Capture(CaptureTarget::Command(command), false)
                )),
                button(l.pick("追加", "Add")).on_press(Message::KeyboardBindings(Action::Capture(
                    CaptureTarget::Command(command),
                    true
                ))),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        );
    }
    bindings = bindings.push(
        text(l.pick(
            "视图导航（左键保留给工具）",
            "View navigation (left click stays with tools)",
        ))
        .size(18),
    );
    for command in crate::navigation_bindings::Command::ALL {
        bindings = bindings.push(
            row![
                text(command.label(l)).width(Length::Fixed(205.)),
                text_input(
                    "Shift+MouseRight; F6",
                    &app.shortcuts.draft_navigation[&command]
                )
                .on_input(
                    move |value| Message::KeyboardBindings(Action::EditNavigation(command, value))
                )
                .width(Length::Fill),
                button(l.pick("默认", "Default"))
                    .on_press(Message::KeyboardBindings(Action::ResetNavigation(command))),
                button(l.pick("录入", "Record")).on_press(Message::KeyboardBindings(
                    Action::Capture(CaptureTarget::Navigation(command), false)
                )),
                button(l.pick("追加", "Add")).on_press(Message::KeyboardBindings(Action::Capture(
                    CaptureTarget::Navigation(command),
                    true
                ))),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        );
    }
    bindings = bindings.push(
        checkbox(app.shortcuts.draft_sticky)
            .label(l.pick(
                "短按转向输入后保持转向（再次转向或 Escape 退出）",
                "Tap look to keep turning (look again or Escape exits)",
            ))
            .on_toggle(|value| Message::KeyboardBindings(Action::Sticky(value))),
    );
    let content=column![
        text(l.pick("按键与视图控制","Keyboard & view controls")).size(23),
        text(l.pick("组合键用 + 连接，多个按键用分号分隔，留空停用。Escape 保留为取消。","Join modifiers with +; separate alternatives with semicolons; leave blank to disable. Escape cancels.")),
        text(l.pick("例如 Ctrl+K、Shift+F6、ArrowUp。加号键写 Plus，分号键写 Semicolon。文本框保留自己的编辑快捷键。","Examples: Ctrl+K, Shift+F6, ArrowUp. Use Plus or Semicolon for those keys. Text fields retain their own editing shortcuts.")),
        text(l.pick("导航可填 MouseRight、MouseMiddle、MouseBack、MouseForward、WheelUp / WheelDown 或按键；键盘导航作用于光标所在视图。滚轮只用于缩放。","Navigation accepts mouse buttons, WheelUp / WheelDown, or keys; navigation keys use the view under the cursor. Wheel bindings are for zoom.")),
        text(l.pick("飞行动作也可绑定鼠标按钮，并可同时保留键盘替代键。","Flight actions also accept mouse buttons and can keep keyboard alternatives.")),
        text(l.pick("按住导航键时拖动鼠标操作视图。保持转向时左键使用准星位置；触发其他拖动导航先退出保持转向。","Hold a navigation key and move the mouse. Sticky look uses the crosshair for tools; another drag navigation first exits sticky look.")),
        scrollable(bindings).height(Length::Fill),
        text(&app.shortcuts.report),
        row![button(l.pick("应用","Apply")).on_press(Message::KeyboardBindings(Action::Apply)),button(l.pick("取消","Cancel")).on_press(Message::KeyboardBindings(Action::Cancel)),button(l.pick("全部恢复默认","Reset all")).on_press(Message::KeyboardBindings(Action::ResetAll))].spacing(10),
    ].spacing(12);
    let content = container(content)
        .padding(18)
        .style(container::rounded_box)
        .width(Length::Fill)
        .max_width(960)
        .height(Length::Fill)
        .max_height(720);
    let modal = iced::widget::opaque(iced::widget::center(content).padding(16));
    let settings = iced::widget::stack![
        base,
        workspace_tabs::block_dialog(modal.into(), Message::KeyboardBindings(Action::Cancel))
    ]
    .into();
    capture::modal(app, settings)
}
