//! 实体外观分类；位置标记独立控制，隐藏对象不进入模型几何或模型拾取。
use crate::{App, Message};
use iced::{
    widget::{checkbox, column, text},
    Element, Task,
};
use mcedit2_world::scene_annotations::{Kind, Marker};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layer {
    Blocks,
    Entities,
    Items,
    ItemFrames,
    BlockEntities,
}

impl Layer {
    const ENTITIES: [Self; 3] = [Self::Entities, Self::Items, Self::ItemFrames];

    pub(crate) fn label(self, language: crate::language::Language) -> &'static str {
        match self {
            Self::Blocks => language.pick("地形方块", "Terrain blocks"),
            Self::Entities => language.pick("实体与玩家模型", "Entity and player models"),
            Self::Items => language.pick(
                "掉落物、经验球与挂画",
                "Dropped items, experience orbs and paintings",
            ),
            Self::ItemFrames => language.pick("物品展示框及其内容", "Item frames and contents"),
            Self::BlockEntities => language.pick("方块实体模型", "Block entity models"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Visibility {
    #[serde(default = "visible_by_default")]
    blocks: bool,
    entities: bool,
    items: bool,
    item_frames: bool,
    #[serde(default = "visible_by_default")]
    block_entities: bool,
}
fn visible_by_default() -> bool {
    true
}

impl Default for Visibility {
    fn default() -> Self {
        Self {
            blocks: true,
            entities: true,
            items: true,
            item_frames: true,
            block_entities: true,
        }
    }
}

impl Visibility {
    pub(crate) fn layer(self, layer: Layer) -> bool {
        match layer {
            Layer::Blocks => self.blocks,
            Layer::Entities => self.entities,
            Layer::Items => self.items,
            Layer::ItemFrames => self.item_frames,
            Layer::BlockEntities => self.block_entities,
        }
    }

    pub(crate) fn set(&mut self, layer: Layer, enabled: bool) {
        match layer {
            Layer::Blocks => self.blocks = enabled,
            Layer::Entities => self.entities = enabled,
            Layer::Items => self.items = enabled,
            Layer::ItemFrames => self.item_frames = enabled,
            Layer::BlockEntities => self.block_entities = enabled,
        }
    }

    pub(crate) fn allows(self, marker: &Marker) -> bool {
        if marker.block_state.is_some() {
            return self.block_entities;
        }
        match marker.kind {
            Kind::BlockEntity => self.block_entities,
            Kind::Entity | Kind::Player => self.entities,
            Kind::Item
                if marker
                    .id
                    .as_deref()
                    .is_some_and(mcedit2_world::hanging::is_frame) =>
            {
                self.item_frames
            }
            Kind::Item => self.items,
            _ => true,
        }
    }

    pub(crate) fn block_layers(self) -> crate::block_visibility::Layers {
        crate::block_visibility::Layers {
            blocks: self.blocks,
            block_entities: self.block_entities,
        }
    }

    /// 某源段不含受更改分类影响的对象时继续复用其几何。
    pub(crate) fn same_for<'a>(
        self,
        other: Self,
        markers: impl IntoIterator<Item = &'a Marker>,
    ) -> bool {
        markers
            .into_iter()
            .all(|marker| self.allows(marker) == other.allows(marker))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    enabled: bool,
    categories: Visibility,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            enabled: true,
            categories: Visibility::default(),
        }
    }
}

impl Saved {
    pub(crate) fn capture(app: &App) -> Self {
        Self {
            enabled: app.entity_models.enabled,
            categories: app.entity_models.visibility,
        }
    }

    pub(crate) fn set_layer(&mut self, layer: Layer, enabled: bool) {
        self.categories.set(layer, enabled);
    }

    pub(crate) fn install(self, app: &mut App) {
        let previous = app.entity_models.visibility.block_layers();
        app.entity_models.set_display(self.enabled, self.categories);
        if previous != self.categories.block_layers() {
            apply_block_resources(app);
        }
        clear_hidden_selection(app);
    }
}

pub(crate) fn can_display(app: &App, marker: &Marker) -> bool {
    let command_model = !marker.is_command_block()
        || marker.block_state.as_deref().is_some_and(|state| {
            app.scene_texture_bank()
                .is_some_and(|bank| bank.declarative_block_replaces(state).is_some())
        });
    command_model
        && app.entity_models.visibility.allows(marker)
        && (marker.block_state.is_some() || app.entity_models.enabled)
}

pub(crate) fn bank_for_display(
    app: &App,
    bank: crate::textures::TextureBank,
) -> crate::textures::TextureBank {
    bank.with_block_layers(app.entity_models.visibility.block_layers())
}

fn apply_block_resources(app: &mut App) {
    let layers = app.entity_models.visibility.block_layers();
    if let Some(bank) = app.texture_bank.as_ref() {
        app.texture_bank = Some(std::sync::Arc::new(
            (**bank).clone().with_block_layers(layers),
        ));
    }
    crate::keyboard_bindings::interrupt(app);
    // 旧源的遮挡与面可见性属于旧策略，不能借新开关继续显示或拾取。
    app.invalidate_resource_reads();
    if app.far.is_some() {
        // 远景保留正在累积的真实采样及读取队列，只更新其显示资源。
        app.surface_assets = app.texture_bank.clone();
        app.surface_block_layers = layers;
    } else {
        app.surface = None;
        app.surface_assets = None;
    }
    app.resource_reload_attempt = None;
    app.quad.invalidate_resources();
    app.retire_spectator_sources();
    app.surface_hover = None;
    app.world_hover.clear();
    app.selected_block = None;
    app.selected_surface = None;
    app.selected_surface_face = None;
    app.grid_main.cancel();
    app.grid_horizontal.cancel();
    app.grid_vertical.cancel();
    app.grid_revision = app.grid_revision.wrapping_add(1);
}

fn clear_hidden_selection(app: &mut App) {
    if app.scene_layers.selected.as_ref().is_some_and(|selected| {
        !can_display(app, selected.marker())
            && selected.marker().layers() & app.scene_layers.mask == 0
    }) {
        app.scene_layers.selected = None;
        app.scene_layers.command_visuals = None;
    }
}

pub(crate) fn set_enabled(app: &mut App, enabled: bool) {
    let mut saved = Saved::capture(app);
    saved.enabled = enabled;
    saved.install(app);
}

pub(crate) fn update(app: &mut App, layer: Layer, enabled: bool) -> Task<Message> {
    let mut saved = Saved::capture(app);
    saved.set_layer(layer, enabled);
    saved.install(app);
    Task::none()
}

pub(crate) fn controls(app: &App) -> Element<'_, Message> {
    let mut body =
        column![text(app.language.pick("模型分类", "Model categories")).size(14)].spacing(4);
    for layer in Layer::ENTITIES {
        body = body.push(
            checkbox(app.entity_models.visibility.layer(layer))
                .label(layer.label(app.language))
                .on_toggle(move |enabled| Message::EntityModelVisibility(layer, enabled)),
        );
    }
    body.push(text(app.language.pick(
        "位置标记单独控制；关闭分类时名称、装备及展示框内容一同隐藏。",
        "Location markers are separate; names, equipment and frame contents follow the model category.",
    )).size(12)).into()
}

pub(crate) fn block_control(app: &App) -> Element<'_, Message> {
    let mut body = column![].spacing(4);
    for layer in [Layer::Blocks, Layer::BlockEntities] {
        body = body.push(
            checkbox(app.entity_models.visibility.layer(layer))
                .label(layer.label(app.language))
                .on_toggle(move |enabled| Message::EntityModelVisibility(layer, enabled)),
        );
    }
    body.into()
}
