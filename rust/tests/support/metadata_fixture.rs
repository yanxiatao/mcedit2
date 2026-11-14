//! 世界 / 桌面共用的真实 gzip NBT 来源，只创建独立临时目录。
use mcedit2_world::{
    nbt_document::{Field, List, Value},
    WorldSummary,
};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static IDS: AtomicU64 = AtomicU64::new(0);
pub fn field(name: &str, value: Value) -> Field {
    Field {
        name: name.into(),
        value,
    }
}
pub fn nbt(fields: Vec<Field>) -> Vec<u8> {
    let mut bytes = vec![10, 0, 0];
    bytes.extend(Value::Compound(fields).payload().unwrap());
    bytes
}
pub fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}
pub fn player(version: i32, id: i32, dimension: &str) -> Vec<Field> {
    let modern = version >= 3837;
    let mut fields = vec![
        field("DataVersion", Value::Int(version)),
        field("Name", Value::String(format!("玩家{id}"))),
        field(
            "Dimension",
            if version == 1343 {
                Value::Int(if dimension == "minecraft:the_nether" {
                    -1
                } else {
                    0
                })
            } else {
                Value::String(dimension.into())
            },
        ),
        field(
            "Pos",
            Value::List(List {
                element_type: 6,
                items: vec![
                    Value::Double(-32.5),
                    Value::Double(70.),
                    Value::Double(18.25),
                ],
            }),
        ),
        field(
            "Rotation",
            Value::List(List {
                element_type: 5,
                items: vec![Value::Float(42.), Value::Float(-13.)],
            }),
        ),
        field("Health", Value::Float(20.)),
        field("Air", Value::Short(300)),
        field("AbsorptionAmount", Value::Float(2.)),
        field("foodLevel", Value::Int(20)),
        field("foodSaturationLevel", Value::Float(5.)),
        field("foodExhaustionLevel", Value::Float(0.)),
        field("SelectedItemSlot", Value::Int(0)),
        field("playerGameType", Value::Int(1)),
        field("Invulnerable", Value::Byte(0)),
        field("OnGround", Value::Byte(1)),
        field("XpLevel", Value::Int(10)),
        field("XpTotal", Value::Int(123)),
        field("XpSeed", Value::Int(i32::MAX)),
        field("XpP", Value::Float(0.5)),
        field(
            "abilities",
            Value::Compound(vec![
                field("flying", Value::Byte(0)),
                field("mayfly", Value::Byte(1)),
                field("flySpeed", Value::Float(0.05)),
            ]),
        ),
        field(
            "Inventory",
            Value::List(List {
                element_type: 10,
                items: vec![Value::Compound(vec![
                    field("Slot", Value::Byte(0)),
                    field("id", Value::String("minecraft:stone".into())),
                    field(
                        if modern { "count" } else { "Count" },
                        if modern {
                            Value::Int(64)
                        } else {
                            Value::Byte(64)
                        },
                    ),
                    field(
                        if modern { "components" } else { "tag" },
                        Value::Compound(vec![field(
                            if modern {
                                "minecraft:custom_data"
                            } else {
                                "ModPayload"
                            },
                            Value::Compound(vec![field(
                                "Opaque",
                                Value::ByteArray(vec![-1, 0, 127, -128]),
                            )]),
                        )]),
                    ),
                ])],
            }),
        ),
        field(
            "EnderItems",
            Value::List(List {
                element_type: 10,
                items: vec![],
            }),
        ),
        field("OpaquePlayer", Value::ByteArray(vec![-128, 0, 127, -1])),
    ];
    if version == 1343 {
        fields.push(field("UUIDMost", Value::Long(0)));
        fields.push(field("UUIDLeast", Value::Long(i64::from(id))));
    } else {
        fields.push(field("UUID", Value::IntArray(vec![0, 0, 0, id])));
    }
    fields
}
pub struct Fixture {
    pub root: PathBuf,
    pub world: WorldSummary,
    pub level: Vec<u8>,
}
impl Fixture {
    pub fn new(version: i32) -> Self {
        let root = std::env::temp_dir().join(format!(
            "mcedit2-metadata-{}-{}",
            std::process::id(),
            IDS.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        fs::create_dir(root.join("region")).unwrap();
        fs::create_dir_all(root.join("DIM-1/region")).unwrap();
        fs::create_dir(root.join("playerdata")).unwrap();
        fs::create_dir(root.join("players")).unwrap();
        let level = gzip(&nbt(vec![field(
            "Data",
            Value::Compound(vec![
                field("DataVersion", Value::Int(version)),
                field("version", Value::Int(19133)),
                field(
                    "Version",
                    Value::Compound(vec![
                        field("Id", Value::Int(version)),
                        field(
                            "Name",
                            Value::String(
                                match version {
                                    1343 => "1.12.2",
                                    3465 => "1.20.1",
                                    3955 => "1.21.1",
                                    _ => panic!("未定义版本"),
                                }
                                .into(),
                            ),
                        ),
                    ]),
                ),
                field("LevelName", Value::String("元数据验证世界".into())),
                field("RandomSeed", Value::Long(i64::MAX)),
                field("Time", Value::Long(48000)),
                field("DayTime", Value::Long(48001)),
                field("Difficulty", Value::Byte(2)),
                field("GameType", Value::Int(1)),
                field("hardcore", Value::Byte(0)),
                field("DifficultyLocked", Value::Byte(0)),
                field("allowCommands", Value::Byte(1)),
                field("SpawnX", Value::Int(0)),
                field("SpawnY", Value::Int(70)),
                field("SpawnZ", Value::Int(0)),
                field(
                    "GameRules",
                    Value::Compound(vec![field("doDaylightCycle", Value::String("true".into()))]),
                ),
                field(
                    "Player",
                    Value::Compound(player(version, 1, "minecraft:overworld")),
                ),
                field("OpaqueWorld", Value::LongArray(vec![i64::MIN, i64::MAX])),
            ]),
        )]));
        fs::write(root.join("level.dat"), &level).unwrap();
        for (id, dimension) in [(1, "minecraft:overworld"), (2, "minecraft:the_nether")] {
            fs::write(
                root.join(format!("playerdata/00000000-0000-0000-0000-{id:012x}.dat")),
                gzip(&nbt(player(version, id, dimension))),
            )
            .unwrap();
        }
        fs::write(
            root.join("players/Alex.dat"),
            gzip(&nbt(player(version, 1, "minecraft:overworld"))),
        )
        .unwrap();
        let world = mcedit2_world::inspect_world(&root).unwrap();
        Self { root, world, level }
    }
    pub fn add_chunk(&mut self) {
        use mcedit2_world::chunk_creation::{empty_chunk, BiomeSource, ChunkCreationProfile};
        let version = self.world.data_version.unwrap();
        assert!(matches!(version, 3465 | 3955));
        let profile = ChunkCreationProfile {
            data_version: version,
            min_y: 0,
            height: 16,
            has_skylight: true,
            dimension_source: "user:元数据验证".into(),
            initial_biome: "minecraft:plains".into(),
            biome_source: BiomeSource::UserSpecified {
                origin: "元数据验证".into(),
                entries: ["minecraft:plains".into()].into_iter().collect(),
            },
        };
        let raw = empty_chunk(&profile, (0, 0)).unwrap();
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        let payload = encoder.finish().unwrap();
        let sectors = (payload.len() + 5).div_ceil(4096);
        let mut region = vec![0u8; (2 + sectors) * 4096];
        region[..4].copy_from_slice(&((2u32 << 8) | sectors as u32).to_be_bytes());
        region[8192..8196].copy_from_slice(&((payload.len() + 1) as u32).to_be_bytes());
        region[8196] = 2;
        region[8197..8197 + payload.len()].copy_from_slice(&payload);
        fs::write(self.root.join("region/r.0.0.mca"), region).unwrap();
        self.world = mcedit2_world::inspect_world(&self.root).unwrap();
    }
    pub fn image(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, PNG).unwrap();
        path
    }
    pub fn file(&self, path: &str) -> Vec<u8> {
        fs::read(self.root.join(path)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // 递归清理前确认绝对目标仍是本次创建的临时根，且未被换成链接。
        let expected = std::env::temp_dir().canonicalize().expect("临时目录可解析");
        assert!(self.root.starts_with(&expected));
        assert!(self
            .root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(&format!("mcedit2-metadata-{}-", std::process::id())));
        let metadata = fs::symlink_metadata(&self.root).expect("测试目录应存在");
        assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
        assert_eq!(self.root.canonicalize().unwrap(), self.root);
        if let Err(error) = fs::remove_dir_all(&self.root) {
            eprintln!("清理元数据验证目录失败：{error}");
        }
    }
}
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 0,
    5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

/// 为选区高度验证写入实际维度定义；保留夹具其他 level.dat 字段。
pub fn define_overworld(fixture: &mut Fixture) {
    use mcedit2_world::nbt_document::{Document, Operation};
    use std::io::Read;
    let bytes = fs::read(fixture.root.join("level.dat")).unwrap();
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes.as_slice())
        .read_to_end(&mut raw)
        .unwrap();
    let mut doc = Document::new(raw).unwrap();
    doc.apply(&Operation::InsertField {
        path: "/Data".into(),
        name: "WorldGenSettings".into(),
        value: Value::Compound(vec![field(
            "dimensions",
            Value::Compound(vec![field(
                "minecraft:overworld",
                Value::Compound(vec![field(
                    "type",
                    Value::String("minecraft:overworld".into()),
                )]),
            )]),
        )]),
    })
    .unwrap();
    fs::write(fixture.root.join("level.dat"), gzip(doc.bytes())).unwrap();
    fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
}
