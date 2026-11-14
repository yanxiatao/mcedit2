# 原生 Python 插件示例

命令行 -eval 的 self 现与插件应用代理共用标签和应用操作接口，包括 loadFile、currentTab、tabWidget、会话 editorTab、关闭 / 退出和存档列表显隐。-eval 仍须绑定成功打开的真实文档。脚本先在私有来源完成数据操作，再集中发布；如调用 save()，等实际保存完成后才按顺序处理应用请求。脚本消息和应用进度显示在状态区；整个启动任务的成功还包含后续应用操作，保存成功不能替代应用成功，错误或取消会停止后续步骤并保留实际完成的工作。

两种入口均使用本次请求的真实目录快照，异步 loadFile 不在同一回调中伪造新会话或新选中标签；等待新文件后继续读写的同步续接仍待迁移。新增路径尚未编译、测试或开窗，待完整迁移后统一验证；下方阶段记录中的 -eval 应用操作待办以本条为准。

应用代理支持 loadFile(filename, readonly=False)、currentTab()、tabWidget.count() / widget(index) / currentWidget() / currentIndex() / indexOf(tab) / setCurrentWidget(tab) / setCurrentIndex(index) / tabBar().moveTab(source, destination)，以及 tabCloseRequested(index)、closeCurrentTab()、exitEditor()、showWorldList()、hideWorldList()。会话的 editorTab 对应真实顶层标签；尚未建立来源的加载或错误标签没有 editorSession。操作随完整候选确认，然后按请求顺序调用原生界面流程；关闭仍会复核未保存修改。与 save() 合用时，只有显式保存成功后才继续这些应用操作，错误和关闭取消会停止后续步骤。界面可取消尚未执行的应用请求，已开始操作由其原生界面处理。

loadFile 和标签改变是异步应用请求，不返回新会话，也不在同一回调伪造已经打开 / 切换后的目录；回调中的 sessions、currentSession()、currentTab() 和索引反映本次请求的实际界面，下一次真实文档调用才获得更新目录。尚未支持阻塞等待新文件并在同一回调继续读写新会话；-eval 应用操作扩展也仍待继续。readonly 仅限制新打开的该份文档；全局 --view 仍限制全部文档，已存在的会话不因再次打开而改变权限。这里提供经核实的原生应用操作，不代表任意 Qt 窗口 API。新增路径未编译、测试或开窗，待完整迁移后统一验证；下方历史待办以本条最新状态为准。

真实会话支持 currentSelection 的设置 / 清空、selectAll()、deselect()、gotoDimension(name)、zoomToPoint(point, distance)、按选区删除内容，以及 save()；应用 saveCurrentWorld() 保存当前真实会话。几何查询直接使用该选区的实际形状，不会在获取选区时读取整片方块。其他会话使用相同接口。视图操作、内容修改及显式保存请求一起进入候选，确认前不改变正式文档；只有视图操作时不增加内容历史。持续工具的后续输入可以读取前序私有选区与维度，冻结局部缓冲仍绑定原实际来源。确认后的显式保存按请求顺序进入已有事务队列，保存当前文档全部已暂存内容；失败状态正常显示，内存修改保留，不自动重试。只读会话可进行纯视图操作；Inspector 外围来源继续禁止内容写入和保存，目标 NBT 沿专用候选处理。加载插件命令目录仅允许读取，构造器夹带编辑或会话操作会报告错误。应用打开 / 关闭 / 退出及主动标签切换仍在迁移；新增实现未编译、测试或开窗，待完整迁移后统一验证。

从世界或已读取的结构文档调用插件时，可导入 mcedit2.editorapp.MCEditApp，通过 MCEditApp.app.sessions 取得本次请求中实际已建立的世界与结构会话，通过 currentSession() 取得当前编辑会话。菜单加载也绑定同一真实目录。清单按实际标签排列，保留各文档未保存的正式暂存内容与同工具的前序候选；会话提供 filename、readonly、currentDimension、currentSelection 和 worldEditor。世界接口按真实维度查询，结构仅代表该文件。可以跨这些来源复制或编辑，全部差异集中确认后才暂存，各有实际改动的文档分别产生历史；只改其他文档时不为主文档增加空历史，仍须显式保存。关闭依赖文档前可回到所属世界或结构插件页复核；已关闭文档的引用失效，重新打开不会冒充旧会话。-eval 也暴露相应原版应用入口。应用打开 / 关闭 / 切换 / 保存请求尚在迁移，任意 Qt 窗口能力未宣称支持。新增接入尚未编译、测试或开窗，待完整迁移后统一验证；下方历史阶段的跨文档待办以此条最新状态为准。

Inspector 的 editorSession.currentDimension、worldEditor.getDimension(name) 及目标引用的 chunk 现在读取实际来源，包括同一世界其他维度尚未保存的正式暂存内容。传入 Inspector 的目标 NBT 仍可私下编辑，最终生成该对象的专用 NBT 候选；通过世界或区块接口写入其他内容会正常报错。真实文档 readonly 标志不因该查询限制而改变。读取来源或实际选择在预检及确认前变化时，候选作废并要求重新加载；独立结构只提供该实际文档的维度，不伪造外部世界。新增接入尚未编译、测试或开窗，跨文档会话继续迁移。

普通命令可以在没有选区时运行，此时 editorSession.currentSelection 为 None，currentDimension 和 worldEditor.getDimension(name) 仍代表实际维度。范围、区块及对象接口可直接使用；需要密集缓冲时使用显式 edit 范围，不会自动读取整个维度或代填一个选区。没有明确放置范围时，返回结构会进入放置预览；若同时编辑世界，先确认这些编辑，成功后再选择结构放置位置，取消候选会一并丢弃该次待放置输出。有范围的组合生成保持原流程。数字方块名称按捕获来源的实际版本解析，不共用其他版本的注册表。新增路径仅完成源码接入，未编译、测试或开窗；Inspector 跨维度读取和跨文档会话继续迁移。

普通命令在实际选区下也可使用 worldEditor.getDimensionNames() / getDimension(name)，读取和编辑同一世界各维度当前未保存的实际内容。一次性工具输入及独立结构普通命令共用原生范围、区块和对象接口，运行完毕立即回收命令宿主。纯生成结果仍进入放置预览；若同时编辑世界，则全部修改及生成结构合并为完整候选，集中确认后暂存，仍需显式保存。无选区的原生命令上下文、Inspector 跨维度读取与跨文档会话仍待迁移。新增实现尚未编译、测试或开窗，待完整迁移后统一验证。

持续工具可通过 editorSession.worldEditor.getDimensionNames() 查询同一存档的真实维度，用 getDimension(name) 读取或编辑其中任一维度。来源包括各维度当前未保存的正式暂存内容及工具前序候选，范围、区块、对象引用和直接复制沿用已有原生接口。每次调用及请求结束前交接待写对象，最终列出所有维度的完整候选；集中确认后才暂存，各维度可通过现有历史撤销，仍须显式保存。后台来源变化或任一维度失败时不部分发布。该接入仅实现于持续工具宿主，一次性普通命令与跨文档会话仍待迁移；新增实现留待完整迁移后统一验证。

动作可用 `setShortcut("Ctrl+K, Ctrl+C")`、`setShortcuts([...])`、`setAutoRepeat(...)` 和 `setShortcutContext(...)` 声明按键。字符串或 QKeySequence 的 Qt 整数构造最多四步；原生命令绑定在起始处优先，每一步最多间隔两秒。原生控件焦点决定 Widget / 子控件作用域，窗口 / 应用作用域绑定当前真实文档；文本控件保留已经消费的输入。重复动作共享实际身份，重复 / 前缀冲突正常报错，按键和点击共用候选流程。工具动作可触发已有工具或正常停用后切换，不伪造鼠标输入。

