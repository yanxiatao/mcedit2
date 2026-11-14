//! 将状态条件与权重区间解析一次，按坐标保留相同随机种子和选中模型签名。
use super::*;
use std::collections::hash_map::DefaultHasher;

pub(crate) struct VariantPlan {
    seed: random::SeedPosition,
    choices: Vec<Choice>,
    fixed: Option<u64>,
    legacy: bool,
}
struct Choice {
    multipart: bool,
    total: u32,
    options: Vec<(u32, Option<ChosenModel>)>,
}
impl VariantPlan {
    pub(super) fn new(inputs: Vec<(&Value, bool)>, state: &State, legacy: bool) -> Option<Self> {
        let seed = random::SeedPosition::new(state);
        let choices = inputs
            .into_iter()
            .map(|(value, multipart)| {
                let Some(_) = value.as_array() else {
                    random::weight(value)?;
                    return Some(Choice {
                        multipart,
                        total: 1,
                        options: vec![(1, ChosenModel::parse(value))],
                    });
                };
                let array = random::ordered_options(value, legacy)?;
                if array.is_empty() {
                    return None;
                }
                let mut total = 0u32;
                let mut options = Vec::with_capacity(array.len());
                for item in array.iter() {
                    let weight = random::weight(item)?;
                    total = total.checked_add(weight)?;
                    if total > i32::MAX as u32 {
                        return None;
                    }
                    // 权重格式错误使整组无效；模型描述错误只影响实际抽中它的坐标。
                    options.push((total, ChosenModel::parse(item)));
                }
                Some(Choice {
                    multipart,
                    total,
                    options,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let mut plan = Self {
            seed,
            choices,
            fixed: None,
            legacy,
        };
        if !plan.position_dependent() {
            plan.fixed = Some(plan.at((0, 0, 0)));
        }
        Some(plan)
    }
    pub(crate) fn position_dependent(&self) -> bool {
        self.choices.iter().any(|choice| choice.options.len() > 1)
    }
    pub(crate) fn at(&self, pos: (i32, i32, i32)) -> u64 {
        if let Some(key) = self.fixed {
            return key;
        }
        let mut signature = DefaultHasher::new();
        // 与 Vec<ChosenModel>::hash 相同的长度前缀及逐元素 Hash 顺序。
        self.choices.len().hash(&mut signature);
        let mut multipart_index = 0_i64;
        for choice in &self.choices {
            let seed = if self.legacy {
                let seed = random::position_seed_legacy([pos.0, pos.1, pos.2]);
                if choice.multipart {
                    let seed = seed.wrapping_add(multipart_index);
                    multipart_index += 1;
                    seed
                } else {
                    seed
                }
            } else {
                self.seed.at(pos, choice.multipart)
            };
            let option = if choice.options.len() == 1 {
                &choice.options[0].1
            } else {
                let target = if self.legacy {
                    random::legacy_weighted_ticket(seed, choice.total)
                } else {
                    random::weighted_ticket(seed, choice.total)
                };
                &choice
                    .options
                    .iter()
                    .find(|(end, _)| target < *end)
                    .expect("完整正权重区间")
                    .1
            };
            let Some(model) = option else {
                return 0;
            };
            model.hash(&mut signature);
        }
        signature.finish()
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared_choice_keys_match_direct_selection_for_multipart_and_invalid_candidates() {
        let state = State {
            name: "demo:machine".into(),
            properties: vec![("loaded".into(), "true".into())],
        };
        let cases = [
            serde_json::json!({"model":"demo:block/base"}),
            serde_json::json!([{"model":"demo:block/a","weight":2},{"model":"demo:block/b","weight":3,"y":90,"uvlock":true}]),
            serde_json::json!([{"model":"demo:block/a"},{"model":"../invalid"}]),
            serde_json::json!([]),
            serde_json::json!([{"model":"demo:block/a","weight":0},{"model":"demo:block/b"}]),
        ];
        for value in &cases {
            for inputs in [vec![(value, false)], vec![(&cases[0], true), (value, true)]] {
                let plan = VariantPlan::new(inputs.clone(), &state, false);
                for n in -128..128 {
                    let pos = (n, -64, n * 13);
                    let selected = inputs
                        .iter()
                        .map(|(value, multipart)| {
                            ChosenModel::parse(choose_weighted(
                                value,
                                deterministic_model_seed(&state, pos, *multipart),
                            )?)
                        })
                        .collect::<Option<Vec<_>>>();
                    let expected = selected.map_or(0, |chosen| {
                        let mut h = DefaultHasher::new();
                        chosen.hash(&mut h);
                        h.finish()
                    });
                    assert_eq!(plan.as_ref().map_or(0, |plan| plan.at(pos)), expected);
                }
            }
        }
        let empty = VariantPlan::new(vec![], &state, false).unwrap();
        let mut hash = DefaultHasher::new();
        Vec::<ChosenModel>::new().hash(&mut hash);
        assert_eq!(empty.at((1, 2, 3)), hash.finish());
        // 参照由官方客户端直接生成，涵盖坐标溢出、Legacy RNG、权重边界及全部种子覆盖类。
        let oracle: Value =
            serde_json::from_str(include_str!("../tests/fixtures/model-random-1.21.1.json"))
                .unwrap();
        for case in oracle["random"].as_array().unwrap() {
            let p: [i32; 3] = serde_json::from_value(case["pos"].clone()).unwrap();
            let pos = (p[0], p[1], p[2]);
            assert_eq!(random::position_seed(p), case["seed"].as_i64().unwrap());
            let mut rng = crate::java_random::LegacyRandom::new(case["seed"].as_i64().unwrap());
            assert_eq!(rng.next_long(), case["next_long"].as_i64().unwrap());
            let values=Value::Array(case["weights"].as_array().unwrap().iter().enumerate().map(|(i,w)| {
                serde_json::json!({"model":format!("demo:block/variant_{i}"),"weight":w})
            }).collect());
            for (mode, multipart) in [("direct", false), ("multipart", true)] {
                let expected =
                    ChosenModel::parse(&values[case[mode].as_u64().unwrap() as usize]).unwrap();
                let seed = deterministic_model_seed(&state, pos, multipart);
                assert_eq!(
                    ChosenModel::parse(choose_weighted(&values, seed).unwrap())
                        .unwrap()
                        .key,
                    expected.key
                );
                let plan = VariantPlan::new(vec![(&values, multipart)], &state, false).unwrap();
                let mut h = DefaultHasher::new();
                vec![expected].hash(&mut h);
                assert_eq!(plan.at(pos), h.finish());
            }
        }
        for case in oracle["special"].as_array().unwrap() {
            let seed = case["seed"].as_i64().unwrap();
            assert_eq!(
                crate::java_random::LegacyRandom::new(seed).next_long(),
                case["next_long"].as_i64().unwrap()
            );
            let ticket = random::weighted_ticket(seed, 10);
            let chosen = if ticket < 2 {
                0
            } else if ticket < 5 {
                1
            } else {
                2
            };
            assert_eq!(chosen, case["choice"].as_i64().unwrap());
        }
        for case in oracle["block_seeds"].as_array().unwrap() {
            let source = State {
                name: case["name"].as_str().unwrap().into(),
                properties: serde_json::from_value::<BTreeMap<String, String>>(
                    case["properties"].clone(),
                )
                .unwrap()
                .into_iter()
                .collect(),
            };
            let p: [i32; 3] = serde_json::from_value(case["pos"].clone()).unwrap();
            assert_eq!(
                random::SeedPosition::new(&source).at((p[0], p[1], p[2]), false),
                case["seed"].as_i64().unwrap(),
                "{} {:?} {:?}",
                source.name,
                source.properties,
                p
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Models {
            prepared_choice_keys_match_direct_selection_for_multipart_and_invalid_candidates();
        }
    }
}
