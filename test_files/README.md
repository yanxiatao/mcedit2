# 原始格式样本

本目录保留 Minecraft 世界、Region、NBT 和结构文件的数据样本。原 Python 测试套件及旧 `create_testbed.py` 生成器已移除。所有纳入版本管理的样本均只读；读取验证不能改变源字节，写入验证使用独立的临时副本。

- `AnvilWorld/` 和 `AnvilWorld_1.8/` 用于 Java `level.dat` 与 `.mca` Region 发现，包括负 Region 坐标。
- `*.schematic`、`*.nbt`、`indev.mclevel`、`PocketWorld.zip` 等保留为格式语料；保留文件不代表对应格式已完成支持或验收。

Rust 世界测试位于 `rust/crates/mcedit2-world/tests/`。不要把用户的实际存档上传到此公开目录。

`Station.schematic` 的原始署名：作者 **bas_de_paljas**，来源为 [MC Schematics 原帖](http://www.mcschematics.com/index.php?topic=14331.msg91903)。此信息从原 `station.txt` 合并至本说明。
