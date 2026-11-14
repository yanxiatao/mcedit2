//! 全局显示图层偏好；运行中的来源、选择和派生快照不进入配置。
use super::State;
use crate::{language::Language, App};
use mcedit2_world::{
    chunk_display::Heightmap,
    scene_annotations::{CommandLayer, Kind, LayerMask},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    mask: LayerMask,
    heightmap: Heightmap,
    inspect: bool,
    controls: bool,
    diagnostics: bool,
    #[serde(default)]
    models: crate::entity_visibility::Saved,
}

impl Saved {
    pub(crate) fn validate(self) -> Result<(), String> {
        let known = Kind::ALL
            .into_iter()
            .fold(CommandLayer::MASK, |mask, kind| mask | kind.bit());
        if self.mask & !known != 0 {
            return Err(format!("显示图层包含未知标志：{:#x}", self.mask & !known));
        }
        Ok(())
    }

    pub(crate) fn capture(app: &App) -> Self {
        let state = &app.scene_layers;
        Self {
            mask: state.mask,
            heightmap: state.heightmap,
            inspect: state.inspect,
            controls: state.show_controls,
            diagnostics: state.show_diagnostics,
            models: crate::entity_visibility::Saved::capture(app),
        }
    }

    /// 桌面读取已核对配置；初次恢复只构造状态，不启动读取或保留旧派生结果。
    pub(crate) fn restore(self) -> State {
        State {
            mask: self.mask,
            heightmap: self.heightmap,
            inspect: self.inspect,
            show_controls: self.controls,
            show_diagnostics: self.diagnostics,
            ..State::default()
        }
    }

    pub(crate) fn restore_models(self, app: &mut App) {
        self.models.install(app);
    }

    pub(crate) fn apply(self, app: &mut App) {
        let state = &mut app.scene_layers;
        let invalidated = (state.mask ^ self.mask) & mcedit2_world::chunk_display::MASK != 0
            || state.heightmap != self.heightmap;
        state.mask = self.mask;
        state.heightmap = self.heightmap;
        state.inspect = self.inspect;
        state.show_controls = self.controls;
        state.show_diagnostics = self.diagnostics;
        if invalidated {
            state.invalidate();
        }
        self.models.install(app);
    }
}

pub(crate) fn set_layer(app: &mut App, kind: Kind, enabled: bool) {
    let mut saved = Saved::capture(app);
    if enabled {
        saved.mask |= kind.bit();
    } else {
        saved.mask &= !kind.bit();
    }
    saved.apply(app);
}

pub(crate) fn set_heightmap(app: &mut App, heightmap: Heightmap) {
    let mut saved = Saved::capture(app);
    saved.heightmap = heightmap;
    saved.apply(app);
}

pub(crate) fn set_command_layer(app: &mut App, layer: CommandLayer, enabled: bool) {
    let mut saved = Saved::capture(app);
    if enabled {
        saved.mask |= layer.bit();
    } else {
        saved.mask &= !layer.bit();
    }
    saved.apply(app);
}

/// 仅包含原 INI 实际写入且已核对语义的图层项；未出现的值保留当前选择。
#[derive(Clone, Debug)]
pub(crate) struct LegacySettings {
    values: Vec<(Kind, bool)>,
    models: Vec<(crate::entity_visibility::Layer, bool)>,
    commands: Vec<(CommandLayer, bool)>,
}

impl LegacySettings {
    pub(crate) fn new(
        values: impl IntoIterator<Item = (Kind, Option<bool>)>,
        models: impl IntoIterator<Item = (crate::entity_visibility::Layer, Option<bool>)>,
        commands: impl IntoIterator<Item = (CommandLayer, Option<bool>)>,
    ) -> Option<Self> {
        let values: Vec<_> = values
            .into_iter()
            .filter_map(|(kind, value)| value.map(|value| (kind, value)))
            .collect();
        let models: Vec<_> = models
            .into_iter()
            .filter_map(|(layer, value)| value.map(|value| (layer, value)))
            .collect();
        let commands: Vec<_> = commands
            .into_iter()
            .filter_map(|(layer, value)| value.map(|value| (layer, value)))
            .collect();
        (!values.is_empty() || !models.is_empty() || !commands.is_empty()).then_some(Self {
            values,
            models,
            commands,
        })
    }

    pub(crate) fn merge(&self, app: &App) -> Result<Saved, String> {
        let mut saved = Saved::capture(app);
        for &(kind, enabled) in &self.values {
            if enabled {
                saved.mask |= kind.bit();
            } else {
                saved.mask &= !kind.bit();
            }
            if kind == Kind::Heightmap && enabled {
                saved.heightmap = Heightmap::Source;
            }
        }
        for &(layer, enabled) in &self.models {
            saved.models.set_layer(layer, enabled);
        }
        for &(layer, enabled) in &self.commands {
            if enabled {
                saved.mask |= layer.bit();
            } else {
                saved.mask &= !layer.bit();
            }
        }
        saved.validate()?;
        Ok(saved)
    }

    pub(crate) fn description(&self, language: Language) -> String {
        self.values
            .iter()
            .map(|&(kind, enabled)| {
                format!(
                    "{} {}",
                    super::label(kind, language),
                    if enabled {
                        language.pick("显示", "on")
                    } else {
                        language.pick("隐藏", "off")
                    }
                )
            })
            .chain(self.models.iter().map(|&(layer, enabled)| {
                format!(
                    "{} {}",
                    layer.label(language),
                    if enabled {
                        language.pick("显示", "on")
                    } else {
                        language.pick("隐藏", "off")
                    }
                )
            }))
            .chain(self.commands.iter().map(|&(layer, enabled)| {
                format!(
                    "{} {}",
                    super::command_label(layer, language),
                    if enabled {
                        language.pick("显示", "on")
                    } else {
                        language.pick("隐藏", "off")
                    }
                )
            }))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}
