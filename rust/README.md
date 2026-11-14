# MCEdit2 Rust + iced workspace

实体 / 玩家、掉落物 / 挂画、展示框模型可独立显示，地形与方块实体模型各有独立开关；另有命令方块类别填色及位置线框。显示偏好随桌面配置保存，旧设置导入已接入原 AllLayers 十二项开关，包括旧刷怪总开关；世界读取、绘制和拾取共用显隐策略，无资源纯色路径也生效。物品图标、结构文档及待放置预览保持完整。新增路径尚未编译和运行，留待完整迁移后统一验证。

普通编辑主窗口采用单实例，再次启动会唤起已有窗口并按参数顺序打开所附文件，NBT 归属前一个世界参数；支持等待当前操作结束及取消后续打开。已有主窗口时，`-eval` / `-resetPrefs` 及所附文件会明确报错，需先关闭后使用；`-view` 仍可打开独立只读窗口。新增行为尚未编译和运行，留待完整迁移后统一验证。

欢迎页与文件菜单新增 saves 目录浏览：世界元数据列表、筛选排序、出生点地表预览，以及编辑打开和同一程序 -view 只读窗口。目录记录支持原生保存及旧 world_list 设置导入；新增路径尚未编译或运行，留待完整迁移后统一验证。

GUI 启动参数可按顺序指定世界和五种结构文件；普通 NBT 放在所属世界参数之后。结构使用独立标签、资源 / 数据包、三维 / 四视图及编辑历史。新增路径尚未编译和测试，完整迁移后统一验证。

当前本机编译、测试与桌面启动结果见[中文重写进度记录](PROGRESS.md)。测试现已改为标准 Rust harness，按行为独立执行；历史的 25 类计数仅属于早期记录。当前命令与设备检查要求见[测试说明](TESTING.md)。完整功能迁移与真实世界验收仍有缺口。下方各阶段历史说明应结合该记录阅读。

开发、测试和本机运行统一使用 `target/debug/mcedit2.exe`。不加参数打开 GUI；`mcedit2 --inspect <世界路径>` 在终端检查存档；插件由同一个程序的内部 `--plugin-host` 模式隔离执行，不再需要独立 CLI / Python 可执行文件。工作区固定同一套 dev 配置，整批实现完成后集中测试，只有明确发布时才构建 release。

区块属性与 NBT 页已接入完整区块树、实际状态字段、Long 精度编辑、快照批次及文档 / 世界撤销。元数据与方块、BE、biome、实体和旧版光照按各自证明组合保存；坐标、版本及受管理的结构通过所属工具修改。Alpha、MCRegion、Anvil、MCC 和新建区块的元数据往返纳入现有 NBT 类别。

工作区的固定存档 / 视图标签、工具分组与独立页面见 [界面说明](UI.md)。本批集中验证状态见进度记录；尚未达到 Amulet 逐像素复刻或全部原版交互等价。

不透明地形默认使用共享缓冲的高度分段裁剪；后台 GPU 对照及视角差异见[进度记录](PROGRESS.md)。这些离屏计时不能代表桌面 FPS。

资源面板默认索引并解码全部合法 PNG，不设纹理总数上限；自动发现也不再截断 Mod、启用包或叠加来源数量。可用物理内存的一半作为自动工作预算，也可指定 MiB，重新加载生效；预算检查模型源数据预估开销、来源索引、目录排序、解码 RGBA 和 CPU/GPU 图集。该数值是阶段工作集估算，不等于整个进程或实际显存用量。3D 图集按需增加独立页面，收录方块目录和模型实际引用的纹理，其他图片保留 CPU 缓存。面板分别显示索引、请求、成功、失败、缺失引用和图集数量，并可展开诊断。`textures/models/` 是合法纹理路径。

Java 裸模型 / 纹理 ID 默认指向 `minecraft`，与资源文件所在模组无关。支持声明式组合模型、独立子模型纹理、可见性继承、世界用 `base`、根变换、反向元素内侧面和模型材质类型；未知模组运行时渲染后端仍明确列入诊断。真实实例已成功解码 73,899 张 PNG；图集源纹理数量按依赖变化，不是固定上限。 方块状态的 X/Y 旋转及根变换联合 UV 锁定已按实际 1.21.1 客户端核对，包含裁切、反向 UV、缩放、平移、剪切和非直角根变换；不可逆根矩阵用于 UV 锁定时明确拒绝并诊断。

现代原版箱子、16 色床与原色 / 16 色潜影盒的实体声明会生成具体静态模型，包含箱盖 / 锁扣、双箱左右半边、床头 / 床尾、床脚以及六向闭合潜影盒外壳，使用资源包实际实体贴图。几何、UV、材质和源拾取复用现有 JSON 链路；显式自定义模型、loader 和空 elements 不会被覆盖。尚未识别的旧箱子贴图布局明确诊断，开合动画、文字 / 图案和其他特殊实体仍需继续迁移。

自由视角默认使用区块全部原始 Section，读取真实水平邻区块判断遮挡；不再仅绘制相机附近 48 层。完整几何默认半径 8 区块，可手动选择 16/32/64/128；远处高度采样需要显式开启。“精确坐标”居中会移动自由相机，按钮亦可调整视线。32×32×48 的局部观察窗口和部分工具的批次限制均不代表整幅 3D 场景的容量。

