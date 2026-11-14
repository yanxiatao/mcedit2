# 旧版刷怪候选图层依据

原版入口为 `7c13c16^:src/mcedit2/rendering/chunkmeshes/mobspawns.py`。候选格自身不是普通立方体或液体，上方不是普通立方体，下方为普通立方体且排除基岩与屏障；方块光小于 8 时，根据天光是否小于 8 分为红色常暗面、黄色夜间面。使用已保存光照，不根据暂存方块重新推算。缺失的下方区段不提供支撑，缺失的上方区段视为空气；跨区段的支撑同样排除基岩和屏障，修正原脚本的边界遗漏。

这不是完整的生物生成判定。没有模拟玩家距离、难度、生物群系、数量上限或额外顶面规则，也不把 1.12.2 的阈值套到现代版本。UI 明确标记版本和候选含义，未核对的源格式会报告诊断。

原版 `minecraft_raw_1_12.json` 和手工补充没有导出 `normalCube` / `materialLiquid`，不能沿用其默认值或以 `opaqueCube` 猜测。准确官方客户端 SHA-1 为 `0f275bc1547d01fa5f56ba34bdc87d981ee12daf`，SHA-256 为 `8ada07da5ee77dad3527bd7278fbd05ee1fc8a597813b216a871a2d7d64cc64f`。通过 javap 核实 `awu$a.l → aow.s` 为材质不透明、完整立方体且不供电，`bcz.d` 默认 false、液体材质 `bcy.d` 为 true。

`LegacySpawnFlags.java` 直接调用准确客户端 API，初始化注册表但不创建世界或窗口，不使用反射。对 254 种已注册方块分别查询 16 个 metadata；客户端明确拒绝的编码在表中为 null，并保留异常说明，运行时读取这种源状态会正常诊断，不替代为空气。其他异常直接使导出失败。

```powershell
python tools/generate_legacy_spawn.py --reference target/legacy-registry-reference --output crates/mcedit2-world/assets/legacy-spawn-1343.json
```

工具只补齐官方版本元数据中列明且逐 SHA-1 核验的必要 Java 依赖。分析副本只去除签名条目，其余成员逐项与官方 JAR 比较；临时 JAR、编译类和日志配置在结束时清除。有效客户端与依赖缓存保留。此独立参照进程不属于 Rust 产品，也不改变单 debug 配置和单产品入口。

未生成地形层取原版 `terrainpop.py` 的 `TerrainPopulated=0` 语义，旧数组为真实 128 格，旧区段为已核对的 256 格合同。命名状态扩展支持 DataVersion >= 2203 的已知生成阶段，以 `full` / `minecraft:full` 为完成；现代高度读取存档实际维度定义。更早的 Status 状态合同尚未核对，未知阶段会明确报告。
