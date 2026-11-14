//! 三版地图来源和实际客户端量化参照，集中并入既有 NBT 流程。
use super::{map_fixture as f, metadata_fixture::Fixture};
use mcedit2_world::{map_data, map_decorations as maps, nbt_document::Value};

pub(super) fn complete() {
    for (version, count) in [(1343, 10), (3465, 27), (3955, 35)] {
        let reference = f::reference(version);
        let catalog = maps::catalog(version).unwrap();
        assert_eq!(catalog.len(), count);
        if version == 3955 {
            for case in reference["projections"].as_array().unwrap() {
                let mut item = f::decorated_item(version, case["type"].as_str().unwrap(), "sample");
                let entry = f::item_decoration(&mut item, version);
                for key in ["x", "z"] {
                    f::change(entry, key, Value::Double(case[key].as_f64().unwrap()));
                }
                f::change(entry, "rotation", Value::Float(90.));
                let mut markers = Vec::new();
                maps::from_item(
                    &mut markers,
                    &item,
                    version,
                    [0, 0],
                    case["scale"].as_i64().unwrap() as i8,
                    case["unlimited"].as_bool().unwrap(),
                )
                .unwrap();
                let expected = case["decorations"].as_array().unwrap();
                assert_eq!(markers.len(), expected.len(), "{case}");
                for (actual, expected) in markers.iter().zip(expected) {
                    assert_eq!(actual.definition.id, expected["type"].as_str().unwrap());
                    assert_eq!(
                        [
                            i64::from(actual.x),
                            i64::from(actual.y),
                            i64::from(actual.rotation)
                        ],
                        [
                            expected["x"].as_i64().unwrap(),
                            expected["y"].as_i64().unwrap(),
                            expected["rotation"].as_i64().unwrap()
                        ],
                        "{case}"
                    );
                }
            }
        }
        for sample in reference["samples"].as_array().unwrap() {
            let data = f::payload(sample);
            let markers = maps::saved(
                &data,
                version,
                [0, 0],
                sample["scale"].as_i64().unwrap() as i8,
            )
            .unwrap();
            assert_eq!(
                markers.len(),
                sample["decorations"].as_array().unwrap().len()
            );
            for (actual, expected) in markers
                .iter()
                .zip(sample["decorations"].as_array().unwrap())
            {
                assert_eq!(
                    &actual.definition,
                    &catalog[expected["type"].as_str().unwrap()]
                );
                assert_eq!(
                    [
                        i64::from(actual.x),
                        i64::from(actual.y),
                        i64::from(actual.rotation)
                    ],
                    [
                        expected["x"].as_i64().unwrap(),
                        expected["y"].as_i64().unwrap(),
                        expected["rotation"].as_i64().unwrap()
                    ]
                );
            }
        }
        for banner in reference["banners"].as_array().unwrap() {
            let data =
                Value::Compound(vec![f::field("banners", f::list(vec![f::payload(banner)]))]);
            let markers = maps::saved(&data, version, [0, 0], 0).unwrap();
            assert_eq!(markers.len(), 1);
            assert_eq!(
                &markers[0].definition,
                &catalog[banner["type"].as_str().unwrap()]
            );
            assert_eq!(
                markers[0].name,
                Some(if version == 3465 {
                    serde_json::json!({"text":"Marker"})
                } else {
                    serde_json::json!("Marker")
                })
            );
        }
        let fixture = Fixture::new(version);
        f::write(&fixture.root, version, f::data(version));
        let before = std::fs::read(fixture.root.join("data/map_7.dat")).unwrap();
        let image = map_data::read(&fixture.root, 7, Some(version)).unwrap();
        assert_eq!(image.center, [0, 0]);
        assert!(image.warnings.is_empty());
        assert_eq!(
            image.decorations.len(),
            if version == 1343 { 0 } else { 17 }
        );
        for (id, definition) in catalog {
            let item = f::decorated_item(version, id, "custom");
            let mut markers = image.decorations.clone();
            maps::from_item(
                &mut markers,
                &item,
                version,
                image.center,
                image.scale,
                image.unlimited_tracking,
            )
            .unwrap();
            let custom = markers.last().unwrap();
            assert_eq!(&custom.definition, definition);
            assert_eq!((custom.x, custom.y, custom.rotation), (20, -19, 12));
            let old = markers.clone();
            maps::from_item(
                &mut markers,
                &item,
                version,
                image.center,
                image.scale,
                image.unlimited_tracking,
            )
            .unwrap();
            assert_eq!(markers, old);
            let mut invalid = item;
            if version == 3955 {
                let Value::Compound(item) = &mut invalid else {
                    unreachable!()
                };
                let Value::Compound(components) = &mut item
                    .iter_mut()
                    .find(|f| f.name == "components")
                    .unwrap()
                    .value
                else {
                    unreachable!()
                };
                let Value::Compound(decorations) = &mut components
                    .iter_mut()
                    .find(|f| f.name == "minecraft:map_decorations")
                    .unwrap()
                    .value
                else {
                    unreachable!()
                };
                f::change(
                    &mut decorations[0].value,
                    "type",
                    Value::String("custom:unregistered".into()),
                );
                assert!(
                    maps::from_item(&mut Vec::new(), &invalid, version, [0, 0], 0, false)
                        .unwrap_err()
                        .contains("custom:unregistered")
                );
            }
        }
        assert_eq!(
            std::fs::read(fixture.root.join("data/map_7.dat")).unwrap(),
            before
        );
    }
}