手动应用“剖切 / 3D 多层可见面”后使用限定体积；“表面 / 3D 地表”使用指定 XZ 范围的完整高度地表。透视、等轴和俯视共用已应用的源快照，保留剖切高度及选区，飞行不会自动扩大限定窗口；“返回世界视图”恢复完整区块流式读取。未应用的坐标草稿不改变当前范围，“重新加载”读取已应用的窗口。世界或维度切换清除窗口和旧剖切状态。

主视图的透视镜头倍率、等轴缩放 / 平移、俯视缩放 / 平移分别保存；水平 / 纵向切片和四视图各窗亦有自己的观察状态。平面及等轴滚轮缩放以光标为锚点，支持中键平移、方向按钮与逐窗重置。透视滚轮与“＋／−”调整自身镜头倍率，绘制、辅助线、视锥和源拾取使用同一投影。四视图在同一世界 / 维度内同步选点或重新读取时保留各窗倍率、平移和等轴方向；草稿、读取失败及过期结果不会移除已验证快照。观察状态当前保存在会话内，未实现跨进程恢复。

透视与等轴 GPU 控件在等待后台网格时自行安排后续帧，CPU 网格就绪后继续按帧上传不透明面、透明面和图集，实际交接画面后才停止等待重绘。中断后仍为部分结果的流式世代会重新提交，旧近景缓存不会代替当前世代的完成判断。限定红床体积的自动构图已经通过真实源读取、GPU 像素回读与桌面点选；极窄视口、DPI 和更多设备仍需继续验证。

等轴按实际源体素、JSON 变换后顶点和原生 OBJ 范围拟合控件宽高比，范围在后台计算并跨相机变化复用；已取消固定的每格最小 / 最大像素比例。384 格高度与极端宽高比通过投影回归，两格真实红床通过 GPU / Canvas 桌面构图和源拾取。普通世界等轴已与自由视角共享流式区块网格、图集和源拾取索引；近景读取完整高度，远景保留实际采样 LOD 及不可编辑语义。世界等轴首帧确定构图范围；后续分批几何及材质交接只更新深度裁剪，避免重新缩放或挪动画面。中键平移和滚轮锚点缩放转换成 f64 世界焦点，流式区块读取随之移动，透视相机保留原位置。定位方块可移动焦点，重置可重新拟合当前网格。旋转和缩放更新正交相机，复用驻留 VBO。世界等轴与透视使用 GPU，Canvas 继续用于限定体积及显式地表 / 远景预览。透视窄视口自动构图、DPI 和更多设备覆盖尚未完成。

世界 GPU 每帧最多提交 16 MiB 大块数据、单次最多 8 MiB，并在扫描时协作让出事件线程；超大区块和图集页也分段，不设总面数或纹理数上限。新网格、原点、图集和拾取完成后一起交接，等待期间保留旧完整帧；未改变的区块与图集页复用驻留资源。排序和驱动调用不受硬实时保证。

网格按完整区块渐进显示；已完成网格同时持有生成 UV 时的图集，资源切换期间不会把旧 UV 交给新图集。重复状态的面定义、遮挡规则、UV 和生物群系着色分别缓存。本项目渲染和世界读取代码在开发模式启用一级优化，其他依赖保留原有调试配置。

区块使用独立坐标原点；移动视角只更新 GPU 坐标偏移，保持几何、拾取索引和驻留缓冲。每个源面保留 4 个角点的位置、UV 和颜色，GPU 展开两个三角形，数据从每面 360 字节降为 152 字节。液体区块按实际水平邻域和上方流体状态核对缓存，邻域变化会重新建模。本机真实 9 区块共 269,600 个面，几何数据从约 92 MiB 降至 39 MiB；首次建模约 1.46 秒，移动原点复用约 16.5 毫秒。这不代表整场景帧率或其他设备性能。

自由视角支持左键点选、拖动框选、Alt 点选 A / Ctrl 点选 B；拾取使用已显示的模型三角形、图集透明度和源方块归属，并复核有效源状态。远景采样不能当作单个编辑方块。范围替换、形状 / 连续笔画、连通填充和 Biome 使用预算化候选与整组历史，默认任务预算为 256 MiB，不再受整笔 4096 格限制；完整剪贴板、五种结构格式、变换和桌面完整克隆 / 移动也采用任务预算；插件选区、差异预检、结构生成及声明式序列采用相同任务预算，旧方块专用工具保留各自限制，见 [BRUSHES.md](BRUSHES.md) 和 [MIGRATION.md](MIGRATION.md)。该任务预算估算新增编辑工作集，大场景和已有会话的整体内存仍可能高于面板预算。

地图工作区的对象页及 NBT 与实体工具页提供独立 NBT Inspector：从选中真实对象打开，分页浏览、直接编辑标量或批量修改，先显示完整候选，再暂存为一次世界撤销。关闭或取消候选会恢复正式来源，文档编辑保留（关闭文档除外）。1.12.2 / 1.20.1 / 1.21.1 已验证实体与 BE 联合保存，以及带乘客的跨区块移动、跨 Region 克隆、新 UUID 和删除保存；现代布局另创建真实独立实体 Region。1.12.2 新槽可携带内嵌实体与乘客，支持缺 UUID 的结构导入、旋转粘贴、后续编辑、再次克隆 / 移动 / 删除及联合保存。1.12.2 的方块、biome、BE、实体和旧版光照可共用同一保存证明，真实临时存档已完成重复回滚、提交及读回；游戏重载仍待验收。

工作区的存档标签、维度与视图栏固定显示，地图侧栏分为编辑、对象、视图、资源；玩家、世界属性、地图物品、结构、插件、NBT 与保存 / 恢复分别归入工具页。切换页面保留文档与候选，滚动位置隔离；布局与剩余验证范围见 [界面说明](UI.md)。