`QtGui.QKeySequence(QtGui.QKeySequence.Copy)` 取当前平台的主绑定，`action.setShortcuts(QtGui.QKeySequence.Copy)` 取该标准动作的全部候选；`keyBindings(...)` 可查询独立序列列表。标准枚举与 Qt 整数键码不混用，未绑定标准项返回空序列 / 列表。Windows、macOS、基础 X11 及 KDE / GNOME 分开选择，macOS Ctrl / Meta 和原生符号文本独立转换；符号键匹配使用实际键盘事件，不要求手动补入生成符号的 Shift。

`QtGui.QIcon(实际文件路径)` 及 `addFile(path, QSize(...), mode, state)` 可设置动作图标；空 QSize 使用文件自身尺寸。命令和表单按钮显示图标与按键说明，支持位图和 SVG / SVGZ，缺失、损坏及预算不足正常报告，不提供替代图标。旧工具类的 iconName 读取与原生工具栏相同的内嵌 PNG，文件统一位于 rust/assets/toolbar；未知名称正常报错，使用自有文件时直接设置 QIcon。API 依据 [Qt QAction](https://doc.qt.io/qt-5/qaction.html)、[QKeySequence](https://doc.qt.io/qt-5/qkeysequence.html)，SVG 解码使用 [resvg 0.45](https://github.com/linebender/resvg/tree/v0.45.1)。主题图标、QPixmap 和任意 Qt 窗口仍未迁移。新增路径未编译、执行或测试，待完整迁移后统一验证。

工具与启动脚本可用 `chunkPositions()`、`containsChunk()`、`getChunk()` 和 `sectionPositions()` 访问当前实际来源。`getSection(cy)` 在区段不存在时返回 None；数字来源的区段支持 Blocks / Data 网格，区块支持真实单值的二维数字 Biomes。视图可按坐标缓存，底层只保留一页实际数据，写入仍进入同一待确认候选。创建使用明确的原生合同；`chunk.Entities / TileEntities` 直接读取实际完整对象行，支持切片、批量删除、追加及重排，无需先密集捕获整个区块。新增路径尚待最终统一验证。

持久工具现在可在当前实际维度中读取 / 写入本地捕获以外的方块，并使用批量、填充、复制、导出和生成接口。`with dimension.edit(selection, biomes=True) as frozen:` 可明确捕获另一范围，在其中使用完整方块 / 对象 / 群系接口；无异常退出后进入同一待确认候选。直接 `getEntities / getTileEntities / getTileEntity` 使用实际范围查询，省略范围时查询实际维度；不会为对象读取构造密集方块数组。范围外对象可以逐根修改或删除；缓存引用及后代标签跟随已核实的稳定身份，写回正常核对完整旧载荷。无 UUID 对象只属于当前输入，下一输入须重新查询。所有修改仍须确认后才暂存，不直接保存；范围不存在、预算不足或来源过期正常报错。新增共享路径尚待最终统一验证。

同次工具激活中，新增实体及乘客写回后由原生回执更新最终 UUID，缓存根和已登记子对象引用可在后续输入继续使用。显式提交的位置 / ID 修改同样接续方块实体引用。回执只属于该宿主及上次实际提交；对象缺失、身份重复、外部无回执修改仍须重新查询，不按相似内容寻找替代对象。这条世界 / 结构共享路径尚待最终统一验证。

旧插件可使用 `QAction / QActionGroup` 及 `QWidget.addAction / addActions` 声明动作。动作以原生按钮行显示，保留文本、说明、启用 / 隐藏、勾选和互斥关系；实际点击调用 `triggered`，`setChecked / toggle` 只更新状态和相应信号。共享动作仍是同一个 Python 对象，不因表单回传复制。

`EditorTool.pickToolAction()`、`toolPicked` 和 `editorSession.chooseTool(name)` 会向界面请求切换。目标可以是本插件实际注册的工具，或原版已核实的 Select / Move / Clone / Brush / Flood Fill / Generate / Inspect Chunk / Inspect Block / Inspect Entity；实际操作复用原生入口。已有编辑候选先确认或取消，再点击“切换工具”；也可取消切换请求。请求待处理时停止采集旧工具指针输入，切换不隐式提交编辑；旧工具开始正常停用后，取消切换不会撤销停用或编辑候选。`getTool / tools / toolActions` 提供当前宿主中真实注册类的对象和动作，动作图标及按键见上文；原生内置工具的旧 Python 对象接口、跨激活实例及任意 Qt 窗口仍待迁移，不返回替代对象。连续折线示例已声明端点显示和切换到选择工具两个动作，新增路径待最终统一验证。

旧 `EditorTool` 类可实现 `mouseMove(event)` 更新光标或其他工具状态。宿主传入实际悬停采样，`button()` 和 `buttons()` 均为 `NoButton`，空处的 `blockPosition` 为空；不伪造点击。后台执行时后续输入进入同一实例的顺序队列，仅合并尚未执行的连续悬停。来源或参数变化后正常取消旧输入，不按新来源自动重播。没有内容修改的悬停保留已有编辑候选。世界工具与独立结构旧类都可在实际候选上继续输入，最后确认合并为一条撤销；世界的原生持久工具及一次性点选同样保留前序候选。参数变化保留未确认内容，累计内容回到原正式会话时清除空候选；其他候选须先确认或取消。连续折线示例用真实悬停移动橙色端点，多次点击可以最后一次性确认。新增路径待最终统一验证。

持久工具现在可以缓存 `editorSession.currentDimension`，区块、区段及网格引用按坐标访问每次重新捕获的真实范围，缓冲视图保留创建时的布局。实体引用使用实际 UUID；方块实体使用位置和保存 ID；库存引用使用保存槽位，乘客与属性使用已核实身份。缓存槽位表示该槽，不跟随被移到别槽的物品。对象消失、身份歧义、无稳定身份且列表内容改变或访问范围外内容时正常报错，重新查询实际来源；`copy()` 是独立载荷。新增根的最终原生身份回执及任意世界读取仍未迁完。新增路径待最终统一验证。

旧工具的 cursorNode / overlayNode 可使用 mcedit2.rendering.scenegraph.scenenode 的 Node / NamedChildrenNode，以及 mcedit2.rendering.selection 的 SelectionBoxNode / SelectionFaceNode。selectionBox 使用实际 BoundingBox，颜色为 0..1 的 RGBA；节点支持 visible、父子节点与状态，matrix 模块提供 Identity / Translate / Scale / Rotate，misc.LineWidth 控制线宽。主视图负责原生填充面和线框投影，位置、朝向、尺寸与主要颜色随实际定义显示；不要求精确复刻旧 OpenGL 特效。自定义 OpenGL RenderNode 和未转换状态正常报错，不被当作空节点。连续折线工具用橙色显示当前端点，蓝色显示上一端点，重置后清空标记。新增路径待最终统一验证。

旧自定义 Inspector 可注册继承 QWidget 并声明真实 tileEntityID 的类，构造函数接收编辑会话和私有方块实体引用。先在位置图层选中实际方块实体，再选择对应命令加载表单；字段与按钮操作先生成批量 NBT 候选，确认后才暂存。对象或暂存内容变化后重新选择命令加载当前值。原生 register_inspector 回调也可返回操作列表，无修改时返回 None；Inspector 不能同时修改选区或返回结构。控件与二进制 Qt 窗口的边界仍按已声明的原生参数类型处理，新增路径尚待最终统一验证。

旧生成器也可通过 getOptionsWidget 声明参数树。常用的 QWidget、表单 / 嵌套布局、标签、整数 / 浮点 / SpinSlider、BlockTypeButton、复选框、文本和下拉选项会转换为宿主原生表单；自定义 QWidget 可注册后作为参数树的一部分复用。信号回调、真实范围、可见 / 可编辑状态和动态参数值随同处理。单选方块继续使用原生选择器，多选按每行一个完整方块状态输入。参数变化延迟刷新完整预览，确认后才暂存；加载表单只读取当前选区的实际几何信息，不读取方块或提供实时世界权限。这些路径尚待最终统一验证。

包目录使用实际目录名作为 Python 包名。例如 `my_tools/__init__.py` 中的 `from .helpers import paint` 与 `from my_tools.helpers import paint` 指向同一份已捕获模块，不会各执行一次；子包、循环导入及 `__name__ / __package__ / __spec__` 沿用 Python 模块协议。包名须为有效模块标识，不能与宿主接口、内置或冻结标准库重名，冲突在加载时正常报告。源码仍整体捕获并参与摘要 / 重载核对，不从用户 Python 环境或当前目录寻找另一份依赖。纯 Python 依赖可随包放入子包并通过相对或完整包名导入；这不提供 CPython 扩展、Qt 或 NumPy。新增导入路径尚待最终统一验证。

在插件界面选择本目录并扫描，启用插件后加载命令。`solid_structure.py` 根据宽、高、长和方块选项生成实际结构，使用当前选区最小坐标作锚点。运行后先查看覆盖统计，再选择暂存；暂存可通过编辑撤销恢复，磁盘保存仍须单独预检和确认。目标区块和 Section 必须满足结构内核要求。

`native_context` 包提供点选替换、拖动替换、自定义名称 Inspector 和连续折线工具四个命令。选择工具并激活后，在视图点选或拖过真实源方块，结果先生成预览，确认后才暂存。拖动工具在同次激活期间记录手势次数；停用后清空。选择 Inspector 前，在位置图层选中真实方块实体；名称输入必须是该 Minecraft 版本接受的文本组件 JSON。

连续折线通过旧 `EditorTool / registerToolClass` 接口注册。先选择实际范围或拾取源方块，再激活；工具缓存同一个活动维度，每次点击连接上一个端点，按真实 Section 分批写入，最后确认暂存。“重新开始”清除起点、计数和标记。普通输入不重复执行 `toolActive`，参数和按钮不伪造鼠标事件；正常停用时执行一次 `toolInactive`，可继续使用当前真实维度及范围接口，其内容修改保留为完整候选，确认或取消后才继续已请求的切换。工具栏、快捷键及其他工具直接入口共用该交接；旧回调及宿主回收完成前不启用目标。停用回调不能重入选择新工具；取消、文档卸载和异常清理不提交编辑。选择另一插件命令保持只选择、不自动激活。本地缓冲固定在初始实际范围，范围外写入使用原生候选；范围和预算不足正常报告。新增路径尚待最终统一验证。

`@register_tool(..., gesture=True)` 接收 `context.gesture`，包含 `view` 和按顺序记录的 `samples`。每项的 `phase` 为 `press / drag / release`，并提供真实屏幕坐标 `screen`、Minecraft 方块坐标中的 `ray.origin / direction`、`shift / control / alt`。`location` 为空表示没有落点；非空时提供 `position / normal / point / state`，其中工作平面 `state` 明确为空，不能当作空气或方块。`context.selection` 是实际用户选区，无选区时为空；`context.snapshot_selection` 是本次可读写的冻结范围。范围外读取正常报错，不读取或补出未捕获内容。

加 `persistent=True` 后，原生工具在同次激活中保留 `context.state` 字典、插件模块及活动维度引用。每次回调重新绑定真实冻结来源，工具状态不会自动转为已确认编辑，也不会自动扩展读取范围；返回结果仍经完整候选、确认与历史交接。停用、切换或关闭后释放宿主，执行错误或超时后须重新激活。跨请求引用的身份与范围规则见上文。未声明持久的工具及普通命令继续使用一次性宿主。旧工具类通过上述独立适配接入，其尚未迁移的视图和来源能力不能视为已支持。

这是 RustPython 原生接口示例。旧版 PySide 控件、Qt SceneNode 和 CPython NumPy 不由这些接口冒充；非空结构实体及跨世界载荷受原生结构内核的实际校验限制。示例尚待项目最终集中验收。

## 原版随附插件

`legacy_examples` 包提供“原版选项表单示例”、“原版世界编辑示例”和“旧 Storage Drawers 四槽库存”。表单示例保留原来的全部选项与默认值，结果在日志中查看。世界编辑示例替换选区中的普通石头，并修改选区中猪的生命值和僵尸的实际 CustomName；新版名称保存为文本组件。三个修改属于同一次候选，确认后沿用原有历史与保存。

抽屉 Inspector 只用于实际 ID 为 `StorageDrawers:halfDrawers4`、Slots 保存 Item Short / Meta Short / Count Int 的旧版四槽对象。先选择“查看”读取左上、右上、左下、右下的实际数据，再选择“修改”、目标槽及要修改的字段；未勾选字段和所有附加 NBT 保持。数量可超过普通物品的 Byte 范围；不存在的槽不自动补齐，其他抽屉版本不会套用此格式。只读 Inspector 返回 None，不制造替换操作或历史；宿主拒绝 Inspector 夹带选区修改或生成结构。

包中保留原 `DrawerItemStackRef / DrawerSlotsListProxy / DrawerInventoryAttr / StorageDrawerRef` 与槽位排列供脚本使用。`raw_id` 读写实际数字，`id / itemType` 的名称转换要求准确目录声明数字映射；缺失映射正常报错，不猜测编号。新建抽屉物品先初始化真实根和 Count / Meta，再明确设置 Item；新槽位置和内容仍由调用方显式提供。Qt 控件使用原生表单与日志承接。原 count_blocks 只包含 TODO 信息框，现有 `count_states.py` 则实际统计状态。以上新增路径尚未运行，留待完整迁移后统一验证。

## 脚本标准库

普通插件与 -eval 共用 RustPython 0.5 的真实原生及冻结标准库，包括 math、random、json、collections、itertools、functools、re、struct 和 time 等基础模块。标准库随程序冻结，不依赖本机 Python 安装；用户 Python 环境、当前目录和 site-packages 不自动替换标准库或已捕获的插件包来源。这里不提供 Qt、NumPy、CPython 扩展 ABI，也未启用 SSL、SQLite 和 Tk 的可选原生实现；未提供的模块按实际导入错误报告。

logging 是标准库本身，宿主 Handler 将日志交给原有日志页，支持格式化、过滤、级别及异常堆栈。print、sys.stdout / sys.stderr 的文本写入进入同一日志通道；显式 file 参数使用实际文件流。图形脚本没有交互终端输入，请使用命令选项或显式文件；标准流不暴露插件协议管道。运行时不声称提供操作系统安全沙箱，正式世界编辑应继续使用已提供的真实候选 / 历史 / 保存接口。新增依赖及路径尚未编译或运行，待完整迁移后的最终统一验证。

## L-system 生成器

扫描本目录后，`lsystems` 包提供“科赫雪花”、“希尔伯特曲线”和“GFX 城市”。在当前真实选区选择迭代次数及方块，再生成完整结构候选；预览、确认、撤销与保存沿用现有流程。雪花在选区内绘制，希尔伯特使用从选区原点起最大的二次幂水平区域；两者默认使用选区最底层。新增路径尚未运行，留待项目最终统一验证。

脚本可从 `mcedit2.synth.l_system` 导入 `Symbol / Geometric / Fill / Line`、`applyReplacements / applyReplacementsIterated / renderBlocks`。自定义 Symbol 用 parameters 保存参数，replace 返回符号列表或迭代器；不声明 replace 的终结符保持不变，没有替换时停止。Geometric 复用实际 BoundingBox，Fill 产生盒体格，Line 调用 Rust 的三维定点 Bresenham；`mcedit2.util.bresenham.bresenham` 提供同一算法，不包含原文件的演示 / 测试函数。

`mcedit2.synth.l_system_plugin.LSystemPlugin` 继续支持 createInitialSymbol、createSymbolList、renderBlocks 和 generateInSchematic。继承它的 options 后可追加原生方块等表单字段；`iterations` 是 1..50、默认 3 的实际表单值，recursive=False 时最终生成采用 MAX_ITERATIONS。生成器先核对全部目标和状态再写私有结构；预算、取消及未知来源正常报告。旧 iterationsSlider / Qt 选项窗口不被伪装，renderSceneNodes / getPreviewNode 显式请求旧 OpenGL 节点时报告不支持；宿主预览实际生成的方块内容。GFX 城市保留种子（-2^30 到 2^30，默认 0）、楼板 / 框架 / 玻璃墙带材料及迭代，默认三轮包含完整墙面规则。城市选区宽、长至少为 8，高至少为 4，宽长之和至少为 48；不足时直接报告。相同种子及尺寸在本实现内可复现，移动世界原点只平移布局；建筑完整留在选区内，不裁墙或补来源外数据。原版固定条件下不可达的斜建筑分支没有额外伪装成可用选项。

## 分区块操作

`mceditlib.operations` 提供 `Operation / ComposeOperations`，子模块 `analyze / block_fill / entity` 提供 `AnalyzeOperation / FillBlocksOperation / blockReplaceTable / RemoveEntitiesOperation`。Operation 每次处理一个真实选中区块，返回 `(已完成, 实际区块总数)`；空范围直接结束，done 钩子只调用一次。组合操作要求同一维度引用和相同形状，逐区块先执行左侧再执行右侧，可继续嵌套。原生 -eval 维度先捕获整份来源，耗尽操作后才一次提交私有会话；提前停止不会自动提交。只读 AnalyzeOperation 不提交修改。

```python
from mceditlib.operations import ComposeOperations
from mceditlib.operations.analyze import AnalyzeOperation
from mceditlib.operations.block_fill import FillBlocksOperation
from mceditlib.util import exhaust

fill = FillBlocksOperation(dimension, selection, "minecraft:glass", ["minecraft:stone"])
analysis = AnalyzeOperation(dimension, selection)
progress = exhaust(ComposeOperations(fill, analysis))
print("进度：%r，统计格数：%d" % (progress, sum(analysis.blockCounts.values())))
```

`blockCounts` 按真实 BlockType 计数，`entityCounts / tileEntityCounts` 按实际 ID 计数。数字来源另可用 `blocks[ID | (metadata << 12)]`，是 65536 项原生整数网格；命名来源没有推测的数字调色板，请使用 blockCounts。负坐标、部分区段、真实选区掩码和未知字段保持原有冻结语义，不把未捕获格当作空气。

`dimension.fillBlocksIter(selection, block, blocksToReplace=(), updateLights=True)` 返回可迭代操作，`analyzeIter(selection)` 返回统计操作。填充也接受 `[(旧状态或旧状态列表, 新状态), ...]`，重复来源最后一项生效，空映射不改变内容；多组替换不再另加 blocksToReplace。同步 `fillBlocks` 延续本重写接口的改变格数返回值。数字 `blockReplaceTable` 为实际 4096×16×2 整数网格，支持普通索引、切片及一 / 二维坐标序列或网格的广播查询；高级坐标结果仍受最多四维网格约束，不是 NumPy ndarray。

`RemoveEntitiesOperation(dimension, selection, removeItems=False)` 保留原版 Item、TConstruct.FancyItem 和现代 minecraft:item；只按已核实 ID 判断，不猜测其他模组类别。区块 / 区段创建仍要求已有正式合同，updateLights 不绕过原生保存的必要光照处理，也不提供未捕获的 opacity / brightness 或即时重光照接口。新增路径待完整迁移后最终统一验证。

## 完整生成结构

世界和独立结构均使用原生类型表单：布尔项为复选框，选择项显示名称并保留实际值的 JSON 类型，文本支持 `placeholder`，数值支持输入 / 增减及明确上下界的范围滑条，方块项可直接输入完整状态或打开所属文档的选择器。`increment` 驱动实际步进；字段无效时保留草稿并报告。切换命令 / 插件定义或文档后，旧控件及选择器不能写入新选项；更改选项沿用已有任务取消与候选撤回。新增路径待项目最终统一验证。

旧 filter 选项支持 `('说明', 'label')`、`('文本', 'string')`、`('文本', ('string', 'value=默认值', 'width=400'))`、`('方块', ('block', 'minecraft:stone'))`、实际 BlockType、字符串选择，以及 `(最小值, 最大值)`、`(默认值, 最小值, 最大值[, 步长])`。布尔值按布尔选项处理。旧数字默认值可明确传 `(ID, metadata)`，只表达该数字状态，不推测跨版本名称映射。方块值在执行时按实际维度注册表解析；现有原生字典选项继续可用，步长保留在元数据中。`mcedit2.plugins.command` 提供原有 `dictFromFilterTuple` 和 `optionsFromFilterInputs`，未知格式正常报错。

生成器也可直接编辑当前冻结维度。旧 `GeneratePlugin.generate(bounds, blocktypes)` 返回 None 时，宿主调用该类实际声明的 `generateInWorld(bounds, dimension)` 或 `generateInDimension(bounds, dimension)`；原版调用代码用前者、基类文档用后者，两者均有时以前者为准。回调只修改传入维度，返回 None 或最终返回 None 的进度迭代器。原生 `register_generator` 可直接修改 `context.dimension` 并返回 None。结果先形成真实完整候选，确认后一次撤销；所有读取 / 修改仍限定当前冻结选区及其形状。

命令、表单、工具、Inspector 和生成器均可返回进度迭代器，用 yield 报告进度，用 return 返回该类别原本支持的结果；生成结构内部的 `generateInSchematic` 任务也会执行完成。普通命令与生成器可返回 `createSchematic` 或完整原生结构字典。宿主记录最后进度和总步数，不逐次占用日志条目；异常正常报告，失败不发布部分源修改。`mceditlib.util.exhaust(task)` 保留原版语义，返回最后一次 yield，空迭代返回 None；它不返回任务的最终 return 值。

原生预览显示实际生成内容，不执行可选 `getPreviewNode` 的 Qt 场景节点。`getOptionsWidget` 及 Qt 异步预览通知仍未支持，选项应使用现有 `options` 原生表单。上述新增路径待项目最终统一验证。

`mcedit2.api.create_structure(shape, context.dimension.blocktypes)` 与旧入口 `mceditlib.schematic.createSchematic(shape, blocktypes)` 使用当前文档的实际注册表，返回独立局部结构。必须传入实际 `blocktypes`；不把字符串 `Alpha` 或其他名称解释为未经核实的版本 / 注册表。旧式 `GeneratePlugin.generate` 继续调用 `generateInSchematic`，生成器也可覆盖 `generate` 并返回该结构；普通命令可直接返回它。

`schematic.getDimension()` 的 `bounds` 是局部范围，起点为零；可使用既有方块、实体 / BE 和群系接口。新结构初始化为空气，对象列表为空，不继承源选区的稀疏掩码或群系。新增对象须提供真实类型化 NBT 和局部位置；现存根引用可先用 `copyWithOffset` 变为生成结构的局部坐标，再加入新维度。完整乘客 / Riding、未知字段和实际画作锚点由原生预检保留，UUID 及整组引用在正式放置时统一处理。

可通过关键字 `biomes` 或 `numeric_biomes` 明确传入完整群系网格，`mask` 提供稀疏定义，`metadata` 提供 `TAG_Compound`；这些输入由新结构独立持有。`schematic.offset` 是实际放置偏移。返回时完整方块、对象、群系、掩码及元数据共同物化为原生结构并进入现有候选，确认后一次撤销。普通命令或生成器也可同时修改冻结源：先应用源修改，再放置返回结构，重叠格以生成阶段的实际写入为准；两步共同显示为完整候选，一次确认和一次撤销，后段失败不会只发布源修改。新增路径待项目最终统一编译与测试。

## 冻结维度复制、导入与导出

`mceditlib.selection` 提供 BoundingBox、FloatBox、ZeroBox、SectionBox、UnionBox、IntersectionBox、DifferenceBox 和 InvertedBox；`mceditlib.selection.hollow.HollowSelection` 取得实际形状的六邻接外壳。`a & b`、`a | b` / `a + b`、`a - b` 保留逐格选中状态；`a.intersect(b)` / `a.union(b)` 返回几何方盒，`expand` 返回扩缩后的方盒。`origin / size / maximum / center`、区块 / 区段坐标、`chunkPositions / sectionPositions / chunkBox` 使用真实坐标；最后一个接口读取传入维度的实际高度。反选保留原版来源区块 / 区段枚举范围，未声明区域不会被补成空气。起点与尺寸不可直接改写，应构造新选区。

`box_mask / section_mask` 返回实际布尔网格或没有选中格时的 None，网格按 Y/Z/X 排列，提供 `shape / ndim / size`、整数 / 切片 / 省略号 / `None` 新轴索引、整数坐标网格和布尔掩码索引、布尔赋值、`& | ^ ~`、比较、原位逻辑运算及 `nonzero / any / all / sum / tolist / ravel`。`nonzero` 返回各轴的一维原生整数网格，可迭代，也可直接加减区块偏移；`contains_coords` 接收标量、规则的一至四维 Python 序列或原生数值网格，共用尾轴广播。

掩码与数值网格共用按轴归并和索引：`any / all / sum(axis=None, keepdims=False)` 支持单轴、负轴和轴元组；成组坐标尾轴广播，混合切片时按连续 / 分隔的高级索引安排结果轴。基本切片和 `ravel()` 共享实际来源，高级索引读取独立值，`array[index] = value` 仍写回原来源；赋值先冻结右侧，再广播，保留重叠切片语义。规则参照 [NumPy 索引文档](https://numpy.org/doc/stable/user/basics.indexing.html)与[归并文档](https://numpy.org/doc/stable/reference/generated/numpy.any.html)，原生结果最多四维并使用任务预算和取消。这些接口没有安装 NumPy、任意 ufunc 或二进制 ABI；依赖 NumPy 模块的外部回调须改用原生运算。新增路径待最终统一验证。

`mceditlib.selection.ShapeFuncSelection(box, shapeFunc)` 已接原生形状回调。`shapeFunc(blockPositions, selectionSize)` 的坐标为相对选区起点的 Y/Z/X 浮点网格，方盒请求的形状为 `(3, 高, 长, 宽)`；尺寸是 Y/Z/X 三项浮点网格。回调返回与请求空间形状相同的布尔网格 / 规则布尔序列，或以 None 表示没有选中格。结果裁到实际选区方盒；标量成员查询按所在方块判断，批量坐标查询保留广播后的形状。

坐标计算提供有限 binary64 数值、四则运算、幂、比较、`abs(grid)`、`grid.sqrt()`、按轴归并和已有切片 / 广播。整数网格与浮点数运算生成独立浮点计算结果；方块 ID / metadata 写回仍要求整数，不截断浮点数，整数幂按 int64 逐步检查溢出。形状、组合、反选和空心选区使用真实批量掩码，单次线性掩码读取只物化一次；填充、快照和复制沿原有候选交接，不把自定义掩码冒充原生 UI 的固定几何类型。

例如无需 NumPy 的椭球回调：

```python
from mceditlib.selection import ShapeFuncSelection

def ellipsoid(positions, size):
    radius = size / 2.0
    positions -= (radius - 0.5)[:, None, None, None]
    positions /= radius[:, None, None, None]
    return (positions * positions).sum(axis=0) <= 1.0

shape = ShapeFuncSelection(context.selection, ellipsoid)
context.dimension.fillBlocks(shape, "minecraft:stone")
```

例如 `HollowSelection(context.selection)` 可直接用于冻结维度的 `fillBlocks` 或 `copyBlocks`，形状和孔洞随真实掩码进入原生完整候选；启动维度的任意形状填充也复用冻结差异。空方盒填充不产生修改，给启动会话赋空当前选区会清除选择。正式 UI 选区仍须是已有原生形状；任意组合形状用于冻结编辑和载荷，不伪装成 UI 已支持的几何类型。上述新增路径待最终统一验证。

`mceditlib.geometry.Ray` 提供 `fromPoints / atHeight / intersectPlane`，`selection.rayIntersectsBox` 返回按前向距离排序的 `(Vector, Face)` 命中或 None。FloatBox 用于几何查询，不把浮点范围直接转换成方块写入；无交点返回 None，零方向报错。

`dimension.copyBlocksIter(sourceDimension, selection, destination, blocksToCopy=None, entities=True, create=False, biomes=False, updateLights="all", replaceUnknownWith=None, copyAir=False)` 使用 Rust 完整复制内核；`copyBlocks(...)` 完成迭代并返回最后进度。模块入口 `mceditlib.block_copy.copyBlocksIter(destDimension, sourceDimension, ...)` 使用同一路径。源和目标必须属于本次实际注册上下文，可以是当前冻结维度或 `createSchematic(...).getDimension()`。

`blocksToCopy` 为 None 时不筛选非空气；旧数字 ID 选择该 ID 的全部 metadata，BlockType 或状态文本选择完整状态。空气独立由 `copyAir` 控制；BE 及物品随实际复制的方块覆盖，普通实体由 `entities` 开关控制。实体整组复制会分配新身份并迁移已有引用，源 / 目标相同和重叠也先冻结原稿。`biomes=True` 需要实际捕获群系；已有编码与相位由原生内核处理，目标选区外的缺失数据不会被猜测。

`dimension.exportSchematic(selection)` 返回完整局部结构，保留形状及未声明格；`exportSchematicIter` 先产生进度，再产出并最终返回该结构。`dimension.importSchematicIter(schematicOrDimension, destination)` 导入实际已有载荷；和原版一样，`importSchematic` 也返回工作迭代器，须用 `exhaust` 或返回给宿主完成执行。维度的 `bounds / getDimension()` 只暴露真实冻结范围和唯一捕获维度，`SelectionBox / BoundingBox` 使用现有 Vector 坐标。

`create=True` 不创建未捕获的世界区块，也不突破目标范围或稀疏声明；生成结构应先明确创建所需尺寸和掩码。正式世界照明更新由原生候选与保存合同处理，不在冻结维度执行旧 relight。未知注册表不转换，`replaceUnknownWith` 不作为替代策略。所有操作仍只改私有数据，最后经完整预检、确认和一次历史发布；新增接口待项目最终统一验证。

## 选区对象与 NBT

冻结维度与启动脚本维度提供 `getBlocks(x, y, z, return_Blocks=True, return_Data=False, return_BlockLight=False, return_SkyLight=False, return_Biomes=False)` 和 `setBlocks(x, y, z, Blocks=None, Data=None, BlockLight=None, SkyLight=None, Biomes=None, updateLights=True)`。坐标和值可用标量、规则的一至四维 Python 序列或现有整数网格；读取按单轴广播保留请求形状，写入按原版先在尾部补足三维再广播。结果是 `mceditlib.fakechunklevel.GetBlocksResult` 五字段元组，未请求的字段为 None，其余数字字段为独立网格。这里没有 NumPy 数组或二进制 ABI。

数字 Blocks 写入保留当前 metadata，Data 可单独写；完整 BlockType / 状态文本保留自己的 metadata，命名完整状态不与旧 Data 混用。重复位置按请求顺序保留最后结果；全批位置、来源和字段先核对再修改。Biomes 使用真实数字单值列，改变整个实际捕获列；多值三维列及命名群系使用原 `getBiome / setBiome`。启动批量调用按实际掩码捕获，群系列取实际维度高度；范围过大须明确缩小批次。未捕获光照的读写报错，updateLights 不覆盖原生保存的必要光照处理，也不提供即时 relight。

`mceditlib.multi_block` 同时提供 `getChunkBlocks / setChunkBlocks`（X/Z 为 0..15 局部坐标，Y 为实际维度坐标）、`getSectionBlocks / setSectionBlocks`（XYZ 为 0..15 局部坐标）、`coords_by_chunk`、`atleast_3d` 和 `maskArray`。区段读取按原接口返回四项列表；维度 / 区块读取返回五字段元组。新增接口仍待最终统一验证。

冻结维度提供 `chunkPositions / chunkCount / containsChunk / getChunk / getChunks / listDirtyChunks`。`getChunk(cx, cz)` 复用本次捕获的实际视图，`chunk.bounds / chunkPosition / dimName / dimension / worldEditor / dirty` 指向当前冻结来源。`sectionPositions / getSection(cy)` 只暴露实际声明格所在区段，负 Y 正常保留；显式请求未捕获的区块 / 区段报错，`create=True` 不创建未知世界数据。独立结构也可按实际来源坐标分组查询，返回值不代表地形 Region 或原文件的完整区块 NBT。

区段 `getBlock / setBlock(x, y, z, ...)` 使用 0..15 局部坐标。数字来源的 `Blocks / Data` 网格形状为 `(16,16,16)`，索引按 Y/Z/X；可用 `dimension.selection.section_mask(cx, cy, cz)` 筛选真实捕获格后操作。基本切片共享原区段，布尔或成组整数高级索引读取独立数据，`copy()` 显式复制；索引赋值和原位下标更新仍作用于原区段。提供布尔比较、整数算术 / 位运算和原位更新、尾轴广播赋值、`fill / flat / ravel / nonzero / tolist / any / all / sum`，归并与索引沿上面的共同规则。整数单元明确为 int64，写回方块 / metadata 时继续核对实际 ID / nibble 范围，不做截断或环绕。未捕获格不能按空气读取，命名来源改用真实 getBlock / setBlock，不推测数字注册表。

`chunk.Biomes` 仅用于本次已捕获的实际数字单值列，按 Z/X 访问，写入保留 i32 和选区掩码；多值三维列不能压成二维，命名群系使用维度的 `getBiome / setBiome`。这里没有 NumPy，整数网格不提供任意广播、浮点 ufunc 或二进制 ABI；没有捕获完整区块 NBT、光照、HeightMap、TileTicks 或 TerrainPopulated，不提供虚假默认数据。以上新增路径待最终统一验证。

例如已捕获数字群系的区段可沿用原版列投影：`mask = selection.section_mask(cx, cy, cz)`；非空时用 `z, x = mask.any(axis=0).nonzero()` 取得实际选中列，再执行 `chunk.Biomes[z, x] = biome_id`。数字方块坐标同样可直接以 `section.Blocks[mask]` 读取，或用 `section.Blocks[mask] = identifier` 写回；不会将读取所得副本的后续独立修改悄悄写回原区段。

`chunk.Entities / TileEntities` 返回共享真实记录的引用列表，可读取、切片及批量修改，追加 / 插入新根、删除实际引用或重排现存根；反转及切片赋值的实际顺序通过原始载荷置换进入历史与保存。世界按各自真实区块整理，独立结构保留文件全局序列；未参与的根保持原槽，完整对象内容与 UUID 不因排序改变。`addEntity / addTileEntity` 将独立引用绑定到本次实际快照，调用后继续修改同一引用仍改变新增内容；重复加入同一活动根不增加副本，跨来源须明确复制并满足实际注册上下文。`removeEntities` 在全部成员核对后一起移除。单树 `copy / copyWithOffset` 由 Rust 为每次复制分配独立身份并重映射树内引用，原根不变；正式发布仍核对世界身份。现存根指向新增树的引用共用最终导入 UUID 和实际位置映射，现存树自身身份保留；原目标已有的来源 UUID 仍指向原对象，不悄悄改指副本。世界与独立结构沿同一次正式候选 / 历史发布，新增路径待最终统一验证。

`mceditlib.anvil.entities` 提供内建生物、属性 / 修饰器 / 效果、命令统计、挂画 / 展示框和旧容器 / 告示牌 / 命令块引用，以及 ItemRef、ItemStackRef、SlotsListProxy 和 EntityPtr。`PCEntityRef / PCTileEntityRef` 按实际保存 ID 构造引用；查询获得的专用视图与原记录共享完整 NBT，不另外保存一份对象。`Passengers` 的子引用及嵌套列表编辑作用于父根，未知字段继续保留。声明的默认值按原接口写入真实私有标签；没有声明的字段不会用假数据替代。现代村民和告示牌不套用旧 Profession / Text1..4 默认类。

容器可用 `container.Items`、`getItemInSlot(slot)`、`createItemInSlot(slot)` 和 `putItemInSlot(stack, slot)`。新物品绑定实际父文档编码，`Count` 访问真实 Count Byte 或 count Int；`Damage` 根据准确来源访问根 Short、tag/Damage 或 components/minecraft:damage。物品 `tag` 仍指真实保存的旧 tag Compound，组件版请通过完整 rootTag 编辑 components。新建独立物品首次设置 id 需要真实父库存，或用 `raw_id` 明确声明完整编码；跨库存活动引用先 copy，不隐式转换不同版本的物品。

`dimension.blocktypes.itemTypes` 读取与桌面同源的准确 1.12.2 / 1.20.1 / 1.21.1 目录，支持命名 ID、旧实际变体和数量 / 损耗元数据。ItemType 的 `displayName` 是源目录的本地化键，不是已翻译文本；当前数据没有数字 ID 时，数字查询 / ID 转换明确报错，不借用方块 ID 或目录顺序。`worldEditor.createEntity(id)` 为已声明的类型创建有实际来源上下文的独立引用，仍须设置真实位置 / 必需字段并显式 addEntity；UUID 由既有原生新增证明处理。`EntityPtr` 只查询当前冻结维度的实际范围，不能访问快照之外。`mceditlib.faces` 提供六向 Face 和真实单位 Vector。以上新增接口待最终统一验证。

普通命令可通过 `context.dimension.getEntities(context.selection, id="minecraft:zombie")` 遍历选区中的真实根实体，通过 `getTileEntity((x, y, z))` 查询方块实体；没有对应对象时返回 `None`。`getTileEntities` 支持相同范围和字段筛选。返回引用的 `rootTag` 为完整类型化 Compound，包含保存的乘客 / Riding 子树、物品及未知字段。

```python
from mcedit2.api import register_command
from mceditlib import nbt

@register_command(name="保留选区中的僵尸")
def preserve_zombies(context):
    for entity in context.dimension.getEntities(context.selection, id="minecraft:zombie"):
        entity.rootTag["PersistenceRequired"] = nbt.TAG_Byte(1)
```

已存在字段可通过 `rootTag["字段"].value` 修改；新增字段明确使用 `TAG_Byte / Short / Int / Long / Float / Double / String`，数组使用 `TAG_Byte_Array / Int_Array / Long_Array`，容器使用 `TAG_Compound / TAG_List`。`TAG_List(..., list_type=...)` 保留空列表的实际元素种类。Compound 提供键访问、插入 / 删除、遍历及复制，List 提供索引、插入 / 删除及追加，数组使用 Python 整数列表；这些接口不声称提供 NumPy 运算。Long 及 LongArray 保持整数精度。

对象提供 `Position`、`id`、`Rotation / Motion` 和实际已保存字段。`Position` 修改该根的保存位置；若要让整棵实体树及所有挂载 / 空间载荷共同平移，继续使用原生移动工具。`copy()` 产生独立载荷，使用 `dimension.addEntity(ref)` 添加复制根，`removeEntity(ref)` 删除实际引用；BE 使用 `addTileEntity / removeTileEntity`。新增实体由原生导入内核分配新 UUID 并处理内部引用，不允许用普通字段编辑绕过现存实体的身份校验。遍历时删除可先用 `list(...)` 冻结引用列表。

群系需先打开页面的捕获开关。`getBiome(x,y,z) / setBiome(x,y,z,value)` 读取或修改实际逐格样本，命名值使用注册 ID 字符串，数字值使用整数；写入结果须满足目标整列或同值完整四格的表达约束。旧 `getBiomeID(x,z)` 只接受真实数字且该列所有选中样本相同，`setBiomeID(x,z,value)` 修改选区中的整列样本。未捕获群系、越界、部分四格或混合列正常报错，不使用默认 ID。`dimension.data_version / source_release` 暴露实际来源版本。

保存为四格网格的命名群系按真实来源 XYZ 相位读取，只读或同值写入不改网格。首次逐格修改会在 Rust 中把真实保存样本展开成独立副本，再修改目标格；其余值保持，正式世界若不能无损表达该单点 / 部分四格修改会正常报告，须明确写满所需范围。插件生成结构和启动脚本冻结编辑使用相同规则与工作预算；数字值保持原 i32 编码及版本。方块和群系修改会更新冻结区块的脏提示，正式候选 / 历史和磁盘保存仍按实际数据核对，不能用提示代替确认。

所有源读取与修改限定本次冻结选区及其形状；对象列表、NBT、方块和群系一起复核来源，先显示完整候选，确认后一次暂存和一次撤销。普通命令 / 生成器可同时返回源修改和结构，生成结构按其实际范围另行验证；Inspector 仍只返回对应真实对象的操作，不能夹带结构或其他快照修改。协议 4 的这些新增路径待最终统一编译与测试。

## NBT 文件与缓冲区

`mceditlib.nbt.load(filename="", buf=None)` 从实际路径、二进制流当前位置或 bytes / bytearray / memoryview 读取 Compound 根，自动识别 raw、GZip 和 Zlib。根名称、Modified UTF-8、所有整数 / 浮点宽度、数组及空列表元素类型由共享 Rust 编解码器处理；损坏编码和尾随数据正常报错，不在失败后尝试其他格式。`NBTFormatError` 继承 `mceditlib.exceptions.LevelFormatError`；文件系统异常保持原类型。

```python
from mceditlib import nbt

root = nbt.TAG_Compound([nbt.TAG_Long(9007199254740993, name="counter")], name="示例")
data = root.save(compressed=False)
copy = nbt.load(buf=data)
print(copy.name, copy["counter"].value)
```

`root.save()` 默认返回 GZip bytes，`compressed=False` 返回 raw bytes；传入路径则明确写该文件，传入二进制输出流则在当前位置写入。只关闭函数自身打开的文件，编码成功后才打开输出，短写正常报错。外部文件 I/O 不自动加入世界 / 结构历史；正式世界修改继续使用已有编辑及事务保存接口。文件输入、解压结果、树节点和 VM 工作集受真实预算约束，文档单项编辑的 1 MiB 限制仍保留。

`gunzip` 要求真实 GZip，`try_gunzip` 仅在没有 GZip 标记时返回原 bytes；损坏的压缩流不会吞错。`walk(tag)` 依次返回 `(字段名或列表下标, 标签, 父路径列表)`，不把数组标量冒充子标签；`nested_string / str(tag)` 提供树诊断文本，`hexdump(data, length=8)` 提供二进制诊断文本。`isList / isCompound`、列表的 `tagClass`、`tag_classes`、Compound 的命名标签序列构造 / `add / get_all` 共用真实类型。修改子标签的 name 会同步字段键，重复名称报错；Float 的 value 使用实际 f32 值，相等判断包含标签类型和内容。新增路径待完整迁移后最终统一验证。

## 属性描述器与自定义引用

`mceditlib.nbtattr` 提供 `NBTAttr / NBTListAttr / NBTCompoundAttr / NBTCompoundListAttr / NBTVectorAttr / KeyedVectorAttr / NBTUUIDAttr`，以及 `NBTCompoundRef / NBTListProxy / SetNBTDefaults`。类型简写为 `b/s/i/l/f/d/t`，也可直接传对应 `nbt.TAG_*` 类。已有字段必须匹配声明类型；缺失标量、列表或向量没有明确默认值时正常报错。读取声明的 Compound / Compound 列表会按其类型创建空容器；明确声明的默认值会初始化相应标签，这类更改仍需最终确认暂存。

```python
from mcedit2.plugins import registerTileEntityRefClass
from mceditlib import nbtattr
from mceditlib.anvil.entities import PCTileEntityRefBase

class ChestRef(PCTileEntityRefBase):
    Items = nbtattr.NBTCompoundListAttr("Items", nbtattr.NBTCompoundRef)

registerTileEntityRefClass("minecraft:chest", ChestRef)
```

注册只选择实际保存 ID 对应的引用类，不注册游戏对象或渲染器。随后 `dimension.getTileEntity(position)` 返回相应视图，其 `rootTag` 与原快照共享真实记录；列表条目是具有父引用的完整 Compound。向另一列表插入已有条目时先 `copy()`，避免混淆所属对象；支持索引 / 切片、插入、删除、追加、反转、查找和计数。`SetNBTDefaults(ref)` 按继承后的实际声明初始化缺失字段，不覆盖已有载荷。注册类的 `create()` 和 `PCTileEntityRef.create(ID)` 只按该类的明确声明构造；没有注册构造类的未知 ID 正常报错。

`PCEntityRefBase.create(ID)` 可开始构造带实际 ID 的独立根，位置与其他所需保存字段须明确填写；不从 ID 猜专用载荷。`copyWithOffset((dx,dy,dz))` 接受整格偏移，调用共享 Rust 变换器处理完整树及已核实的空间 / 关联字段；来源版本来自实际快照，未知语义正常报错。新增实体仍由导入内核分配新 UUID，普通属性编辑不能绕过身份校验。`NBTUUIDAttr` 返回可读取 `int / hex / bytes` 的 128 位值，支持实际四项 IntArray 或两个 Long 的存储，不冒充完整 Python `uuid` 模块。

`mceditlib.geometry.Vector` 提供坐标访问、逐项加减 / 乘除、缩放、长度 / 归一化、叉积、向下取整和区块位置；无需 NumPy。对象的 `chunk` 仅表示本次冻结选区中的实际坐标、维度和修改上下文，不提供完整世界区块数组或 NBT。上述新增路径均待最终统一验证。

## 命令行启动脚本

`with dimension.edit(box, biomes=True) as frozen:` 的冻结维度现与插件共用完整复制、结构导入 / 导出接口。`createSchematic(shape, frozen.blocktypes)` 使用该维度已实际捕获的来源；应先进入冻结编辑，不能传未核实的注册表名称。生成结构和旧冻结对象各保留创建时来源，同一实际维度的不同捕获可复制，根序号分别绑定各自原始 NBT；跨世界 / 维度注册表转换不据相似版本推测。

```python
from mceditlib.schematic import createSchematic
from mceditlib.util import exhaust

with dimension.edit(box) as frozen:
    schematic = createSchematic((4, 4, 4), frozen.blocktypes)
    generated = schematic.getDimension()
    generated.fillBlocks(generated.bounds, "minecraft:stone")
    exhaust(frozen.importSchematic(schematic, box.origin))
```

目标冻结范围须容纳所导入结构；`importSchematic` 和 `importSchematicIter` 都返回待执行迭代器，`copyBlocks` 则完成复制。`exportSchematic` 返回独立局部结构，保留形状 / 稀疏声明；实体、BE、过滤、空气和已捕获群系遵循前述共同接口。原生快照由真实租约保留，生成结构或冻结对象仍引用时保持有效，最后引用释放后归还来源；保留来源与本次操作一起计入预算。冻结修改在 `edit` 正常退出时交给私有会话，脚本整体成功后才按原流程发布；异常或取消不发布前半段。新增及共用路径待最终统一验证。

实际世界维度也提供相同的复制和结构导入 / 导出接口，无需先建立目标冻结数组。下面的 `box` 应为实际来源范围，`destination` 为用户确定的目标最小坐标：

```python
dimension = session.currentDimension
dimension.copyBlocks(dimension, box, destination, entities=True, copyAir=False)

schematic = dimension.exportSchematic(box)
exhaust(dimension.importSchematic(schematic, destination))
```

`mceditlib.block_copy.copyBlocksIter` 同样按实际目标类型执行；世界源在改写目标前冻结，支持重叠复制、当前私有修改及已有生成结构。命名来源只有在实际版本 / 注册表可核对时才能跨世界使用，旧数字来源不推测跨世界 ID。冻结目标仍只允许同一真实来源的范围内操作；`create=True` 不扩大冻结目标。

世界目标缺少区块或 Section 时，明确提供创建合同。例如 `profile = dimension.creationProfile("minecraft:plains")` 使用用户指定的初始群系，并读取该实际维度的高度、天空光和数据包来源；旧数字来源传实际整数群系 ID。随后使用 `dimension.copyBlocks(source, box, destination, create=True, creation=profile)`，或 `exhaust(dimension.importSchematic(schematic, destination, creation=profile))`。普通完整目标不要求合同；未知维度几何、缺失合同、不匹配版本或不能表达的群系修改正常报错，不用默认群系创建。创建和复制均只进入私有会话，脚本成功后统一发布，保存仍显式调用 `session.save()`。

`dimension.bounds` 扫描实际地形槽和暂存创建，返回 XZ 包围范围及真实维度高度的 `SelectionBox`；空维度返回 `None`，不分配密集方块数组。`session.selectAll()` 将该范围设为当前方盒选区，空维度则清除。范围内可能有未保存地形的空洞，选择不创建区块；后续读取或编辑仍遵守各自来源与预算约束。未知高度正常报错，不猜默认维度；新增路径待最终统一验证。

同一 `mcedit2` 程序使用 `-eval CODE 文档路径...` 启动 GUI，文档可为实际世界或支持的独立结构。`session` 是启动时选择的实际会话，`self.sessions` 保留顶层标签顺序，`self.currentSession()` 返回该会话；不把 NBT 子标签冒充地形文档。脚本等待所列文档读取、资源解释及恢复状态查询完成后后台执行一次。旧 schematic 须在文件页明确选择实际来源版本并读取；失败、关闭或取消正常报告，不跳过输入或改用后台世界。不与 `--inspect` 同时使用。

世界分支直接使用私有原生世界会话，能读取选区外的实际位置；结构分支使用真实文件文档，只访问实际文件范围及声明格。两者共用 `currentDimension`、选区、读写、完整冻结编辑、复制 / 结构交换和 `save()`；结构只有自身实际维度，`worldEditor.getDimension()` 不带名称时返回该会话的当前维度。结构脚本坐标保留文件的真实来源原点，`dimension.bounds` 返回该来源范围；界面在发布时转换为局部坐标，不把文件当作世界地形。结构不能使用世界区块创建合同，也不会因 create 参数扩大文件尺寸；未知版本 / 注册表及不可表达的载荷正常报错。

结构中的多个私有步骤在整段成功后折叠为一次文件撤销；无最终变化保留原历史和 redo，世界仍按各修改维度一次撤销。全部输入在整组发布前核对实际来源、修订及解释定义；用户后续改变的选区草稿或相机不被旧任务覆盖。`session.save()` 的世界 / 结构请求按实际顺序执行：世界走混合事务，结构走已有文件保存和恢复流程，未请求的文档不写盘。保存失败后保留已发布编辑及实际记录，不自动重试或跳过。以上新增与共用路径待最终统一验证。

```python
from mceditlib.selection import SelectionBox
from mceditlib import nbt

dimension = session.currentDimension
box = SelectionBox((0, 64, 0), (16, 8, 16))
session.currentSelection = box
dimension.fillBlocks(box, "minecraft:stone")
with dimension.edit(box, biomes=True) as area:
    for entity in area.getEntities(box, id="minecraft:zombie"):
        entity.rootTag["PersistenceRequired"] = nbt.TAG_Byte(1)
session.zoomToPoint((8, 68, 8))
# 需要写盘时显式调用；省略则保留为可撤销的内存编辑。
session.save()
```

`getBlock / setBlock / getBlockID / getBlockData / setBlockID / setBlockData` 与 `fillBlocks` 使用真实维度；数字 ID / Data 只适用于实际数字编码，方块名通过准确来源版本的注册表解析。`fillBlocks` 可传 `blocksToReplace` 状态列表，保留形状外方块。缺失区块 / 区段、未知注册表和越界正常报错，不用空气替代真实缺失数据；示例范围须位于实际已保存区段。

NBT、实体、BE 和群系通过 `with dimension.edit(box, biomes=...) as area` 显式取得完整冻结内容，使用上文相同对象接口；退出上下文时核对来源并合并差异。群系默认不捕获，开启后遵循真实整列 / 四格覆盖。独立上下文不会将未提交的对象更改隐式暴露给其他读取；重叠修改导致来源过期时报告错误。实时维度对象不伪装成旧版自动写回的实体 / NumPy 区块引用。

`session.worldEditor.getDimensionNames() / getDimension(name)` 返回真实维度，`session.gotoDimension(name)` 选择脚本会话的维度；`currentSelection` 可读写或设为 `None`，`deselect()` 清除。`deleteBlocks / deleteEntities / deleteEntitiesAndItems / deleteSelection` 对应四类原生删除。`zoomToPoint(point, distance=15)` 定位到实际坐标。界面维度和定位跟随启动时的当前世界，不自动激活其他世界标签；执行期间用户改变相机或文档后不再跟随旧任务，选区分别按实际维度核对是否已被用户修改。

脚本整体成功后，全部维度重新重放领域证明，核对源修订，再一次发布；每个修改维度只增加一次撤销。异常或取消不发布脚本修改。`session.save()` 合并同一世界的请求，在脚本成功后保存最终脚本状态，复用原生混合事务、备份与实际恢复记录；不等价于旧版在 Python 代码中交错即时写盘。若执行期间来源改变，报告冲突；保存失败后保留已暂存编辑和真实恢复记录，由保存页明确完成或回滚，不自动重试。未显式请求的世界不写盘，多个世界按实际请求顺序保存，磁盘事务以世界为边界。

`print` 和 `logging` 实时进入运行日志及日志底栏，终端 INFO 输出使用 `-v`。界面显示执行 / 保存状态，纯执行可取消；开始持久事务后沿用文档关闭保护。失败、取消或关闭前未完成使程序返回失败状态；完成后 GUI 保持打开供查看。源码提供随程序冻结的 RustPython 标准库；不提供 Qt、NumPy 和任意旧版 GUI 方法，不声称操作系统安全沙箱。以上新增路径尚未编译、执行或测试，留到完整迁移后的最终统一验证。
