# Rust 验证

测试使用标准 Rust harness，按独立行为命名。入口数量不设上限；不再维护 Category 枚举或逐模块分派。历史类别结果保留在 PROGRESS.md，不能作为当前提交的验证结果。

## 开发与基本验证

Linux 需要 Rust stable、C/C++ 编译器、pkg-config、libffi-dev 和平台图形库（CI 清单见 .github/workflows/rust-iced.yml）。在 rust/ 执行，使用仓库的同一套 dev 配置和 target/debug 缓存：

```sh
cargo test --locked --workspace
cargo test --locked --workspace -- --list
cargo build --locked -p mcedit2-iced
cargo run --locked -p mcedit2-iced -- --inspect ../test_files/AnvilWorld
```

只验证受影响行为时，直接使用函数名、模块名或现有集成测试目标，例如：

```sh
cargo test --locked -p mcedit2-plugins plugin_timeout_and_host_exit_release_descendant_pipes
cargo test --locked -p mcedit2-world --test nbt_document
cargo test --locked -p mcedit2-iced far_tile_cache_is_bounded_and_lod_sensitive
```

普通源码、Cargo.lock、资源与 vendored renderer 的变更都会触发 Linux / Windows CI；PR 同样检查。发布打包工作流保持显式启用。

配置测试在独立的 harness 子进程中修改 XDG_CONFIG_HOME，避免影响其他并行用例或用户配置。无效可选设置的测试核实：编辑器可以启动、报告错误，自动持久化不会覆盖原配置。

## 需要本机资源的检查

GPU、真实 Minecraft 实例和框架检查显式标记 ignore。默认测试通过不代表这些检查已运行。先用 --list 确认当前函数名，再按名称选择，例如：

```sh
cargo test --locked -p mcedit2-iced hardware_depth_alpha_and_culling_pixels -- --ignored --nocapture --test-threads=1
```

按所选检查设置准确的本地资源路径：

- MCEDIT2_CLIENT_JAR：相应版本官方客户端。
- MCEDIT2_TEST_WORLD：解压后的实例存档目录。
- MCEDIT2_FRAMEWORK_JAR：框架模型检查需要的准确安装文件。
- MCEDIT2_TEST_RESOURCE_MIB：可选的测试资源预算，例如 4096；不修改产品配置。

这些是本地文件要求，不是凭据。软件渲染或无窗口逻辑测试不能代替设备性能、窗口输入及游戏保存重载验收。

## 原生窗口与性能

默认入口 target/debug/mcedit2 打开 GUI，--inspect 只检查源文件。真实窗口测试需图形会话；结束后关闭本任务启动的窗口和进程。仅在明确发布时构建 release。

MCEDIT2_NATIVE_PROFILE 指定绝对 JSON 输出路径，配合 -view <存档> 可记录自动回放。可选 MCEDIT2_NATIVE_PROFILE_WINDOW=1600x900、MCEDIT2_NATIVE_PROFILE_SCALE=1.5；使用独立 XDG_CONFIG_HOME 隔离视图设置。结果是提交节奏和 CPU/GPU 阶段数据，不直接等于显示帧率。

真实源网格基准由已忽略的实例检查提供；MCEDIT2_MESH_RADIUS、MCEDIT2_MESH_TAG、MCEDIT2_MESH_REFERENCE 控制范围、输出标签和逐字节对照。只在相关性能任务中运行，不作为每次改动的附加门禁。

## 维护约定

保留真实格式往返、来源证明、事务中断恢复、撤销重做和生产任务链的检查。参数化场景适合共用同一不变量；独立行为使用独立测试，保证失败后其他测试继续执行。不要用常量存在、空容器、Debug 字符串或删除功能的源码字符串断言代替行为验证。

新实验若没有产品入口，不在正式模型选择中保留空分支，也不把测试专用实现作为产品覆盖率。记录实际通过、失败、忽略及未运行的目标；不重复使用历史日志来宣称当前提交通过。

本轮审查修复与实际验证结果见 [REVIEW_FIXES.md](REVIEW_FIXES.md)。