文件面板提供独立的全维度玩家列表，可按实际文件来源打开属性和背包、移动到相机并同步朝向。世界时刻快捷键保留当前天数；普通元数据保存后后台重读，并保留未保存的区块编辑和历史。地图支持连续图片导入、按世界撤销 / 重做与联合保存。当前批次的集中验证结果及支持版本见 [PROGRESS.md](PROGRESS.md)，真实窗口和游戏重载仍待验收。

工具与文件中的“NBT 查找与替换”可按整维度 / 选区搜索实体、方块实体及区块 NBT，筛选名称、值、类型和 ID，定位结果或打开实际节点；选中 / 全部替换和删除先预检完整候选，再暂存为一次撤销，磁盘保存仍需确认。详情见 [UI.md](UI.md)，当前验证边界见 [PROGRESS.md](PROGRESS.md)。

已选方块信息现在同时提供资源语言表中的名称和原始 ID / 状态属性。界面中文、英文分别读取 zh_cn / en_us，支持资源包内 JSON / 旧式 .lang，并从准确客户端版本元数据指定的本机 assets 索引读取原版语言。资源包按原有顺序覆盖；切换语言后即时刷新，不重新读取世界。缺少对应译名时保留原始标识。

只读源快照与区块缓存共享不可变方块状态 / 生物群系，完成的面数组与拾取索引回收增长冗余容量。289 个完整高度区块的同场景基准中，已统计 CPU 数据载荷从 3.73 GB 降至 2.07 GB；全部 897 万个面与直接发射路径逐字节一致。此统计不代表桌面进程总内存或帧率，详情见进度记录。

视图侧栏提供实体、物品 / 悬挂实体、方块实体、玩家和计划更新的位置图层，以及已保存区块段、可选高度图、1.12.2 常暗 / 夜间刷怪候选和未生成地形区域。各图层保留源路径与摘要，支持透视、等轴、俯视、二维切片和四视图的实际投影及只读点选，后台按区块批次读取并取消过期结果。刷怪候选使用准确客户端方块属性和已保存光照，不代表完整的生物生成判定；未核对版本明确诊断。实体模型的具体范围和验证边界见进度记录，完整实体及动态渲染迁移继续推进。

通用加权模型与 multipart 使用客户端的坐标种子和随机序列；门、床及双格植物使用原版种子偏移。模组专用渲染器不逐个内置适配，未知加载器保留诊断并显示紫黑缺失材质，后续由插件扩展。

本分支是原生 Rust 重写。旧实现保留于 `master` 历史与独立 Python 3 Draft PR。本程序不依赖系统 Python、Cython 或 Qt；命令插件由同一程序启动的独立进程内 RustPython 执行。完整迁移范围及当前差异见[迁移验收表](MIGRATION.md)。

## Crates

| Crate | Purpose |
| --- | --- |
| `mcedit2-world` | Bounds-checked Java Minecraft world/NBT/region formats, original-state editing, transaction and schematic helpers |
| `mcedit2-iced` | iced 0.14 desktop, wgpu 3D view and selection, streaming/chunk meshing, vanilla resource-pack models and multi-atlas textures |
| `mcedit2-plugins` | 插件目录、启停配置、进程协议、候选编辑校验 |
| `mcedit2-python` | RustPython 0.5 运行库，不再生成单独程序；GUI 入口及该库需要 Rust 1.93 |

## Resource-pack overlay order

The native texture input supports multiple local folders, ZIP resource packs
or client JAR paths separated by `;`, **lowest priority first**. The selected world's `saves/` parent determines its Minecraft game
directory. The renderer attempts to layer a matching vanilla client JAR,
resource-bearing JARs under that same instance's `mods/`, generated
`kubejs/assets/` files, and the `file/...` packs enabled in that
instance's `options.txt`, in increasing priority. Only explicitly selected
packs are enabled; other installed resource packs are ignored. Manually
selected ZIP/JAR/folder layers have the highest priority. Upper layers override
only matching PNG, model JSON or blockstate JSON identities; absent overrides
retain an intact lower layer.
A new static PNG does not inherit stale `.mcmeta` animation instructions
from a lower layer. All decoding happens in the existing background worker,
and the combined atlas is generated once under existing memory/GPU budgets.

## Dotted mod resource identities and asset roots

Minecraft Java namespaces and resource paths may contain periods (for example
`create.addon:block/machine/casing.v2`). The native PNG and JSON readers
now share strict canonical validation for these IDs, while rejecting invalid
case, backslashes, empty or dot-only directory segments, and traversal
components. Directory resource packs refuse an `assets/` symlink and ignore
symlinked children; loading remains read-only. Directory iteration is
deterministic even if the OS returns entries in a different order.

## Development commands

From `rust/`:

```sh
cargo test --locked --workspace # 可按模块或行为名称过滤
cargo run -p mcedit2-iced
cargo run -p mcedit2-iced -- --inspect "/path/to/extracted/world"
```

Windows packaging is an opt-in workflow. Do **not** trigger Actions, tests, builds or packaging on the GitHub branch merely to validate source changes unless the user expressly requests them.

## Current implementation

