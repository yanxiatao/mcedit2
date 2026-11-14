# Java 1.21.1 加权模型选择参照

`ModelRandomReference.java` 直接调用已校验的官方客户端坐标种子、LegacyRandomSource、WeightedRandom 区间查询及原版注册表中的方块状态。不使用反射，不启动游戏窗口，不读取或写入世界。参照固定为 1.21.1：客户端 SHA-256 为 `499f6897d1837516680f3114072d8106e11c9adcd933fe5cf051b551089b0c99`；混淆名来自官方 client mappings（SHA-1 `2244b6f072256667bcd9a73df124d6c58de77992`）。

客户端字节码确认：普通 WeightedBakedModel 读取 `Math.abs((int) random.nextLong()) % totalWeight`；MultiPartBakedModel 先读取一个 long，再为每个匹配部分创建使用同一子种子的随机源，不能按部分编号添加盐值。Mth 坐标种子先以 int 计算 X 乘积，再进入 long 运算和有符号右移。LegacyRandomSource 使用 48 位 LCG，nextLong 的两个 int 均按有符号值参与拼接。

参照含 1,578 组坐标 / 权重选择、5 组原始随机边界及 6,312 组原版方块状态种子。坐标包含负数、世界边界和 int 极值；权重覆盖单项、96 项及 int 总权重上界。专门构造的种子使 nextLong 低位成为 MIN_VALUE，直接核对客户端 Math.abs 溢出后的首项选择。原版注册表查出 46 种具有床 / 门 / 双格植物种子规则的方块，遍历其全部状态；不根据模组名称推断 Java 继承或自定义 getSeed 行为。

在 Rust 工作区使用已有 JDK 21 和客户端依赖重新生成：

```powershell
python tools/generate_model_random_reference.py --client-jar $env:MCEDIT2_CLIENT_JAR --version-json target/shulker-reference/version.json --libraries $env:MCEDIT2_LIBRARY_ROOT --output target/model-random-reference/regenerated.json
```

`client_reference.py` 与 UV 参照生成器共用准确版本校验、签名隔离的分析副本及逐成员字节核对；只在工作区 target 下去掉分析副本的签名元数据，原始客户端保持不变。生成记录位于 `target/model-random-reference/evidence.json`；当前输出 SHA-256 为 `345d20e447ecbf936612d8b43a98e77b4ca43b1ced5a219b119ea5911b9a5abc`。

冻结参照为 `crates/mcedit2-iced/tests/fixtures/model-random-1.21.1.json`，由现有模型类别执行，既验证直接选择，也验证预解析模板签名。没有增加独立测试入口。它证明这些 1.21.1 数值与选择场景；其他客户端版本和模组运行时重写的随机方法仍需插件或后续对应版本验证。
