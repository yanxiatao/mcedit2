# 方块实体注册依据

## 1.12.2

输入为 Mojang [1.12.2 元数据](https://piston-meta.mojang.com/v1/packages/832d95b9f40699d4961394dcf6cf549e65f15dc5/1.12.2.json) 中声明的[官方客户端](https://piston-data.mojang.com/v1/objects/0f275bc1547d01fa5f56ba34bdc87d981ee12daf/client.jar)。客户端 SHA-1 为 `0f275bc1547d01fa5f56ba34bdc87d981ee12daf`，SHA-256 为 `8ada07da5ee77dad3527bd7278fbd05ee1fc8a597813b216a871a2d7d64cc64f`；本机统一复用 PrismLauncher 的准确客户端，路径为 `E:/Users/chexson/AppData/Local/Programs/PrismLauncher/libraries/com/mojang/minecraft/1.12.2/minecraft-1.12.2-client.jar`。

`javap -p -c` 静态读取 `avj` 的 25 个类型注册和 `aow` 的数字方块注册，再沿 48 个方块的实际工厂及父类查出所构造的实体类。全部数字 ID 和 `hasEntity` 覆盖与原 MCEdit2 `9fd9e91` 的 `minecraft_raw_1_12.json` / `idmapping_raw_1_12.json` 一致。过程未使用反射或初始化游戏类；不是一次游戏运行验收。

Amulet/PyMCTranslate build 348 的 1.12.2 规格用于交叉核对，最终关系服从客户端字节码：普通箱子与陷阱箱都是 `api` 工厂，构造 `avl`，注册 ID 同为 `minecraft:chest`；不接受翻译器为陷阱箱标注的较新 `minecraft:trapped_chest` 分型。结构方块由 `aul` 构造已注册的 `awe`，也纳入完整表。

同一客户端的 `api` / `aqk` 的 `a(int)` 通过 `fa.a(int)` 按六方向索引取余，竖直轴统一转为 north；`e(awt)` 输出 `fa.a()` 的标准索引 north=2、south=3、west=4、east=5。编辑旋转据此解码 0–15 并写回标准朝向，原状态未变时保留原 metadata；渲染器共用此解码合同，模型和 variant 继续取原 BlockDumper 的对应方向绑定。该依据只适用于准确 1.12.2 的普通 / 陷阱 / 末影箱，未推断其他旧版别名。

在 `rust/` 下使用已有 JDK 重新生成：

```powershell
python tools/generate_legacy_be_registry.py --client-jar E:/Users/chexson/AppData/Local/Programs/PrismLauncher/libraries/com/mojang/minecraft/1.12.2/minecraft-1.12.2-client.jar --reference 9fd9e91 --output crates/mcedit2-world/assets/block-entity-types-1343.json
```

生成器按准确客户端摘要、完整类型覆盖、数字 ID 及单一工厂结果核对，遇到差异直接报错。产物同时列出命名方块和 `numeric:<id>`，用于结构导入和方块差异应用中的载荷兼容判断；其他版本、未知模组和旧非命名 BE 别名没有由此获得推断兼容。

## 1.20.1

剪贴板集中验证暴露 3465 未接入注册关系，导致普通箱子载荷导入被拒绝。新增数据直接读取 Mojang [1.20.1 元数据](https://piston-meta.mojang.com/v1/packages/fff64d6821a97cf12ce5f16df58dd5db0ed210b3/1.20.1.json) 声明的官方客户端及 `client_mappings`：客户端 SHA-1 `0c3ec587af28e5a785c0b4a7b8a30f9a8f78f838`，映射 SHA-1 `6c48521eed01fe2e8ecdadbd5ae348415f3c47da`。本机复用 PrismLauncher 已有客户端，没有复制 JAR 或初始化游戏类。

`generate_be_registry_1201.py` 用 `javap -p -c` 读取 `czp` 的全部静态类型注册和方块数组，以同一官方映射解析 `cpo` 字段，并逐一核对 `Blocks` 实际注册 ID。提取结果完整覆盖官方映射中的 41 个类型、163 个方块；数组写入数、重复项与类型字段全部核对。1.20.1 保持普通 / 陷阱箱分型，包含樱花 / 竹制告示牌、悬挂告示牌、考古方块及饰纹陶罐；没有借用 1.21.1 的新增类型。来源摘要随 `block-entity-types-3465.json` 保存。

```powershell
python tools/generate_be_registry_1201.py --client-jar <准确客户端路径> --mappings target/be-registry-reference-1.20.1/client_mappings.txt --javap <已有JDK的javap路径> --output crates/mcedit2-world/assets/block-entity-types-3465.json
```

静态提取不会读取运行时的模组注册表，也不把其他版本视为已核实。完整剪贴板流程在既有 NBT 综合类别验证，结果见 `../PROGRESS.md`。