- Accept original Java Minecraft world folders, metadata, signed coordinates and known Alpha, MCRegion and Anvil source layouts. Unknown/modded data must not be invented or silently rewritten.
- Open directly into a 3D spectator-style viewport; fly, mouse-look, inspect, select and stream existing chunks. Sparse distant LOD is a visualization only, never an editing authority.
- Auto-discover Java PNG, `.mcmeta`, blockstate JSON and model JSON assets from the selected world's vanilla client, installed Mods and enabled resource packs. Mixed-resolution, multi-page GPU atlas packing preserves per-face page and UV mapping. Unresolved foreign-namespace models display the explicit purple/black missing texture, not inferred block geometry or name-based sprites.
- Use in-memory edit groups, preview/undo, native source-validation and deliberately guarded save paths. World writes must not occur implicitly from inspection or a resource-pack load.

Read [COMPATIBILITY.md](COMPATIBILITY.md) for the detailed data-version/storage safety policy. See [original-format regression fixtures](../test_files/README.md) and `crates/mcedit2-world/tests/` for native regression coverage.

## Migration status

This is an active rewrite, not complete MCEdit2 feature parity. Old Python sources, Sphinx docs, PySide/Cython setup and Python-only benchmarks/tests are intentionally absent from **this branch**. They remain available in the original branch/history. Do not use the old Python app instructions for building the Rust binary.

The latest code must be checked with the toolchain before a distributable build is described as verified. The previous Windows development ZIP does not automatically contain newer source changes.

## Rendering architecture: Amulet reference and source-driven visibility

