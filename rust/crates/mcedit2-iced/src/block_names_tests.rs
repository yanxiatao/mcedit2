use super::*;
use crate::{
    language::Language,
    textures::{java_obj_tests::Fixture, TextureBank},
};
use std::io::Write;

fn directory_and_zip_language_layers_override_only_matching_keys_and_locales() {
    let base = crate::canvas_raster::tests::fixture(1, 1, &[255; 4]);
    base.put(
        "assets/test/lang/zh_cn.json",
        r#"{"block.test.tile":"测试方块","block.test.nested.machine":"机器"}"#.as_bytes(),
    );
    base.put(
        "assets/test/lang/en_us.json",
        br#"{"block.test.tile":"Test Block"}"#,
    );
    let upper = Fixture::new();
    let archive = upper.path.join("language.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    zip.start_file(
        "assets/test/lang/zh_cn.json",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(r#"{"block.test.tile":"覆盖名称"}"#.as_bytes())
        .unwrap();
    zip.finish().unwrap();
    let bank = TextureBank::open_stack_java(&[base.path.clone(), archive]).unwrap();
    assert_eq!(
        bank.block_name("test:tile", Language::Chinese),
        Some("覆盖名称")
    );
    assert_eq!(
        bank.block_name("test:tile", Language::English),
        Some("Test Block")
    );
    assert_eq!(
        bank.block_name("test:nested/machine", Language::Chinese),
        Some("机器")
    );
    assert_eq!(bank.block_name("test:missing", Language::Chinese), None);
    assert_eq!(
        bank.block_name("test:nested/machine", Language::English),
        None
    );
}

fn malformed_language_overlay_is_diagnosed_without_partially_overwriting_names() {
    let base = crate::canvas_raster::tests::fixture(1, 1, &[255; 4]);
    base.put(
        "assets/test/lang/zh_cn.json",
        r#"{"block.test.tile":"原名称"}"#.as_bytes(),
    );
    let upper = Fixture::new();
    upper.put(
        "assets/test/lang/zh_cn.json",
        br#"{"block.test.tile":"changed","bad":}"#,
    );
    let bank = TextureBank::open_stack_java(&[base.path.clone(), upper.path.clone()]).unwrap();
    assert_eq!(
        bank.block_name("test:tile", Language::Chinese),
        Some("原名称")
    );
    assert!(bank
        .warnings
        .iter()
        .any(|warning| warning.contains("语言资源") && warning.contains("zh_cn.json")));
    let mut names = Names::default();
    names
        .insert(
            "assets/minecraft/lang/zh_CN.lang",
            "\u{feff}# 注释\ntile.stone.name=石头\n".as_bytes(),
        )
        .unwrap();
    assert_eq!(
        names.name("minecraft:stone", Language::Chinese),
        Some("石头")
    );
    assert!(names
        .insert(
            "assets/minecraft/lang/zh_CN.lang",
            b"tile.stone.name=partial\nbad line"
        )
        .is_err());
    assert_eq!(
        names.name("minecraft:stone", Language::Chinese),
        Some("石头")
    );
}

fn rich_text_entries_preserve_plain_names_and_unresolved_overrides_do_not_resurrect_old_values() {
    let mut names = Names::default();
    names
        .insert(
            "assets/test/lang/zh_cn.json",
            r#"{"block.test.dynamic":"旧值"}"#.as_bytes(),
        )
        .unwrap();
    let diagnostics=names.insert("assets/test/lang/zh_cn.json",r#"{"block.test.tile":"机器","block.test.rich":["前缀",{"text":"名称","extra":[{"text":"！","color":"red"}]}],"block.test.dynamic":[{"index":0}],"item.test.number":42}"#.as_bytes()).unwrap();
    assert_eq!(names.name("test:tile", Language::Chinese), Some("机器"));
    assert_eq!(
        names.name("test:rich", Language::Chinese),
        Some("前缀名称！")
    );
    assert_eq!(names.name("test:number", Language::Chinese), Some("42"));
    assert_eq!(names.name("test:dynamic", Language::Chinese), None);
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].contains("block.test.dynamic"));
}

fn client_language_reads_only_the_asset_index_named_by_its_version_metadata() {
    use std::io::Read;
    let f = Fixture::new();
    let hash = "0123456789012345678901234567890123456789";
    let other = "1123456789012345678901234567890123456789";
    let bytes = r#"{"block.minecraft.stone":"正确版本"}"#.as_bytes();
    f.put(
        "meta/net.minecraft/1.21.1.json",
        br#"{"assetIndex":{"id":"correct"}}"#,
    );
    f.put(
        "assets/indexes/correct.json",
        &serde_json::to_vec(&serde_json::json!({"objects":{"minecraft/lang/zh_cn.json":{"hash":hash,"size":bytes.len()}}})).unwrap(),
    );
    f.put(
        "assets/indexes/other.json",
        &serde_json::to_vec(
            &serde_json::json!({"objects":{"minecraft/lang/zh_cn.json":{"hash":other,"size":0}}}),
        )
        .unwrap(),
    );
    f.put(&format!("assets/objects/01/{hash}"), bytes);
    f.put(
        &format!("assets/objects/11/{other}"),
        br#"{"block.minecraft.stone":"wrong"}"#,
    );
    let path = f
        .path
        .join("libraries/com/mojang/minecraft/1.21.1/minecraft-1.21.1-client.jar");
    let index = crate::client_assets::discover(&path, None)
        .unwrap()
        .unwrap();
    let files = index
        .objects
        .into_iter()
        .filter(|(name, _)| locale_asset(name).is_some())
        .map(|(name, object)| {
            let mut bytes = Vec::new();
            object.open().unwrap().read_to_end(&mut bytes).unwrap();
            (name, bytes)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        files,
        vec![("assets/minecraft/lang/zh_cn.json".into(), bytes.to_vec())]
    );
    f.put(
        "meta/net.minecraft/1.21.1.json",
        br#"{"assetIndex":{"id":"../other"}}"#,
    );
    assert!(crate::client_assets::discover(&path, None).is_err());
}

fn source_pick_name_switches_language_immediately_and_preserves_original_state() {
    let (mut app, _, _, request) = crate::picking_integration_tests::fixture();
    let original = request.hit.owner.as_ref().unwrap().clone();
    let f = crate::canvas_raster::tests::fixture(1, 1, &[255; 4]);
    let (namespace, path) = original.state.name.split_once(':').unwrap();
    let key = format!("block.{namespace}.{}", path.replace('/', "."));
    f.put(
        &format!("assets/{namespace}/lang/zh_cn.json"),
        serde_json::to_string(&serde_json::json!({key.clone():"源方块"}))
            .unwrap()
            .as_bytes(),
    );
    f.put(
        &format!("assets/{namespace}/lang/en_us.json"),
        serde_json::to_string(&serde_json::json!({key:"Source Block"}))
            .unwrap()
            .as_bytes(),
    );
    app.texture_bank = Some(f.open());
    app.pick_spectator(request);
    app.language = Language::Chinese;
    let chinese = app.block_name_info().unwrap();
    assert!(chinese.starts_with("名称：源方块\n") && chinese.contains(&original.state.name));
    app.language = Language::English;
    assert!(app
        .block_name_info()
        .unwrap()
        .starts_with("Name: Source Block\n"));
    assert_eq!(app.selected_block.as_ref().unwrap().state, *original.state);
    assert_eq!(app.current_session((0, 0)).unwrap().pending_cells(), 0);
    app.query_coordinates = "300,200,100".into();
    assert!(
        app.block_name_info().is_none(),
        "坐标草稿不能套用旧选点的名称"
    );
}

#[test]
fn names_behavior() {
    directory_and_zip_language_layers_override_only_matching_keys_and_locales();
    malformed_language_overlay_is_diagnosed_without_partially_overwriting_names();
    rich_text_entries_preserve_plain_names_and_unresolved_overrides_do_not_resurrect_old_values();
    client_language_reads_only_the_asset_index_named_by_its_version_metadata();
    source_pick_name_switches_language_immediately_and_preserves_original_state();
}
