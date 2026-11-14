# Java 1.21.1 UV 锁定数值参照

`UvLockReference.java` 直接调用本机官方客户端的 `BlockModelRotation.getRotation` 和 `FaceBakery.recomputeUVs`，只输出数值，不运行游戏或接触存档，不使用反射。固定混淆名来自官方 client mappings（SHA-1 `2244b6f072256667bcd9a73df124d6c58de77992`）。客户端 SHA-1 必须为 `30c73b1c5da787909b2f73340419fdf13b9def88`。十种根矩阵覆盖原点、旋转、缩放、镜像、平移、剪切和 32 度旋转；全部 16 组状态旋转、6 个面、4 种作者 UV / 旋转产生 3,840 组独立结果。

使用已有的 JDK 21、Python 和该客户端的安装依赖，在 Rust 工作区执行：

```powershell
python tools/generate_uvlock_reference.py --client-jar $env:MCEDIT2_CLIENT_JAR --version-json target/shulker-reference/version.json --libraries $env:MCEDIT2_LIBRARY_ROOT --output target/uvlock-reference/regenerated.json
```

`version.json` 是 Mojang 版本元数据，包含 `id=1.21.1`、下载摘要和依赖列表；Prism 的补丁元数据格式不同，不能直接代用。当前工作区已按 Mojang 官方版本列表的 SHA-1 校验该文件，工具本身不下载依赖。

客户端混淆类位于有签名的默认包，未签名测试入口无法与其直接共用包。脚本只在 `target/uvlock-reference/client-analysis.jar` 分析副本中去掉两个签名元数据成员，并逐项验证其余所有成员字节与原件一致；不改原客户端，不分发分析副本。`javac -proc:none` 禁用无关注解处理器。`evidence.json` 记录原件 SHA-256、排除成员、数值结果 SHA-256 和用例数。

默认 Rust 测试直接读取冻结的 `crates/mcedit2-iced/tests/fixtures/uvlock-1.21.1.json`，无需 JDK、Minecraft 安装或联网。重新生成后须核对差异与来源，不得根据 Rust 输出反写期望值。

NeoForge 21.1.234 的实际 `ElementsModel` 字节码确认：根变换通过 `composeRootTransformIntoModelState` 合并，再交给原版面烘焙。参考 [ElementsModel](https://github.com/neoforged/NeoForge/blob/1.21.1/src/main/java/net/neoforged/neoforge/client/model/ElementsModel.java) 与 [UnbakedGeometryHelper](https://github.com/neoforged/NeoForge/blob/1.21.1/src/main/java/net/neoforged/neoforge/client/model/geometry/UnbakedGeometryHelper.java)；分支源码可能变化，当前验收以本机已核对摘要的 21.1.234 universal JAR 为准。