The 3D renderer is being brought closer to the chunk lifecycle used by
[Amulet Map Editor's chunk builder](https://github.com/Amulet-Team/Amulet-Map-Editor/blob/0.10/amulet_map_editor/api/opengl/mesh/level/chunk/chunk_builder.py)
and [region merger](https://github.com/Amulet-Team/Amulet-Map-Editor/blob/0.10/amulet_map_editor/api/opengl/mesh/level/region.py).
These are architectural references, not copied code or proof of parity.

- No fixed global spectator face or visible-voxel count truncates readable
  original source geometry. Requested source chunks are shown completely when
  their mesh is available; no geometry is fabricated to fill absent chunks.
- Exact chunk vertices are cached against exact source voxel values, model
  style and source origin. A cache entry is retired when its chunk leaves the
  current source snapshot (or its source/style changes), rather than after an
  arbitrary fixed number of cached vertices or chunks.
- Source stream snapshots coalesce: same-world chunk batches do not repeatedly
  cancel the currently active geometry build, which prevented any completed
  viewport from appearing when IO was faster than geometry creation.
- Large meshes are segmented on complete six-vertex faces at the active
  adapter's `max_buffer_size` and the draw API's index bounds. All segments
  are uploaded and rendered; the segmentation is not a face-count budget.
- Offscreen color/depth targets use `max_texture_dimension_2d` from the
  active wgpu device, not a fixed 4096px threshold. A 128-chunk streaming
  radius is now selectable, while the planner does not impose a hidden 64
  chunk restriction.

Resource decompression, image sizes and GPU texture-array limits still need
validation against memory/device capabilities; these are not permission to
discard successfully read world chunks. The existing mixed-resolution atlas
memory guard is still present and may downsample high-resolution textures in
large packs. Improving multi-array atlas scalability remains separate work.

The 3D stream now directly borrows the near view and individual far
patches into the mesh worker. It builds a global **reference-only exact
neighbor index** for cross-chunk face culling and water/LOD dependencies,
but it does not allocate a combined scene-wide SurfaceView. Each original
source chunk or LOD patch produces an immutable, independently owned
`Arc<Vec<Vertex>>` with a stable source key. Exact unchanged chunks are
reused from the source-exact mesh cache, while changes to an adjacent chunk
invalidate its shared seam faces. The worker publishes a complete
`StreamMesh` generation only on successful completion.

GPU residency is now keyed directly by these source chunk identities.
Unchanged `Arc` mesh parts reuse their existing device vertex buffers and
page draw spans, with **no second whole-scene face regrouping pass**.
Opaque/cutout chunk buffers are drawn independently. Only translucent
faces are collected and sorted globally when needed, preserving the
cross-chunk ordering required by glass and liquids. Texture atlas pages,
source origin, and the chunk mesh generation remain paired during frame
handoff. The legacy screen-space isometric and non-streamed preview paths
remain separate for compatibility.

The above Rust tests are source-level regressions, not a verified Windows
GPU result. A full local `cargo check --workspace`, `cargo test --workspace`,
and a large authentic Modded-world visual comparison against Amulet are
still required before describing renderer parity.

## Incremental chunk source and GPU refresh

The native terrain compositor now separates three distinct operations:

1. **Original chunk reads** are queued in camera-facing order, without
   filtering any valid source chunk out of the requested radius.
2. **Visible scene assembly** retains the publication order of already
   completed chunk reads. New chunks append to the existing draw order and
   an upgraded LOD replaces the same chunk's slot. Camera yaw changes only
   reprioritize unread IO; they do not rebuild terrain geometry.
3. **GPU uploads** compare complete six-vertex faces by their actual POD
   bytes. Equal-length replacements update dirty intervals rather than
   rewriting all later unchanged chunks. Insertions/removals use a safe
   shifted-suffix upload. Changed camera uniforms without transparent
   geometry do not scan the source mesh each frame.

The exact-chunk mesh builder iterates directly over contiguous original
source voxel slices instead of allocating intermediate per-chunk vectors of
references. Cache validation still uses exact `SurfaceVoxel` equality, so
changes to an original face mask, blockstate, biome or staging status
invalidate affected mesh data without relying on a hash collision assumption.
Region seams remain checked against real original neighbors, using a
hash-indexed lookup rather than repeated balanced-tree searches. Existing
coarser source patches remain available during finer reads after camera
replanning; absent source chunks are never synthesized.

Both 3D spectator and the isometric projection no longer truncate visible
faces at historical 150k/175k-face thresholds. Rendering hardware constraints
remain handled by device-sized GPU buffer segmentation, not a fixed
user-visible geometry quota. Cached layouts and atlases continue to require
independent memory and device-capability checks.

These changes were developed from the chunk/region lifecycle principles in
Amulet Map Editor. They are not an Amulet port. The newer stream renderer
now preserves per-chunk geometry and GPU buffers without a combined scene
mesh, while conventional isometric and non-stream preview code remains
separate. Atlas page bindings use independently allocated GPU textures.

No GitHub Actions/CI or Windows packaging runs were initiated for this
change; offline source inspection and regression test additions are not a
substitute for running `cargo check`, the tests and real-save GPU validation.

## Chunk-resident GPU geometry and immutable render generations

The spectator GPU renderer no longer uploads a single monolithic opaque
world buffer. It groups original six-vertex faces by source chunk coordinates
and atlas page and keeps **independent wgpu vertex buffers** for each group.
When a chunk changes, unchanged groups reuse their byte-identical buffers;
inserting terrain in one chunk does not shift the GPU offsets of another.
The separate global translucent stream remains sorted back-to-front across
chunk and texture boundaries so glass and water do not lose painter order.

The atlas uploader now uses **one independently allocated one-layer texture
array per occupied atlas page** and switches bindings at ordered face runs.
This removes the former 256-array-layer and 768 MiB total-atlas caps and
avoids reserving unused GPU layers. Updated pages can be uploaded without
recreating unchanged page textures. The PNG loader's per-asset decompression
guards and existing CPU decoded-image memory protection remain in place;
hardware resource exhaustion must still be tested on actual devices.
Non-power-of-two atlas page dimensions avoid allocating an 8192px square
just to fit a single 4096px sprite with edge padding.

For streaming scene publications, the UI stores **immutable Arc-backed
near views and chunk patch references**. The background mesh builder
consumes those source slices directly, using a shared reference-only neighbor
index to reconcile exact original faces. It does not materialize a combined
SurfaceView, even off the UI thread; output consists of independently cached
chunk meshes. The orthographic/legacy preview path is retained separately.

A previous completed frame comprises its vertex buffer, original coordinate
basis and the exact atlas that produced its UVs. During a texture-pack or
scene refresh the old complete generation remains visible until a replacement
mesh is ready; GPU page bindings and mesh draws are updated in the same render
preparation cycle. World switches and mutations still revoke stale frames.

**Validation:** the code includes regression coverage for atlas page draw
order, negative chunk coordinates, frame handoff and Arc-backed snapshots.
The revision has not been compiled with Cargo, run with a GPU, or compared
against a real Minecraft modpack; rendering correctness and large-pack memory
behavior remain unverified. CI and Windows packaging remain user-triggered.

## Incremental LOD halo invalidation and correct GPU material passes

Far-source `FarChunkPatch` mesh caching now records an immutable source
identity and its immediate horizontal-neighbor identities, plus the near
exact-surface identity only where a patch borders the near viewport. Each
stream mesh publication builds a coordinate-indexed dependency table once:
a distant patch is reused unchanged when unrelated chunks arrive, regardless
of the order in which other patches finished streaming. A newly loaded,
replaced, or departed neighboring source invalidates dependent distant cliff
meshes without relying on lossy hashes. Existing exact-source mesh entries
share their `Arc<Vec<Vertex>>` directly with the completed frame and the
GPU instead of copying a full vertex array on cache insertion.

The stream GPU importer separates blended quads from opaque/cutout quads.
Only opaque/cutout faces are placed in source-resident vertex buffers. Blended
faces are submitted once through the camera-sorted transparent pass. This
prevents duplicate vertex transfers and fragment work for water and glass
while maintaining original page bindings. Fully opaque source chunks take
a borrowed, allocation-free preparation path.

Original cross-chunk occlusion indexing is restricted to candidate block
faces on chunk boundaries. Known internal faces are already resolved by the
world reader. When a seam mask must change, it is computed once per source
run, and only the affected chunk's original voxels are cloned. No arbitrary
scene-face, visible-chunk or LOD-mesh-count budget is introduced. Hardware
buffer limits and actual resource allocation remain authoritative.

Regression cases cover an unrelated distant chunk arriving before an
existing patch, a newly discovered adjacent source invalidating its LOD
cache, negative-coordinate source seams, exact Arc cache identity and mixed
transparent/opaque material page routing. These tests must still be compiled
and exercised against real Java worlds and graphics drivers.

## Persistent boundary indexes and camera-invariant transparency

Cross-chunk face culling no longer rebuilds one world-wide coordinate hash
from every source voxel on every progressive stream update. Instead, a
worker-owned `SeamIndexCache` keeps only boundary voxel **offsets** for each
immutable near-view and original far-chunk `Arc`. New/replaced sources build
new indexes; unchanged source pointers reuse their original allocation;
departed far sources are pruned. Sampled-only and empty far tiles share one
empty index. A small per-snapshot chunk-to-source directory locates
neighboring original states without copying palette/NBT data. Only verified
cross-chunk horizontal neighbors can suppress a source face, retaining
correctness at unknown or unloaded boundaries.

Original face-mask corrections are now copy-on-first-change: unchanged
source runs never allocate temporary masks or duplicate their voxel array.
Only a chunk with a real seam difference creates a local corrected copy;
every other original source buffer remains shared.

Global blended material order remains exact, but the pipeline extracts
each transparent six-vertex quad and its original four-corner centroid once
when the mesh generation changes. Camera rotation sorts a cached compact
quad-index array instead of rescanning every opaque source chunk. Pure
camera translation changes each transparent depth by the same constant,
so the previous draw ordering and GPU transparent buffer remain valid:
only the camera uniform and offscreen image are refreshed. There is no
heuristic sorting cap or guessed Mod-specific rendering behavior.

The new regression cases check source-index reuse and replacement, cache
pruning, consistent near/far origin handling, transparent quad sort order,
and translation-versus-rotation semantics. Tests have not been executed
with Cargo or a real wgpu adapter; these are source-level changes, not
claims of compiled or visually verified results.

## Camera-facing full-ring streaming

The native spectator still schedules **every** actual Java chunk inside the
selected radius and keeps the exact nearby voxel sources separate from
read-only sampled distant LOD. Within each ring, the streaming worker now
prefers the chunks in front of the camera before those behind it. The first
distant warmup uses the same priority, so the visible horizon can appear
earlier without discarding nearby geometry. Eight quantized camera headings
reorder only unread tasks and the display budget; already cached Region data,
in-progress reads, edit history and source NBT are not changed. Priority never
acts as a visibility or source-data filter.

## Minecraft instance resource auto-detection

With 3D spectator enabled, MCEdit2 probes the exact version-matched vanilla
Java client JAR, first near the world save and then at known Minecraft,
PrismLauncher and MultiMC paths. When the world is inside `saves/`, its parent
game directory is also used for a **bounded, flat** read-only scan of
`mods/*.jar` files that actually contain `assets/<namespace>/...`;
disabled JARs and plain Java libraries without assets are skipped.
`kubejs/assets` is treated as a generated resource layer. Only `file/...`
entries enabled in `options.txt:resourcePacks` are loaded from `resourcepacks/`,
in their recorded low-to-high order. No Mod code/scripts are executed and no
network/download or disk-wide discovery occurs. The UI can rescan the same
instance without clearing the previous completed atlas. Switching game
instances invalidates the old atlas and discards stale worker results.

**Core renderer scope:** Mod assets use the same declarative vanilla Java
blockstate variants, weighted choices, multipart conditions, JSON parent
inheritance, elements, per-face UV, rotation, tint and `textures` aliases.
Only standard PNG and `.mcmeta` sources are imported by the production
resource loader. If a foreign-namespace block has no resolvable Java model,
has an unresolved parent, specifies a custom `loader`, or has invalid
geometry, the viewer uses the fixed purple/black missing-texture marker on
a conservative placeholder rather than guessing a same-named PNG. Missing
individual JSON faces remain absent, not invented. Namespace preservation
prevents accidental cross-Mod sprite borrowing.

Custom Mod geometry, OBJ/MTL, glTF/GLB, block-entity renderers, dynamic
NBT-dependent meshes and executable client render hooks are **not enabled**
in the production asset path. They are reserved for a later opt-in plugin
rendering interface; no current plugin support is implied.

## Exact 3D chunk seam visibility

The native 3D spectator streamer now uses the original world reader's
block-face occlusion predicate when independently loaded exact chunks
meet. Identical glass and leaves hide only their proven internal faces;
ice/stone and glass/stone keep the original visible opaque face, while
full slabs, partial slabs, eight-layer snow, fluids, and unfamiliar mod
blocks follow the same conservative rules as within a source chunk.
Read-only distant LOD samples never count as an exact occluding neighbor.
This avoids both visible internal seams and accidentally missing faces
when a 3D camera crosses Region/chunk boundaries. The scanner borrows
source blockstate data instead of duplicating all palette strings.

## Corrupt resource-pack fallbacks

Directory packs, ZIP packs and client JARs now share the same resource
classifier, per-entry byte limits, and texture/model ingestion path. An
invalid modern `textures/block/` PNG cannot reserve an ID or suppress a
working legacy `textures/blocks/` PNG in the same pack. Valid modern
textures still take precedence independently of ZIP member order. If a
higher-priority pack has an unreadable block image, the previously loaded
lower-priority image and its animation metadata remain intact. This is
display-only: assets are not extracted or written back. Four regression
tests cover corruption, archive ordering, layered fallback, and identical
directory/ZIP lookup behavior.

## Native shape fallback for connected vanilla blocks

When a texture-only resource pack omits the original Java blockstate/model
JSON, spectator rendering uses state-aware, read-only geometric fallbacks
instead of full placeholder cubes for glass panes, stained glass panes, iron
bars, low/tall walls, fence gates and wooden fences. North/east/south/west
connections, wall post flags, gate facing/open/in-wall flags and independent
upper/lower fence rails all affect the native 3D vertices. This is a bounded
fallback, not a substitute for arbitrary modded geometry or the complete
Minecraft special renderer. When genuine resource-pack JSON resolves, its
declared geometry **always wins**. Unknown namespaces stay conservative.

Seven Rust regressions exercise the original connection-state geometry,
resource-model precedence and the actual perspective mesh's open/closed
passage. No source NBT is changed, and the distant sampled LOD is untouched.

## Original stair corner geometry

For vanilla stair blocks without a resolvable Java blockstate/model JSON,
the native 3D spectator renderer now respects the original `shape` property:
`straight`, `inner_left`, `inner_right`, `outer_left` and `outer_right`.
Combined with four `facing` directions and `top`/`bottom` halves, these
produce 40 tested source-state arrangements rather than one rectangular
tread. The upper or lower step is split into exact quarter-block cuboids
to preserve inner corners and prevent oversized outer corners; exact shared
inner faces remain subject to mesh occlusion. Unknown shape/facing values
fail closed to a conservative selection cube, and custom Mod namespaces
are never assigned vanilla stair geometry. Loaded resource-pack JSON model
elements remain authoritative instead of using these fallback boxes.
Tests include a world-space spectator mesh inspection of all five shapes.

## Native Java double-height door shape fallback

When the installed Java client or a texture-only resource pack does not
provide a resolvable original blockstate/model JSON, native spectator mode
renders vanilla wooden, metal and copper double-height doors as a thin
3/16-block panel rather than a full opaque cube. The source `facing`, `open`,
`hinge` and `half` properties control edge placement for both door blocks.
Invalid or foreign-namespace states conservatively retain the previous
fallback; existing blockstate/model JSON always takes precedence. Source
world data, block entities and other editor state are never modified.
Regression tests cover all 4 directions, both hinges, open/closed panels,
upper/lower alignment and the real world-space GPU vertex extents.

## General Java block JSON 3D model coverage

The native 3D model/texture pipeline no longer assumes that every block
model or sprite lives under `models/block/` or `textures/block/`.
Blockstate files can reference **namespaced model parents and elements** in
`models/item/`, `models/custom/` or other original JSON model families.
Their face textures can resolve namespaced sprites in `textures/item/`,
`textures/entity/` or other Java resource folders. Canonical
`textures/block/` and historical `textures/blocks/` names retain their
existing shared block sprite IDs without colliding with non-block families.

Authentic model elements may extend outside the source block, within Java's
bounded [-16,32] source model unit range. They retain their exact
read-only 3D vertex coordinates and declared UVs; models with up to 256
elements each and 512 elements total per blockstate are supported before
conservatively falling back rather than consuming unbounded CPU/GPU memory.
Model parent recursion, resource path validation, archive byte budgets and
render face limits remain enforced. New regressions check directory/ZIP
model/PNG parity, nested item/custom model inheritance, namespaced item and
entity textures, large multipart models, malicious paths, and the emitted
GPU spectator vertices for a real out-of-cell Mod block model.

This improves arbitrary static JSON-defined block display but **does not
claim every special renderer is implemented**. Minecraft block-entity
renderers (e.g. chests, banners, signs and beds with dynamic NBT content),
entity model rigs, shader-specific animated geometry, runtime OBJ/custom
loaders, multipart non-JSON renderer hooks and true tiled UVs outside
0–16 still require specialized rendering work. No remote shader, executable
or Mod code is loaded. Render samples never become edit sources.

## Generic static atlas-safe repeated UV rendering

The perspective spectator renderer can now display original static JSON face
UV ranges in [-256, 256] without incorrectly stretching or clamping a texture
at the 0–16 boundary. A renderer-agnostic partitioner splits each face along
the original 16-unit texture periods, preserves UV quarter turns/reflections
and exact 3D vertex geometry, and maps each subface back into the original
atlas slot. Because GPU atlas sampling remains clamped, UV repetition never
samples a neighboring Mod sprite. Work is capped at 256 tiles/source face
plus the existing total mesh face limits. Independent tests and a resource-pack
to GPU-vertex integration test validate actual repeated quads and UV bounds.

This is generic static model handling, not per-block renderer special cases.
Dynamic NBT-dependent block entity shapes and non-JSON loaders still need a
source-aware plugin/model contract; no Mod executable render code is loaded.

## 当前原生 Java OBJ / MTL

自动资源栈现按“原版客户端 → 实例声明的 Forge / NeoForge 框架 JAR → Mod → 生成资源 → 启用的资源包”排列。框架版本来自当前 Prism / MultiMC 实例的 `mmc-pack.json`，读取对应组件元数据中的库声明；本地 patch 和 local 库路径也按声明处理。只读取已有准确版本，不混用其他实例或已安装版本，不在应用中下载或执行 JAR。缺失框架会明确报告；手动资源入口仍可补充准确版本的包。

桌面自动和手动资源入口 `open_stack_java` 已支持 `forge:obj` / `neoforge:obj`，包括组合模型中的 OBJ 与 JSON 元素、根变换与反射、分组可见性、`mtl_override`、材质纹理别名、正负索引、法线和逐顶点颜色。MTL 引用仅从已索引资源包解析；作者机器上的纹理路径按 NeoForge 的资源 ID 转换规则处理，不访问外部文件。生产 Java 路径不载入 glTF / GLB、模型旁置图片或二进制缓冲。

透视、等轴 GPU 与 Canvas 使用实际静态三角形、UV、逐顶点 RGBA 和原始源方块归属。等轴拾取保留材质 Alpha 和源邻面遮挡，关闭纹理仍保留几何。选中、悬停、方块实体和暂存标记按实际源方块范围画一次边框，不描出模型内部的所有三角边。等轴 Canvas 复用 GPU 投影网格，逐像素采样原始图集并执行深度测试、Alpha 遮罩和透明混合，不再随面数降低纹理精度。后台任务可取消，完成后自动刷新；材质采样档位参与缓存，Detailed 使用双线性过滤，其他档位使用最近邻。图片与辅助线使用独立图层，避免图片遮住源选框。俯视和二维切片的采样精度仍需继续核对。尚未注册的模组运行时颜色、FX 纹理和动态渲染器仍会缺失或明确诊断，不能据此宣称整包渲染已经全部兼容。

## 历史实验后端（以下保留旧实现说明）

以下英文段落描述曾经的实验 `open_stack` 和静态适配器，部分范围、上限与拓扑说明已经被当前生产路径替代。尤其 MTL 的 `d/Tr`、`map_d` 和 PBR 参数属于实验扩展；原生 Java OBJ 遵循 NeoForge 的 Kd Alpha 及实际使用字段。当前支持范围以本节和中文进度记录为准，不能用实验测试推断桌面原生支持。

The universal resource-pack model path now includes a **guarded static OBJ
backend** selected by `forge:obj` / `neoforge:obj` in the original block
JSON. Source OBJ files under `assets/<namespace>/models/` are parsed
read-only; their declared vertex/UV data, material names and blockstate
X/Y rotations are mapped to the same texture atlas and wgpu spectator scene
as ordinary Java cuboid models. No resource file is executed or extracted
and no external `mtllib` or remote references are followed. Resource bytes,
vertex/triangle counts, coordinate ranges and available material names
remain bounded. Unknown custom model loader identifiers are not guessed.
Existing GPU batch sorting expects six vertices per face; each static OBJ
triangle uses one degenerate trailing triangle for coherent transparency
grouping. Integration tests exercise real resource JSON -> OBJ -> atlas
material -> 3D GPU vertex flow, without hardcoded block identifiers.

This backend is **not** a complete Wavefront implementation: this first
static subset supports indexed triangular and quad faces with UVs; complex
polygons, nested MTL pipelines, vertex normals, runtime loaders, bones and
dynamic block entity renderers still need independent, source-verified
render backends. Current OBJ UV values outside the sprite's [0,1] domain
are clamped; the general repeated-UV path presently applies to Java JSON
faces, not arbitrary OBJ triangles.

## Static resource-renderer adapters

Production resource discovery supports Java JSON models and declared OBJ/MTL assets. An explicit `mcedit2:static_renderer` adapter can bind an OBJ model without executing Mod code:

```json
{
  "loader": "othermod:custom_client_renderer",
  "mcedit2:static_renderer": {
    "format": "obj",
    "model": "othermod:models/custom/structure.obj",
    "textures": {"particle": "othermod:item/structure"}
  }
}
```

OBJ triangles share the atlas-safe UV clipping, transparency, source picking and resource-budget paths. MTL diffuse color, opacity and pack-local opacity maps use the same production pipeline. Geometry is resolved from explicit assets; unknown executable renderers report diagnostics. The test-only glTF/GLB experiment and its unreachable model branches have been removed.

The last complete streamed mesh retains its matching atlas and source origin until the next complete result is ready. Cached high-detail chunks can replace sampled far patches; far LOD remains a non-editable visualization. This frame handoff preserves source identity rather than inventing missing terrain.

## Fullbright spectator flicker and missing-texture diagnostics

The compact 3D sidebar now distinguishes the **Textures on** preference
from a successful resource-pack load. If no original client JAR or resource
pack is installed/recognized, it explicitly reports that the current view
is a palette-only preview and offers the existing **Open ZIP/JAR** or folder
controls. When assets are loaded, it reports the PNG, atlas-page and model
counts and shows any asset discovery/decode error in the compact sidebar.
Original block textures cannot be inferred from saved Java block IDs alone.

The streaming GPU viewport preserves the **last complete spectator mesh and
its matching source coordinate origin** until a newly streamed source scene
has finished meshing. Source updates are coalesced while the single GPU mesh
worker is active; this prevents every incoming chunk batch from cancelling
the only work in progress and leaving a blank frame. The newest ready mesh
takes priority automatically, and original world/texture-source invalidation
discards any stale fallback so it cannot display wrong atlas pages or worlds.
Camera movement continues to update the camera uniform rather than
rebuilding complete meshes. The handoff regression checks both the old
origin while waiting and new origin when the replacement arrives.

This fixes empty-frame mesh handoff artifacts; it does not claim full
Minecraft client rendering fidelity from distant sampled LOD, nor does it
invent PNG textures when none have been supplied. Full 3D visual validation
against a real world, client resource JAR, and GPU remains a separate task.

## Advanced tools: bilingual grouped interface and native file pickers

The advanced editor is now organized into six selectable tool areas rather
than showing every operation in one continuously stacked form: world/block
inspection, GPU/texture/3D scene controls, block entity NBT/entities,
2D/vertical maps, staged block editing, and file exchange/Region recovery.
The active area is explicit and the navigation, buttons, descriptive
instructions, file-path placeholders and safety notices support both
Chinese and English with the existing language toggle. Source world and
dimension selection remains visible at the top.

All ten advanced import/export file-path families now have a native
**Browse** action; the eight file families that export data also offer
**Save as**. Covered paths include BE batches/journals/snapshots, entity
snapshots, clipboard, MCEdit/Sponge schematics, Region plans, plugin
manifests and edit journals. Existing-world and resource-pack pickers are
also exposed in the advanced interface. Dialogs use `rfd` on separate
threads, with conservative filename-extension filters. Cancelling a
dialog never changes the current value, and manual path typing is still
supported. File selection alone never triggers world modification;
the original explicit preview/stage/save confirmations remain mandatory.

Rust offline regressions exercise Chinese/English tab states and bounded
file-picker extensions. UI navigation does not reset selection, staging,
undo/redo, loaded worlds or source NBT. This changes UI and path entry, not
the behavior or safety requirements of any editor operation.
