//! 创建结构目标区块时的显式维度和 biome 配置。
use iced::{
    widget::{checkbox, column, text, text_input},
    Element,
};
use mcedit2_world::{
    chunk_creation::{BiomeSource, ChunkCreationProfile},
    WorldSummary,
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CreationPanel {
    pub enabled: bool,
    pub custom_dimension: bool,
    pub biome: String,
    pub min_y: String,
    pub height: String,
    pub skylight: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum Action {
    Enabled(bool),
    CustomDimension(bool),
    Biome(String),
    MinY(String),
    Height(String),
    Skylight(bool),
}

impl CreationPanel {
    pub fn update(&mut self, action: Action) {
        match action {
            Action::Enabled(v) => self.enabled = v,
            Action::CustomDimension(v) => self.custom_dimension = v,
            Action::Biome(v) => self.biome = v,
            Action::MinY(v) => self.min_y = v,
            Action::Height(v) => self.height = v,
            Action::Skylight(v) => self.skylight = v,
        }
    }

    pub fn profile(
        &self,
        world: &WorldSummary,
        dimension: &str,
    ) -> Result<Option<ChunkCreationProfile>, String> {
        if !self.enabled {
            return Ok(None);
        }
        let biome = self.biome.trim().to_owned();
        let source = if world.data_version == Some(1343) {
            BiomeSource::LegacyNumeric {
                origin: "用户指定的创建合同".into(),
                id: biome
                    .parse()
                    .map_err(|_| "1.12.2 初始 biome 必须为 0..255 的实际数字 ID")?,
            }
        } else {
            BiomeSource::UserSpecified {
                origin: "用户指定的创建合同".into(),
                entries: BTreeSet::from([biome.clone()]),
            }
        };
        let profile = if self.custom_dimension {
            ChunkCreationProfile {
                data_version: world.data_version.ok_or("世界 DataVersion 未知")?,
                min_y: self.min_y.trim().parse().map_err(|_| "最小 Y 必须是整数")?,
                height: self.height.trim().parse().map_err(|_| "高度必须是正整数")?,
                has_skylight: self.skylight,
                dimension_source: format!("user:{dimension}"),
                initial_biome: biome,
                biome_source: source,
            }
        } else {
            ChunkCreationProfile::from_level(world, dimension, biome, source)
                .map_err(|e| e.to_string())?
        };
        profile
            .verify_source(world, dimension)
            .map_err(|e| e.to_string())?;
        Ok(Some(profile))
    }

    pub fn view(&self) -> Element<'_, Action> {
        self.view_with_label("允许创建缺失的区块或区段")
    }
    pub fn view_with_label<'a>(&'a self, label: &'a str) -> Element<'a, Action> {
        let mut body = column![checkbox(self.enabled)
            .label(label)
            .on_toggle(Action::Enabled)]
        .spacing(6);
        if self.enabled {
            body = body
                .push(text("空白区域的初始 biome 必须明确指定"))
                .push(
                    text_input("命名 biome；1.12.2 输入实际数字 ID", &self.biome)
                        .on_input(Action::Biome),
                )
                .push(text(
                    "1.12.2 使用自定义维度合同：最小 Y=0、高度=256，并明确天空光。",
                ))
                .push(
                    checkbox(self.custom_dimension)
                        .label("自定义维度范围")
                        .on_toggle(Action::CustomDimension),
                );
            if self.custom_dimension {
                body = body
                    .push(text_input("最小 Y", &self.min_y).on_input(Action::MinY))
                    .push(text_input("高度", &self.height).on_input(Action::Height))
                    .push(
                        checkbox(self.skylight)
                            .label("有天空光")
                            .on_toggle(Action::Skylight),
                    );
            }
        }
        body.into()
    }
}
