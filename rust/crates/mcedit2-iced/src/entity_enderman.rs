//! 末影人身体、眼睛及按真实保存方块状态绘制的搬运层。
use super::*;
use mcedit2_world::sections::State;

fn carried(visual: &EntityVisual, bank: &TextureBank) -> Result<Option<State>, String> {
    let version = visual.source_version.ok_or("末影人缺少实际 DataVersion")?;
    if bank.item_data_version() != Some(version) {
        return Err("末影人搬运方块与源资源版本不同".into());
    }
    if version == 1343 {
        let Some(block) = visual.source_field_optional("carried")? else {
            return Ok(None);
        };
        let meta = match visual.source_field_optional("carriedData")? {
            None => 0,
            Some(Value::Short(value)) => value as u16,
            _ => return Err("旧末影人 carriedData 必须为 Short".into()),
        };
        let state = match block {
            Value::Short(0) => return Ok(None),
            Value::Short(id) => bank.legacy_carried_numeric(id, meta)?,
            Value::String(name) if name == "minecraft:air" || name == "air" => return Ok(None),
            Value::String(name) => bank.legacy_carried_name(&name, meta)?,
            _ => return Err("旧末影人 carried 必须为 Short 或注册名 String".into()),
        };
        return Ok(Some(state));
    }
    if !matches!(version, 3465 | 3955) {
        return Err(format!("尚未核实 DataVersion={version} 的末影人搬运格式"));
    }
    let Some(block) = visual.source_field_optional("carriedBlockState")? else {
        return Ok(None);
    };
    if !matches!(&block, Value::Compound(_)) {
        return Err("末影人 carriedBlockState 必须为 Compound".into());
    }
    let mut bytes = vec![10, 0, 0];
    bytes.extend(block.payload().map_err(|error| error.to_string())?);
    let mut state =
        mcedit2_world::sections::decode_block_state(&bytes).map_err(|error| error.to_string())?;
    if !state.name.contains(':') {
        state.name = format!("minecraft:{}", state.name);
    }
    Ok((!matches!(
        state.name.as_str(),
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    ))
    .then_some(state))
}

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let [yaw, pitch] = visual.rotation.ok_or("末影人源实体缺少 Rotation")?;
    let carried = carried(visual, bank)?;
    let layer = "minecraft:enderman#main";
    let mut poses = BTreeMap::new();
    for path in ["/head", "/hat"] {
        poses.insert(
            path.into(),
            PartPose {
                rotation: Some([pitch.to_radians(), 0., 0.]),
                ..Default::default()
            },
        );
    }
    for (path, side) in [("/right_arm", 1.), ("/left_arm", -1.)] {
        poses.insert(
            path.into(),
            PartPose {
                rotation: Some(if carried.is_some() {
                    [-0.5, 0., side * 0.05]
                } else {
                    [0., 0., side * 0.1]
                }),
                ..Default::default()
            },
        );
    }
    let style = npc::RenderStyle {
        yaw,
        groups: None,
        outer: if catalog.version == "1.21.1" {
            npc::scale(visual)?
        } else {
            1.
        },
        tint: [1.; 4],
        ordered: false,
    };
    let mut result = npc::render(
        catalog,
        bank,
        layer,
        "minecraft:entity/enderman/enderman",
        &poses,
        style,
    )?;
    result.extend(eyes::compile(
        catalog,
        bank,
        visual,
        layer,
        eyes::ENDERMAN_TEXTURE,
        &poses,
        style,
    )?);
    if let Some(state) = carried {
        use crate::root_transform::Affine;
        let mut transform = Affine::IDENTITY;
        // 旧 bvo.renderModelBrightness 的最后 Y(90) 与现代 CarriedBlockLayer 中相同的旋转合并。
        for step in [
            serde_json::json!({"origin":"corner","translation":[0.,0.6875,-0.75]}),
            serde_json::json!({"origin":"corner","rotation":{"x":20}}),
            serde_json::json!({"origin":"corner","rotation":{"y":45}}),
            serde_json::json!({"origin":"corner","translation":[0.25,0.1875,0.25]}),
            serde_json::json!({"origin":"corner","scale":[-0.5,-0.5,0.5]}),
            serde_json::json!({"origin":"corner","rotation":{"y":90}}),
        ] {
            transform = transform.compose(Affine::parse(&step)?)?;
        }
        // Affine 接收方块中心坐标；身体模型仍使用原生局部坐标。
        let transform = Affine::translation([0.5; 3]).compose(transform)?;
        for mut quad in crate::spectator::single_block_quads(bank, &state, &|| false)? {
            for point in &mut quad.xyz {
                *point = mcedit2_world::entity_model::living_offset(transform.point(*point), yaw)?
                    .map(|value| value * style.outer);
            }
            quad.mode |= crate::material_mode::VISIBLE_WHEN_INVISIBLE;
            if catalog.version == "1.12.2" {
                quad.mode &= !crate::material_mode::SINGLE_SIDED;
            }
            quad.classify_axis_facing();
            result.push(quad);
        }
    }
    Ok(result)
}
