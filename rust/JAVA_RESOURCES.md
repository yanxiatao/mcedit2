# Java 模型与图片元数据读取

## 剩余投射物与位置辅助

静态读取 1.21.1 gnm / cod 的危险骷髅头、gka 龙火球、gni 风弹及 btu 不祥物品生成器的实际图片、模型与 item 字段。bsn 的 Radius 在现代原生 setRadius 中限制为 0..32，交互实体使用实际 width / height。无普通模型的对象以编辑器位置 / 范围辅助显示，不模拟粒子或未保存的随机分支；浮标不从不存在的保存物主关系臆造钓线。

旧 cbl 明确实例化默认 bqv 凋灵骷髅头模型，该层加入完整提取计划至 61 层，未执行。旧注册目录确认经验球为 xp_orb，共享判断已补齐此身份。新增实现未编译或测试。

## 末影龙、哞菇与巨人僵尸

静态读取 1.21.1 gke$a / glq / goq / cfr / gku / fvp：龙重复提交颈部模型构成五节脖颈和十二节尾部，哞菇 Type 及三个蘑菇方块位置 / 头部挂接，巨人使用非攻击态 AbstractZombieModel、手持与护甲层。静态展示保留完整轮廓和源朝向，不恢复飞行历史或逐帧动作。

旧 bzc 使用 brn；其公开模型注册顺序是 head / jaw / neck / body / wing / wingtip / frontleg / frontlegtip / frontfoot / rearleg / rearlegtip / rearfoot，原生重复镜像单侧肢体。该模型加入完整提取计划至 60 层，未运行生成器。旧哞菇图片为 entity/cow/mooshroom，现代为 red_mooshroom / brown_mooshroom；旧哞菇复用原生 CowModel。旧 acz 巨人将 vg 构造的 .6 / 1.8 尺寸乘六，眼高常量 10.440001；现代沿实际注册尺寸与版本眼位。新增实现待最终统一编译测试。

## 下落方块、TNT 与展示实体

静态读取 1.21.1 cjg / cji 保存字段：下落方块 BlockState，TNT block_state；旧 vi 注册 ack / acm，ack 按 Block、TileID、Tile 和 Data 解释状态。显示复用既有方块资源模型，不模拟运动或引信效果。

现代 Display / BlockDisplay / ItemDisplay / TextDisplay（1.21.1 bsq 及其 b / g / l 子类）保存 transformation、billboard、brightness 与各自 block_state、item / item_display、text 等字段。新增接线读取真实内容、分解变换 / 仿射矩阵、资源字体、背景与透明度；固定保存朝向，不把静态展示称为原客户端的相机约束、插值或混淆字形动画重建。透视矩阵明确报告未支持。亮度覆盖通过共享探针返回实际保存的 block / sky，不读取无关世界通道；结构预览也保留该覆盖。新增实现未编译或测试。

## 物品投掷实体

九类实体复用物品模型而不是另造图片。静态读取现代 ThrowableItemProjectile、Fireball、EyeOfEnder、FireworkRocketEntity 以及对应物品 renderer：现代 Item、烟花 FireworksItem、空载荷的原生默认物品分别解释；1.21.1 对应 cnw / cnj / cni / cnk。旧 vi 注册表确认 xp_bottle、eye_of_ender_signal、fireworks_rocket 等身份，aez 药水读取 Potion，aem 烟花读取 FireworksItem，旧其他固定物品实体不读取现代 Item 字段。

实际物品载荷沿既有库存 / 物品外观 / 资源模型逻辑，保留药水颜色、自定义模型和特殊材质。编辑器使用保存朝向下的一份静态模型，不追踪相机旋转、飞行、散布或粒子。仅静态读取和实现，未编译、运行客户端或测试。

## 小型对象的静态展示

新增箭 / 光灵箭精灵和三叉戟、潜影弹、羊驼唾沫、尖牙、拴绳结、水晶的模型接线。静态读取 1.21.1 ArrowRenderer、ThrownTridentRenderer、ShulkerBulletRenderer、LlamaSpitRenderer、EvokerFangsRenderer、LeashKnotRenderer、EndCrystalRenderer 的方向、图片与模型空间关系；只保留可辨认形态，不重建播放时钟。水晶 ShowBottom 控制底座，外壳 / 内核固定展示；尖牙固定展开。

旧版读取 cap / cac / bzh / bzy / bzb 的实际模型与图片，以及 bqr / bqd / bps / bqb / bro 的公开构造部件，五层加入 LegacyEntityReference 计划，累计 59 层。旧尖牙图片是 entity/illager/fangs，旧水晶为 entity/endercrystal/endercrystal；旧注册名分别为 evocation_fangs、ender_crystal。未运行生成器、编译、客户端或测试，新增实现待最终统一验证。

## 船与矿车静态展示

静态读取 1.21.1 BoatRenderer / MinecartRenderer、Boat / AbstractMinecart 及固有载物定义；1.12.2 byt / cad / afd / afe / bpj / bqe 的公开构造与保存字段。船采用实际模型及图片，现代普通船 / 箱船各九种木材，旧船六种；矿车复用主壳，载物进入既有方块资源后端。只保留静态保存朝向与载物高度，不模拟轨道推进、桨或受击动画。

旧矿车使用 DisplayTile（String 或 Int）/ DisplayData，现代使用 DisplayState；共同开关 CustomDisplayTile 与高度 DisplayOffset。默认载物来自原生矿车类型，箱子偏移 8、漏斗 1、其余 6。旧命令矿车注册名是 commandblock_minecart，现代为 command_block_minecart。旧船 / 矿车新增两个公开部件提取计划至 54 层，未执行生成器；实现待最终统一编译测试。

## 六类生物的静态展示

新增接线复用已提取的 1.20.1 / 1.21.1 模型目录与客户端图片：camel、sniffer、axolotl、tadpole，以及 1.21.1 的 armadillo、breeze / breeze_wind。静态读取 CamelModel、SnifferModel、ArmadilloModel、AxolotlModel 与对应 renderer：保留幼年模型空间变换、骆驼鞍具部件、嗅探兽固定幼年头部变换、犰狳展开 / 壳部件、美西螈五种 Variant 到真实图片的绑定、旋风人主体 / 风体 / 眼睛的独立图片。犰狳图片实际为 `textures/entity/armadillo.png`。

`extract_entity_animations.py` 仅用 javap 读取常量构造数据流，输出两版 27 / 37 个动画定义及来源摘要，不加载或执行客户端。共享 Rust 读取器供骆驼固定 sit_pose 与嗅探兽固定 baby_transform 使用，不以完整动画复刻为后续门槛；没有推进游戏时钟。新增实现未编译或测试。

## 监守者部件选择与原图先裁切

静态读取准确本机 1.20.1 fdr / frv / fts / bxs / fkf，以及 1.21.1 fxw / gnh / gph / cmb / gfh。模型每次 resetPose，再应用观察、行走、idle、触须和六个 AnimationState；保存加载没有启动这些 AnimationState，心跳 / 触须计数初始零。零时刻 idle 头 Z+=cos(0)*.06，身体 X+=cos(0)*.025；无步态时保留实际手臂 (-13/-13/1、13/-13/1)、腿和部件层级。保存 anger 更新愤怒值 / 心跳间隔，并不保存已经播放的心跳插值。

五层顺序为 bioluminescent、pulsating_spots_1、pulsating_spots_2、基础图片的触须选择、heart。生物发光只选头 / 双臂 / 双腿；脉动另加身体；触须只选两根触须，心脏只选身体。选择只改变各部件 skipDraw，不隐藏父链或把子部件自动加入。Alpha 分别为 1、max(0,cos(age*.045)*.25)、max(0,cos(age*.045+π)*.25)、getTendrilAnimation、getHeartAnimation；1.21 将 Alpha 经 floor(alpha*255) 打包，零时刻首脉动为 63/255，1.20 为 .25。透明度零且 COLOR_WRITE 的层没有贡献；身体隐形时整个附加层跳过。

两版 entity_translucent_emissive 均为 SRC_ALPHA 混合、NO_CULL、COLOR_WRITE，shader 只在原图 Alpha<.1 时丢弃，再乘 vertexColor / ColorModulator；没有场景光照贴图。分数透明度以 (-1,0) 的内部负 Alpha 保留其先裁切语义，采样恢复正值；这不是源图片、NBT 或公开资源材质中的负透明度，不与守卫者 Alpha=-1 参数混用，不占用既有材质位或量化原图。GPU / f32 栅格 / double 重心拾取使用同一片元合同，普通 Alpha Test 和凋灵加色规则分别保留。新增实现未编译或测试。

## 凋灵三头、无敌计时与加色护甲

静态读取准确本机 1.12.2 brj / cbj / ccp / abx，1.20.1 fdu / frx / ftu / fsr / buv，以及 1.21.1 fya / gnk / gpj / gog / ciu。保存 Invul 默认为零；正数时且 (Invul>80 或 (Invul/5)%2!=1) 使用 wither_invulnerable，否则 wither。零 partialTick 的外层比例为 2-Invul/220f*.5（正数计时），普通为 2，不自加计时限制。零参考 ribcage X=(.065+.05*cos(0))*π，tail pivot=(-2,6.9+cos(ribX)*10,-.5+sin(ribX)*10)，tail X=(.265+.1*cos(0))*π。中央头使用观察俯仰；侧头数组未保存、初始零，prepare 阶段减真实 yBodyRot。Entity.load 明确从保存 Rotation 设置 yBodyRot，不应用中央头角度或冻结偏航。

isPowered 为实际 Health<=MaxHealth/2，与 Invul 无关；默认 MaxHealth=300，保存属性和 NumericTag Health 按实际载入规则解释。独立 armor 模型膨胀 .5；xOffset=cos(age*.02)*3，yOffset=age*.01，零时刻按原生 repeat 取零。现代 energy_swirl 使用 ONE/ONE、NO_CULL、默认深度写入，其 fragment 在 Alpha<.1 时丢弃；shader 仅采样原图、不读取光照贴图。1.20 RGB=.5，1.21 packed 色 -8355712 对应 RGB=128/255；旧 ccp RGB=.5，沿完整 packed 光照，仅身体非隐形时写深度。三版层均不因身体隐形跳过。旧 abx.av 返回 15728880，现代 renderer 只返回 block=15。

旧 brj 的公开 ModelBase 注册列表依次为 shoulders / ribcage / tail / center_head / right_head / left_head；生成器补充主模型与 .5 护甲层，完整计划现为 52 层，未执行。仅静态读取，没有客户端执行、编译或测试。

## 眼睛位移参数与材质 Alpha 位

源材质的十进制 32 已用于原生 Alpha .1，GPU 内部还占用重复纹理 / 边界扇位。眼睛位移参数因此以四角 Alpha=-1 的明确几何标识保存，四组 RGB 的定义不变；原生颜色只允许 0..1，采样阶段将该标识恢复成白色 / Alpha=1，不修改源材质位。完整面、反绕序、平移、包围盒、拾取及 GPU 共用这一判别，不额外扩大缓冲格式。普通 cutout、文字及加色眼睛沿原有材质规则；修改未编译或测试。

## 守卫者尖刺 / 尾部与观察者眼睛

静态读取准确本机 1.12.2 bpu / bzp / bza / ada / act / vg，1.20.1 fbm / fpm / foq / bvy / bvr / bfj，以及 1.21.1 fvr / gkx / gkc / cjz / cjs / bsr。三版十二根尖刺使用相同实际六张坐标 / 旋转表；零参考位置因子 1+cos(i)*.01-(1-SpikesAnimation)*.55，载入插值为零。头应用保存俯仰，眼 pivot Z=-8.25；三节尾的真实父链、后两节 pivot (-1.5,.5,14) / (.5,.5,6) 保留。尾初始相位来自未保存 RNG，只能显式选择编辑器参考相位；运行时攻击目标 ID 未保存，不生成虚构光束。

无活动攻击目标时原生模型使用当前相机实体：观察者眼位高于源实体眼位时眼 Y=0，否则 Y=1；将源眼位减观察者眼位的水平向量 normalize / yRot(π/2)，与实体视向的水平投影点乘 d，眼 X=sqrt((float)abs(d))*2*(float)sign(d)。视向公式分别保留旧 vg.f 的 -yaw*DEG-π 与现代 calculateViewVector 的 -yaw*DEG。GPU 完整面在原位置 / UV 之外复用四角 RGB 存源眼位、模型局部 X / Y 每像素世界向量和视向，实际采样色仍为白；CPU 投影 / 拾取使用相同定义。新字段不改变 WorldQuad 步幅、不量化 UV，也不从包围盒估算眼位。

旧 bpu 公开 ModelBase 注册列表顺序为 head、spike0..11、eye、tail0..2；完整旧模型生成器新增两层，计划共 50 层，不访问私有字段或反射，尚未运行。仅静态读取，没有客户端执行、编译或测试；新增路径待最终统一验证。

## 猪灵三类的静态身体 / 装备姿态

静态读取准确本机 1.20.1 fcj / fqn / bxd / bxe / bxh / bxf，以及 1.21.1 fwo / glz / clm / cln / clq / clo。零参考耳朵左 Z=-.5235988-cos(0)*.08、右 Z=.5235988+cos(0)*.08；普通手臂右 Z=.1 / 左 Z=-.1，僵尸猪灵额外调用非攻击 animateZombieArms。僵尸猪灵 renderer 隐藏右耳，三类 PlayerModel 的专用 /ear 与 /cloak 不作为身体提交；最终袖、裤、外套复制对应部件姿态。三类 CustomHeadLayer 外层缩放均为 (1.0019531,1,1.0019531)，位于幼年和头部变换外，护甲不应用该缩放。

getArmPose 的跳舞标志先判断，但没有保存且初始 false；实际副手物品属于 piglin_loved 时进入观察姿态，头 X=.5 / Y=0。LeftHanded 为真时右臂 X=-.9 / Y=-.5，否则左臂 X=-.9 / Y=.5，原有 Z 微摆保留。攻击、充弩相关同步标志未保存且初始 false，不从武器 / Brain 预测后续 AI。extract_item_tags.py 只读取两版客户端归档内全部物品标签 JSON（99 / 147 项），保留客户端、各源条目、提取器及输出摘要；运行了静态提取，没有客户端执行、编译或测试。世界实际标签层复用 registry_tags，覆盖过滤 / replace / 嵌套 / 可选引用与循环错误解释，未知定义正常报告。新增实现待最终统一验证。

## 下界兽类与实际维度转化条件

静态读取准确本机 1.20.1 fbp / fpn / fsb / bwy / bwu / fdi / frh / bwp，以及 1.21.1 fvu / gky / gno / clh / ckv / fxn / gmt / ckq。两种疣猪兽模型共用实际年龄合同：幼年头 S(1.5/1.9)T(0,8/16,6/16)，身体 S(.5)T(0,24/16,0)，鬃毛 Z 幼年 -3 / 成年 -7。攻击动画计数未保存，初始零对应头 X=.87266463 / Y=2，观察俯仰不控制头 X。疣猪兽 isConverting 为 !dimensionType.piglinSafe && !IsImmuneToZombification && !NoAI；TimeInOverworld 不是 renderer 的转化判断。复用准确客户端维度 JSON 及真实数据包 / 内联 NBT 解释 piglin_safe。

炽足兽只经 ItemBasedSteering 保存 Saddle，另继承 Age；寒冷同步标志初始 false 且没有保存字段。模型零步态将身体 Y 从构造 1 改为 2，腿 Y=8；无乘客才应用观察俯仰，六根鬃毛零时刻均为实际构造角。旧 renderer 幼年整身 .5，新 renderer 使用 getAgeScale（同为幼年 .5）；不是 AgeableListModel 的头身分组。SaddleLayer 只检查鞍，不检查身体隐形，沿同一身体 / 姿态和 cutout 层提交。仅静态读取，新增实现待最终统一验证。

## 熊猫基因与劫掠兽颈部 / 下颌

静态读取准确本机客户端 1.20.1 fce / fqj / bsa / bsa$a、fcs / fqs / bwh，以及 1.21.1 fwj / glv / cft / cft$a、fwx / gme / cki。熊猫只新增保存 MainGene / HiddenGene：normal、lazy、worried、playful、brown、weak、aggressive；brown / weak 为隐性，必须双基因相同才表现。图片来自 renderer 的真实枚举映射。头身幼年分组参数为 (true,23,4.8,2.7,3,49)，坐姿、进食、喷嚏、仰躺和翻滚状态未保存，构造 / 同步初始均为零；gor 持物层先要求坐姿，因此不能仅按 HandItems 绘制。

劫掠兽实际保存三个 Int：AttackTick、StunTick、RoarTick。攻击使用 triangleWave(t,10)，g=(1+wave)/2，颈 Z=-6.5+12g³；下颌 t>5 时 sin((t-4)/4)π*.4，否则 .15707964*sin(πt/10)。无攻击时颈回 (0,-7,5.5)，眩晕颈 X=.21991149 / 横摆 (float)Math.sin((stun/40d)*10d)*3，下颌 π*.05；无眩晕且咆哮时下颌 (π/2)*sin(((20-roar)/20f)*π*.25)，普通为 π*.01。原生准备阶段先使用颈部前帧旋转计算 Y；静态参考采用真实构造零角，不共享历史可变模型。仅静态读取，未执行客户端或生成器，新增实现待最终统一验证。

## 狐狸嘴部独立姿态与山羊角

1.20 fbh / fpg / fst / brv 与 1.21 fvm / gkr / goi / cfo：实际 Type 为 red / snow，图片分别 fox / snow_fox 及各自 _sleep。模型准备依次判断 Crouching、Sleeping、Sitting；setupAnim 对 Sleeping 和 Crouching 独立处理。睡眠身体 Z=-π/2、头 pivot=(1,19.49,-3)、头 Y=-2.0943952 / Z=cos(age*.027)/22，四腿隐藏；幼年身体和尾另有位置 / 角度。蹲伏深度插值未保存，初始为零，零时刻身体 Y 转角=.01、后腿 Z=.01、前腿 Z=.005。头身幼年分组为 .75 / .5，头 T(0,8/16,3.35/16)、身体 T(0,1.5,0)。

FoxHeldItemLayer 不调用整个头部 pose：先应用幼年 S(.75) / T(0,.5,.209375)，只平移模型头 pivot，再使用独立 HeadRoll / 相对头偏航 / 观察俯仰。前两者无保存初始偏移，采用零。幼年睡眠 / 清醒 T(.4,.26,.15) / T(.06,.26,-.5)，成年睡眠 / 清醒 T(.46,.26,.22) / T(.06,.27,-.5)，随后 RX(90°)，睡眠再 RZ(90°)。固定主手、ground、handedness=false；不能继承睡眠模型头 Y / Z 旋转，也不能复用海豚按主臂选槽。

1.20 fbl / fpl / btg 与 1.21 fvq / gkw / che：HasLeftHorn / HasRightHorn 加载时直接 getBoolean，字段缺省为 false，模型无额外年龄判定；真实保存值控制角。幼年头因子 1.5/2.5=.6、T(0,19/16,1/16)，身体 .5 / T(0,1.5,0)。Goat body 保留准确构造旋转 0，未保存的 RammingXHeadRot 初始零，不从 Brain 冷却推算撞击。新增实现未编译或测试，待最终统一验证。

## 蜜蜂与幻翼的保存状态 / 静态参考

1.20 fal / fob / brm 与 1.21 fuo / gjl / cff：四种实际图为 bee、bee_nectar、bee_angry、bee_angry_nectar，愤怒沿保存剩余 AngerTime>0，HasStung 隐藏 stinger。地面静止为 OnGround&&Motion.lengthSqr()<1e-7，使用全向而非海豚的水平速度；初始 RollAmount=0。正常飞行在零时刻保留原生翼幅 π×.15、触须 π×.03、身体 X=.1+π×.025 / Y=19-.9 及各腿分支；愤怒不套正常悬停分支。AgeableListModel(false,24,0) 的头列表为空，幼年整身 .5 并 T(0,1.5,0)。

1.20 fcg / fql / fte / bwe 与 1.21 fwl / glx / got / ckf：幻翼原生 Size 限制 0..64，renderer 为 S(1+.15×Size)、T(0,1.3125,.1875)，父旋转后再 RX(保存俯仰)，随后沿普通生物 -1.501 平移；generic.scale 位于该组变换之前。模型 body / head 构造角保留，零相位左右翼 Z=±16°、尾两节 X=-10°，身体 / 眼睛使用同一姿态和矩阵。

getUniqueFlapTickOffset 来自未保存的运行时数字 ID，不能由 UUID 或存档路径恢复；编辑器显式选择零相位静态参考，不伪造运行时 ID。原生加色眼睛复用现有现代 EyesLayer 材质、光照和隐形语义，独立矩阵包装复用同一后处理。新增实现未编译或测试，仍待最终统一验证。

## 海龟与海豚的模型 / 原生单物品层

1.20 fdn / fba / fsn / bsm / brt 与 1.21 fxs / fvf / goc / cgf / cfm。两版海龟模型的 AgeableListModel 参数为 scaleHead=true、头平移 120/0、头比例除数 9、身体比例除数 6、身体平移 120；因此头缩放 1.5/9、身体 1/6、两者 T(0,120/16,0)。成年 egg_belly.visible=HasEgg，绘制前对全体部件 T(0,-.08,0)。零步态幅度下陆地 / 水中 / 挖卵四肢结果一致，不需要推测未保存的挖卵时序。

海豚模型使用实际 Motion.horizontalDistanceSqr()>1e-7；零时刻运动分支身体额外 X=-.1、尾 X=-.1、尾鳍 X=-.2，静止保留构造尾 X=-.10471976。两版 carrying layer 在主臂 RIGHT 时选主手，否则选副手，ground 显示且 handedness=false；俯仰因子 f=abs(pitch)/60，负俯仰 T(0,1-.5f,-1+.5f)，其他 T(0,1+.8f,-1+.2f)，不挂接身体部件或追加 RX(180)。现有单物品准备路径沿这些实际规则接线。

vg.f / bfj.g / bsr.g 均将已保存 Motion 各分量绝对值大于 10 的值置零，缺字段读取初始零向量；这是准确客户端的既有载入语义，读取结果不回写源 NBT。公共读取器要求实际三项 Double 和有限值，普通源错误继续报告。所有新增实现仍待最终统一验证，未编译或运行客户端。

## 四类鱼 renderer 与热带鱼组合定义

1.20 foj / fqu / fqq / fro / ftq 与 1.21 gjv / gmg / gmc / gna / gpf 的实际入口：前三类需要水状态的鱼沿父旋转后处理离水翻转，鳕鱼 T(.1,.1,-.1)，鲑鱼 / 热带鱼 T(.2,.1,0)，再 RZ(90°)；鲑鱼在该分支之前始终 T(0,0,-.4)。河豚为父旋转前 T(0,cos(t*.05)*.08,0)，不查询水状态。模型采用准确现代目录及两版已核实共享层，不执行客户端或参照生成器。

bsl$b / cge$b 实际注册 SMALL 的 KOB / SUNSTREAK / SNOOPER / DASHER / BRINELY / SPOTTY 与 LARGE 的 FLOPPER / STRIPEY / GLITTER / BLOCKFISH / BETTY / CLAYFISH，每类内部 pattern=0..5；packedId=shape|(pattern<<8)，底色在 bit16..23，花纹色在 bit24..31。真实图为 tropical_a / tropical_b 及各自 pattern_1..6。cen / cti 的全部 16 项 texture diffuse RGB 与已有共享染料表相同；1.20 转三路 float，1.21 加不透明 Alpha，双层分别使用实际颜色。

河豚 fwr / fws / fwt 左鳍在零时刻 Z=.2、右鳍 Z=-.2，大 / 中模型路径为 left_blue_fin / right_blue_fin，小模型为 left_fin / right_fin；鳕鱼 tail_fin、鲑鱼 body_back 和热带鱼 tail 的零时刻 Y 为零，保留其余实际构造旋转。两版 rendertype_entity_cutout_no_cull.fsh 明确 color.a<.1 时 discard；旧 caa 为 GL_GREATER(516,.1)。共享原生渲染包装据此显式声明 .1，新增代码仍待最终统一验证。

## 鱼类实际水接触与流体标签

1.20 bfj.a(TagKey,double) 与 1.21 bsr.a(TagKey,double) 都将实际 AABB 内缩 .001，floor(min) / ceil(max) 遍历，先以 float 做 Y+FluidState.height，再提升 double 与包围盒下界作 >= 比较。读取流体接触不推进推力或实体运动。FlowingFluid 的源 / 下落 amount=8，其余按 level 还原 amount，再除以 9；同族上方流体使高度为 1，不能以标签成员替代同族判断。

extract_fluid_tags.py 只静态读取准确客户端内全部 tags/fluids（3465）或 tags/fluid（3955）JSON，输出两版目录及各自 proof，包含客户端、每个归档条目、提取脚本和输出 SHA256。实际水 / 流动水与岩浆 / 流动岩浆标签从原文件提取，不用名称猜测数据包成员；运行时复用既有注册标签解释器。四类鱼的 renderer / 模型静态入口已定位，渲染仍在迁移，尚未编译或测试。

## 鹦鹉与玩家肩部模型

准确旧 caj / bqh / aac / cca、1.20 fqk / fcf / bsb / ftd、1.21 glw / fwk / cfu / gos：五种真实图片均为 entity/parrot/parrot_red_blue、parrot_blue、parrot_green、parrot_yellow_blue、parrot_grey。Sitting 先于 isFlying（!OnGround），原生 isBaby 恒为 false；准备头羽 X=-.2214、身体 X=.4937、双翼 X=-.6981 / Y=-π，坐姿和离地腿角沿实际单精度加法。跳舞标记、flap 与 flapSpeed 不持久化，采用零时刻目标，不构造时序。

现代 renderOnShoulder 显式 ON_SHOULDER；旧 cca 给新 bqh 传玩家对象，保留初始站立状态，零运动结果一致。肩部层使用父玩家镜像 / 缩放 / -1.501 平移后再 T(±.4,-1.5,0)，不经过头 / 手臂部件。原生蹲伏时的 -1.3 只由同步蹲伏状态驱动，基础实体 / 生物 / 玩家存档均不持久化该状态，不由其他字段冒认。载荷 Variant 决定皮肤，载荷自身 Sitting、Age、Rotation 不参与肩部姿态。

cct / fty / gpo 明确缩放 .9375；旁观属性隐藏全部模型，仅显示 head / hat，LivingEntityRenderer 的附加层循环跳过旁观玩家，身体隐形不跳过肩部层。旧模型新增 bqh 公开注册的 11 部件，完整生成计划为 48 层，尚未执行；新增实现待最终统一验证。

## 蝙蝠与鱿鱼的原生全局变换

准确 1.12.2 byr / bpg / zt 与 cax / brb / aaj；1.20.1 foa / fak / brg 与 frf / fdh / bsk / fpk / bfr；1.21.1 gjk / fun / cez / fhh / fhf 与 gmr / fxm / cgd / gkv / bte。仅通过 javap 静态读取，不执行客户端。现代模型沿实际目录，旧模型公开注册顺序新增 bpg 八部件及 brb 九部件，完整生成计划为 47 层，尚未运行。

BatFlags 按 bit 0，加载缺省零；旧 / 1.20 renderer 先平移 -0.1（倒挂）或 cos(0)*.1（飞行），再走普通父旋转、镜像及 .35 缩放。1.21 无这组缩放 / 升降，BatAnimation 零时刻倒挂头身 X=180°、Y 偏移 -.5、两翼 Z 偏移 1、翼 / 翼尖 Y 为 ±10° / ±120°；飞行身体 X=40°、脚 X=10°、翼 / 翼尖 Y 为 ±85° / ±10.5°。position 通道沿 fhf 对 Y 取反；新版头部只在倒挂响应相对偏航，不套保存俯仰。

SquidRenderer 的 setupRotations 不调用父方法：T(0,.5,0)、RY(180-yaw)、RX(xBodyRot)、RY(zBodyRot)、T(0,-1.2,0)，之后才镜像及普通 -1.501 平移。两个 bodyRot 及 tentacleAngle 均无 NBT 编码，使用构造零值，保留目录中的触手放射 Y 方向。1.21 glk 在该组矩阵之前应用实际 generic.scale。新增实现均待最终统一验证。

## 猫变种注册与版本化猫 / 豹猫模型

`tools/extract_cat_variants.py` 只读取准确客户端及官方映射，通过 javap 的实际注册键构造、bootstrap 纹理调用和 Cat.defineSynchedData 的缺省入口提取目录。1.20.1 brp / bro 与 1.21.1 cfi / cfh 都有 11 个原生键，缺省均为 black。`cat-variants-3465.json` / `cat-variants-3955.json` 及相邻 proof 保留来源和输出摘要；没有执行类、编译参照或测试，不把注册名当作纹理名。

旧 cah / bqg / aab 与现代 fof / faq / fcd、gjr / fuv / fwi 区分旧 CatType 及独立现代实体。三版幼年头 S(.75) / T(0,10/16,4/16)，身体 S(.5) / T(0,1.5,0)；坐姿前腿旧 Y=15.8、现代 Y=16.1。旧版 .8 缩放取真实 isTamed，现代 Cat 固定 .8、Ocelot 不套该缩放；现代 cat#collar 使用真实独立模型和 cat_collar 图片，沿保存染色与父模型姿态。旧导出新增一层，完整计划为 45 层，尚未执行；新增实现待最终统一验证。

## 傀儡身体、损伤层与固有南瓜物品

静态读取旧 brf / bqz、cbf / cau / ccj、aak / aai 与 vi 注册；旧实体实际为 villager_golem / snowman，图片为 entity/iron_golem 与 entity/snowman。现代模型 fvz / fxk（1.20.1 fbu / fdf）及渲染 / 层 gle / gmo / gom / gpb 使用独立现代图片。铁傀儡最大生命默认 100，原生属性范围 1..1024；现代裂纹在实际生命比例严格小于 .75 / .5 / .25 时选择 low / medium / high，旧版没有该层。

三版南瓜层均按 Pumpkin 状态创建实际物品并使用 HEAD：head 部件、T(0,-.34375,0)、Y(180)、S(.625,-.625,-.625)。旧使用 pumpkin，现代使用 carved_pumpkin；不以固定方块立方体替代资源模型。隐形普通层不提交，现代原生发光描边分支留待全局描边通道。旧导出新增两层，完整计划为 44 层，尚未生成；没有编译、测试、执行客户端 / 参照或开窗。

## 鸡、兔子与北极熊的实际头部分组

静态读取旧 `bpm / bqn / bqk` 及 `byv / can / cam`、1.20.1 `fau / fcq / fcl` 与 1.21.1 `fuz / fwv / fwq`。鸡的 head / beak / red_thing 是三个独立根，幼年头部 T(0,5/16,2/16)，身体 S(.5) / T(0,1.5,0)。兔子头部包括 head / nose / 两耳，成年所有部件 S(.6) / T(0,1,0)，幼年头 S(.56666666) / T(0,1.375,.125)，身体 S(.4) / T(0,2.25,0)。北极熊渲染器外层 1.2，幼年头 S(.6666667) / T(0,1,.25)，身体 S(.5) / T(0,1.5,0)。不改造原始部件父链来伪造头部分组。

兔子真实数字变种 0..5 / 99 对应 brown、white、black、white_splotched、gold、salt、caerbannog；三版资源均包含 toast。原生比较实际名称展开结果并去除 §[0-9A-FK-OR]（忽略大小写），未知码保留，Toast 大小写精确匹配。普通 / 杀手兔缺少自定义名称时使用实际版本的默认翻译键；组件与资源语言复用既有解释，不执行文本事件。跳跃、展翼与直立时序未保存，不从其他字段猜测。

旧准确导出定义新增三层，完整计划为 42 层。这里只有静态读取、源码实现和格式整理，尚未生成参照、编译、测试或运行客户端；新增模型与公共分组 / 文本接口待最终统一验证。

## 恼鬼与悦灵的浮空模型和挂接

静态读取准确旧 `bre / cbe / adp / bzr`、1.20.1 `fdo / frr / bwq` 与 1.21.1 `fxt / gnd / ckr`。旧恼鬼采用 .4 倍双足模型，构造器隐藏 hat / left_leg、替换 right_leg 并新增两翼；零时刻沿继承的手臂摆动和原生弯腿 / 翼角，Charging 同步位不保存。现代采用独立 root 层级，持有层依次提交 root / body / arm、S(.55) 和左右 .046875、-.15625、.078125 偏移；不能用旧双足挂接近似。

悦灵对应 1.20.1 `fae / fnw / bsp`、1.21.1 `fug / gjf / cgk`，实际模型层共用已有准确几何。初始根部 Y 增加 .25，翅膀 / 手臂读取 setupAnim 的零时刻值；持有层仅提交 root / body、固定平移、rightArm.xRot、S(.7) 和 X(.0625)，不提交手臂外展的完整旋转。两版持物插值字段初始为零，读存档不恢复它，跳舞 / 旋转也不保存；CanDuplicate、listener 和实际手持物不被冒作动画进度。

两种现代模型均直接引用 entityTranslucent；实际片元着色器的 Alpha 阈值为 .1。旧恼鬼的头部 / 鞘翅 / 双手层来自 bzr，现代两种渲染器均只有持有层；几何、装备和材质按版本分别接线。旧模型完整计划现为 39 层，尚未生成，没有编译、测试、运行参照或开窗，新增实现待最终统一验证。

## 灾厄村民与女巫的版本化持有层

本次仅静态读取准确客户端：旧 `bpy / bzi / cbh / bzt`，1.20.1 `fbt / fpr / fpb / frt / fps / fqo` 与 1.21.1 `fvy / glc / ckg / ckn`。三版灾厄身体缩放 .9375；旧幻术师图片名为 illusionist，现代为 illusioner。SpellTicks 是保存的 Int；客户端同步法术 ID、攻击 / 庆祝 / 装填和幻象时序不在保存载荷中。模型的交叉 / 展开手臂、帽子及持弩条件与装备层分别解释，现代头部层不倒灌旧版，也不自动添加护甲层。

女巫对应旧 `bri / cbi / cco / adr`，1.20.1 `fdt / frw / ftt / bws`，1.21.1 `fxz / gnj / gpi`。原生 holding 条件是主手非空；零时刻鼻部 roll=2.5×.017453292，持物 x=-.9。旧 y / z 是绘制 offset，postRender 只用本部件 pivot / rotation，不带父头部或 offset；现代药水层明确先提交 head、nose 的矩阵。旧持有层实际读取默认方块 ENTITYBLOCK_ANIMATED、弓、Item.D_ / F_，末尾 X(-15)、Z(40) 并使用 THIRD_PERSON_RIGHT_HAND；现代普通药水有独立矩阵，然后共用 GROUND 交叉手臂层。未知物品定义报错，不按资源模型是否立体猜测原生属性。

旧模型完整导出计划现为 38 层；旧物品生成工具增加上述持有属性。当前没有重新生成模型 / 物品目录、编译、执行客户端或测试，全部新增实现待完整迁移后的统一验证。

## 整图与精灵的独立材质合同

同一 PNG 可以同时供 SimpleTexture 与 atlas 使用。共用一次解码，字体与整图保留原始尺寸，atlas 独立解释 animation；无效 atlas 声明继续报告错误，不影响本来不读取该节点的整图合同。裁切精灵的完整源采用借用方式参与图集排布，普通静态图共享物理槽；内部采样身份不能被资源包注册名占用，也不在界面添加新入口。独立完整矩形保留原始比例和坐标，不接收精灵时钟写入。

原生模型层 material.texture_source 默认 simple，可显式指定 atlas；未知值正常报错。原版元素配方通过 mcedit2:simple_cutout 声明来源，沿用普通 JSON 父链和现有材质 / 重复 UV 后端。世界模型、特殊物品图标、装备、经验球、旧挂画与旧地图使用相同绑定；现代精灵和普通 generated 物品保留 atlas 动画。旗帜 / 盾牌整图底材与实际图案图集分开读取。运行时材质提供同样的绑定身份，组合动画保持它自身已经计算的时序。

源 Alpha 分类按合同分别缓存，资源身份包含整图和完整动画源；旧箱子比例检查核对原始 PNG。资源预算计入物理槽、mip 和新增绑定。普通图片直接移动原解码缓冲，额外整图排布借用既有完整源。准确客户端 SimpleTexture 的静态读取确认 PNG 与动画节点分开，texture.blur / clamp 的独立采样仍待迁移。新增实现未编译或测试，原生像素、资源切换、动画旧帧和各视图对照留到最终统一验证。

## 动物身体与装备

狼、现代马类和羊驼使用实际模型目录与各层原生变换，身体和装备不分开猜坐标。静态姿态解释实际保存状态；旧狼后腿坐姿高度、染料 / UUID 格式与现代版本分开适配。狼的静态 Cuboid / UV 复用已核实定义，旧参照工具补有原生导出，尚未执行。马的幼年头部 / 长腿、羊驼三组非均匀缩放、鞍具部件 / 缰绳、驴骡比例、马铠 / 染色、羊驼装饰以及狼铠覆盖 / 裂纹来自准确客户端字节码。

狼外观注册表来自准确 1.21.1 客户端的 wolf_variant JSON，保留源摘要，接入世界数据包层和缓存。资源加载前传入世界来源，将实际注册定义的图片加入依赖，允许自定义 namespace 与纹理目录；普通资源包 data/ 不冒充世界注册表。未知外观或缺失图片正常报告错误。三版属性格式、真实修饰身份及运算统一解码，使用方采用原生已知属性范围。

没有编译、运行新参照或执行测试；旧马类 / 羊驼、其他动物、未持久化的原生动作、原生像素和最终窗口对照仍待完成。整图与 atlas 分流已接入，见前节，尚未验收。

## 头部物品与鞘翅

三版 CustomHeadLayer 的通用物品变换、头颅专用变换、村民偏移与幼年外层使用实际字节码；头部不按物品逐件写模型。普通物品沿真实 head 资源上下文，头颅沿现有 SkullBlockRenderer、真实资料与模型目录。头盔由实际护甲槽判定，与头部物品层分离。

三版 ElytraModel 的原生 Cuboid / UV / 镜像一致，共享已核实的静态模型层；旧版幼年 Z 偏移、保存 Motion 的飞行角度与实际实体整体缩放由适配层解释。默认整张 PNG 使用 SimpleTexture 语义，Mojang 资料中的 ELYTRA / CAPE 保留原图尺寸与纹理哈希缓存。新缓存记录装饰缺省，旧的仅皮肤缓存不用于推断装饰；装饰失败独立报告，正常皮肤继续显示。未保存的披风开关使用编辑器可见策略，完整原生动作与图像等价仍待迁移及最终验证。

没有编译或运行新的参照；旧模型参照工具已补鞘翅导出，旧夹具留到最终统一生成与核对。

## 护甲纹饰的组合动画

材质组合区分 SimpleTexture 与 atlas：护甲底图按整张 PNG 解释，纹饰使用实际 Sprite 与 Sequence。输入先按各自保存的动画定义取帧及执行原生 RGB 插值 / 当前帧 Alpha，再做染色、cutout 与 decal；不展开共同周期，不把首帧或合成后插值冒充整个动画。组合身份包含全部源像素与时序，弱引用缓存不保活离场定义。

RuntimeTexture 携带可选组合源，运行时图集追加与场景页协调走同一入口；原素材绑定改为共享不可变身份，组合样本使用完整输入帧状态。CPU 的冻结图集、GPU 小页更新、源拾取和预览复用同一帧；前缀证明保留父时钟与旧绑定，旧快照不被新一帧改写。工作集按实际输入与动态页预算约束，定义冲突正常报告错误。

新增路径未编译或运行验证；原生合成像素、不同分辨率 / 过滤 / mip、跨视图同步、旧帧和预览交接仍须在完整迁移后统一验收。

## 护甲与纹饰

现代 HumanoidArmorLayer（gol）按实际 ArmorItem 槽位选择内 / 外层，复制身体部件变换并设置可见部件；材料层使用 ArmorMaterials（csh）的 leather / chainmail / iron / gold / diamond / turtle / netherite 与皮革 overlay。静态实现复用准确模型目录；旧版实际 ModelZombie 的 64 高默认贴图坐标按 ModelBiped 的 32 高护甲调整，膨胀量为 .5 / 1 像素。HumanoidModel 将帽层姿态复制自头部；现代护甲使用 VIEW_OFFSET_Z_LAYERING。

准确 1.20.1 / 1.21.1 客户端的 trim_pattern / trim_material 与 1.21.1 item tags 合并保存在版本化资源中，并记录客户端摘要。世界定义走现有注册资源过滤、目录 / ZIP 和 overlay；真实 Holder、内联定义、材料覆盖与资源 namespace 决定最终纹饰 sprite。资源索引接入护甲 PNG 和实际 paletted_permutations 输出，不造图案对照表。

静态护甲各材质共用几何及 UV，按 cutout 的实际覆盖顺序合成为运行时材质；decal 限定在前层已经覆盖的护甲像素。不同分辨率使用共同采样网格，受现有资源预算约束。此实现未做原生像素、纹理过滤及旧版完整 renderer 等价验证；动画组合已接入共享图集，具体实现与未验证范围见上节。鞘翅与特殊头部已接入，动物装备继续迁移；没有编译或执行新的参照 / 测试，新增路径待最终统一验证。

## 第三人称手持物

只读三版本 ItemInHandLayer（1.12.2 ccc、1.20.1 fsz、1.21.1 goo）：幼年先平移 .75、缩放 .5，部件 translateToHand 后 X(-90)、Y(180)，物品平移为 (±1/16,.125,-.625)。PlayerModel 的细手臂另调 .5 像素，SkeletonModel 调 1 像素；三版本对应 bqj / bqw、fck / fda、fwp / fxf。身体与手持物的部件姿态、年龄及整体缩放使用同一 Rig，避免按可见网格范围猜手掌位置。

ItemTransform.apply 的左手处理只改变位移 X 与旋转 Y / Z，左显示定义缺省时按原生每级父链继承同级右定义；显式空对象保留其真实恒等变换。手持物沿实际模型覆盖及专用模型后端准备材质，使用同一世界图集与预览偏移。lefthanded 是持有者属性，不能以当前正在绘制左手代替；恢复指南针读取玩家保存的 LastDeathLocation。

没有编译、执行客户端、生成或运行新参照，所有新增路径待最终统一验证。当前是静态保存姿态，不能将其记为攻击、拉弓、使用、潜行、逐 tick 指针或玩家未保存惯用手设置的动态等价实现；护甲 / 装饰与更多实体挂接尚未接入。

## 装备来源与部件挂接

只读准确客户端的保存字节码：1.12.2 的 vq / abz，1.20.1 的 bgb / bux / byo，1.21.1 的 btp / ciw / cmx；护甲与双手按 ArmorItems / HandItems 的固定列表下标保存，玩家使用 Inventory 与 SelectedItemSlot，现代 Mob 另保存 LeftHanded，1.21.1 增加 body_armor_item。实际装备解析与槽位编辑共用版本化完整物品解码，不生成额外 Slot 字段。

模型目录新增部件挂接点转换，直接复用烘焙的完整父链运算；不可见的手臂不意味着丢弃其坐标系。1.21.1 ItemInHandLayer 的 goo 字节码已经读取，第三人称上下文、左右手及年龄外层变换现已接线，验证状态见上节；没有生成、编译或执行新参照，不能将此进度视为装备模型 / 像素等价验收。

## 实体名称与资源字体

名称只读取实际源 CustomName / CustomNameVisible；1.12.2 为字符串，1.20.1 / 1.21.1 为 JSON 组件。复用告示牌的资源字体与文本组件排版，字体、翻译、颜色、粗体 / 斜体及装饰来自实际资源和组件；换行后各行居中，同一名称的所有平行四边形字形共用一个面片锚点。需要游戏执行上下文的组件或未实现的混淆字形正常报告错误，不用系统字体或源文字冒充。

静态编辑器采用常显 / 选中对象语义，以源几何或成功编译的实际模型范围确定标签位置，局部字形缩放 .025、背景 Alpha .25、距离 64 格。玩家只使用已有实际资料名称，不另开在线查询；预览只显示常显名称。此实现没有完成原生名称渲染器、实体物理高度、队伍 / 计分板、遮挡穿透或动态文字效果的等价对照，当前新增路径均待最后统一验证。

## 经验球与朝向相机的面片

只读分析准确客户端的经验球渲染器 `bzj` / `fpc` / `gkn` 与实体 `vm` / `bfp` / `btc`，没有编译或执行原生参照。三版 Value 均保存为 Short，精灵阈值为 3、7、17、37、73、149、307、617、1237、2477，采用实际 `textures/entity/experience_orb.png` 的 4×4 分格。局部矩形 (-.5,-.25)..(.5,.75) 缩放 .3，真实旋转锚点在 Pos 上方 .1；颜色固定为初始化相位的 (127,255,3,128)。不把保存 Age 冒作现代 tickCount 或旧版未保存的颜色计数，不声称脉动、光照、物理或原生像素等价。

`billboard` 统一面片源编码、相机基、局部旋转锚点和实际四角。WorldQuad 原步幅内保存前三角及锚点，GPU 普通 / 区域 / 常驻透明与通用顶点入口均解释该编码，CPU 投影、射线和显示轮廓同样解释；完整 UV / RGBA 没有挪作锚点。动态包围体覆盖全部相机方向，舍入向外；面片不会进入静态 AABB 紧凑编码、固定平面合并或固定轴朝向证明。透明面片按源锚点保留排序身份；各相机只改变显示位置，不修改源面或要求重新建模。新增路径待最终统一验证。

## 掉落物静态显示

实现读取本机准确 1.12.2 / 1.20.1 / 1.21.1 客户端的 `bzu` / `fpu` / `glf` 字节码，未执行渲染器或编译 / 运行测试。三版数量阈值为 1、16、32、48，最多五份模型；ground 缩放、平面层距 0.09375 和 0.15 / 0.075 散布幅度沿用原生绘制逻辑。资源场景单独保留模型的平面 / 立体属性和实际 display scale，不能把 gui_light=front 当作平面几何证明。

世界与预览共用 `item_geometry` 的 Item 解析、资源模型、运行时材质和四角转换。编辑器使用实际保存位置 / Rotation 的静态姿态，按物品 ID / damage 稳定散布；它与原生数值物品注册 ID 的随机种子、未保存的随机相位及逐 tick 漂浮 / 旋转不等价，不将此实现记为原生像素对照。现代掉落物指南针沿实际实体朝向解释世界目标；旧版没有持有者 / 展示框的指南针按原生返回 0。均待最后统一验证。

## 保存中的移动活塞

`tools/generate_piston_reference.py` 与 `PistonRenderAdapter.java` 以准确 1.12.2 / 1.20.1 / 1.21.1 客户端为来源，实际载入保存字段并调用原生渲染器。分析副本只截获构造、世界非空入口、缓存 / 图形状态及最终模型提交，保留原生分支、状态属性修改和现代 PoseStack。共 12,168 / 24,336 / 24,492 组记录覆盖六方向、伸缩、源活塞、石材 / 楼梯 / 活塞 / 活塞头状态及十三种进度。证明记录原始客户端、适配成员、生成源、适配器和压缩输出摘要；这不是游戏窗口、GPU、AO、光照或原生材质像素对照。

压缩数值夹具及原生资源 JSON 位于世界库 `tests/fixtures/piston-render-*.json.gz`、`piston-models-*.json`。原生渲染和资源选择使用的实际采样坐标与平移独立保留；旧客户端采用 BE 坐标及四分之一阈值，现代采用运动反向采样坐标和原生活塞头分支，回缩基座使用自身坐标。存档读取与刷怪碰撞共用 `piston::Source`，碰撞盒不参与资源模型构造。

产品通过原版空模型 / 准确旧注册表绑定接管，资源包显式几何及声明式 BE 替换继续使用其定义。被移动的状态沿用普通方块的 JSON / OBJ、种子选择、UV、材质通道及四角后端，独立托管网格保留相邻面，由现有深度通道处理遮挡。当前继续使用编辑器满亮；实际群系邻域着色已接入、待最后统一验证，更多真实世界 / 客户端画面对照仍待推进，不将模型提交参照称作完整游戏渲染复刻。

## 现代刷怪候选参照

`tools/generate_spawn_catalog.py` 复用准确客户端及依赖校验，通过 1.20.1 / 1.21.1 的公开 API 导出全部 1,003 / 1,060 个方块、24,135 / 26,684 种状态。生成物为世界库的 `assets/spawn-properties-*.json`、原生方块标签、四种维度类型及相邻证明文件；同时保存每版 20 组光照整数提供器的原生 Codec 接受性与边界参照。分析使用临时客户端副本和直接接口，不用反射，不启动游戏，不修改原始 JAR。

候选采用原生僵尸的地面支撑谓词和 `NaturalSpawner.isValidEmptySpawnBlock` 身体空间合同。导出时分别记录原生完整标签结果、清除 FIRE / PREVENT 标签后的基础结果，以及谓词是否实际读取世界；生产再合并实际启用数据包的标签，不能把原生标签固定为不可修改规则。仅声明动态形状而未读取世界的状态正常使用；实际读取 BE 的状态通过保存上下文解释，不能把空环境调用值当作完整实现。

`tools/generate_spawn_context.py` 直接导出两版全部状态的碰撞盒，并按原生类绑定静态、潜影盒、移动活塞与活塞基座合同，每版共用 320 种形状。移动活塞按实际 `blockState`、`facing`、`progress`、`extending`、`source` 构造形状，包含回缩源基座、长短活塞头、原生 Float 运算和 `1e-7` 体素比较。位置偏移状态逐个证明其 X 或 Z 碰撞宽度小于一格；在非源移动的单一形状下，任何位置偏移都不能使其成为完整顶面或完整格体。潜影盒开启动画不保存，按原生重新读取后的关闭盒体解释，不臆造存档中的动画进度。缺失 / 错误 BE、未知移动状态、无效字段及同一移动上下文递归引用正常报告。

参照同样由准确客户端实际读取移动 NBT 后调用原生谓词，每版 39,840 组，覆盖 324 组代表状态、六方向、伸缩、源活塞和十种 Float 进度（包括超出常规范围及接近 0 / 1 的值）。数据及证明在 `assets/spawn-context-*.json`；真实 Region 集中验证范围与结果见 PROGRESS.md。此目录只服务刷怪谓词，不把资源渲染模型当作碰撞，也不表示已播放实时方块实体动画。

光照读取要求准确版本、已完成区块和保存的 `isLightOn=1`，使用实际维度高度、天空光开关、方块光上限及整数提供器最大测试值。支持 constant、uniform、biased_to_bottom、clamped、clamped_normal、weighted_list，版本格式差异集中解释；不把提供器边界当作均匀概率。原生 clamped 的报告边界可能倒置，按真实接口保留。省略的齐次零光照层按原生保存规则解释，高度外的身体状态为 `void_air`，也参与实际标签判断。

图层表示保存光照下的空间候选，未模拟难度、玩家距离、生成上限、种群选择及完整随机生成过程。“需降低天空光”不保证维度会自然入夜。真实编码临时 Region 的逻辑对照不替代游戏重载或实际大世界验收。

## 本机内容寻址资产

`client_assets` 复用原有准确版本元数据定位规则，读取 Prism / 原版启动器所声明的 `assetIndex.id` 和本机 `assets/objects`。现代 JAR 使用内部实际版本 id，重命名不会误取另一份元数据；旧版使用现有客户端路径合同。只打开渲染或语言读取实际支持的对象，声音等未消费条目不会触发全量缓存读取，不下载文件或枚举其他版本索引。

JAR 与该索引属于同一个原版资源包，同路径优先使用索引对象，避免将空定义或坏副本重复并入字体；后续资源包保留原有覆盖 / 字体合并规则。原版定义中的任意命名空间、bitmap / unihex / reference 等沿用同一字体与图片入口，不按字符或字库名称硬编码。声明对象缺失、类型 / 大小不符及解析错误正常报告，来源包含索引与逻辑路径。哈希用于缓存路径定位，不将此读取描述为重新校验全部资产摘要。

同一二进制字库的完整 Unicode 索引随资源快照共享，取消和错误不缓存；替换资源不会改变旧快照字形。保留现有原生字形宽度、整数 advance、边界覆盖、样式和几何排版。共享解析缓存按 32 MiB 字节预算保留，不另设条目数上限，活跃工作集仍通过字体预算校验。

## 地图装饰参照

`tools/generate_map_reference.py` 调用准确 1.12.2 / 1.20.1 / 1.21.1 的地图公开 API，导出 10 / 27 / 35 项标记目录、真实旗帜 / 展示框 NBT 和五档缩放的正负边界投影；载荷与数值位于世界库 `tests/fixtures/map-reference-*.json`，相邻证明保存客户端、生成器和输出摘要。生产目录使用同一输出，不按每张贴图猜图标索引。

1.21.1 的 `MapCpuAdapter.java` 只在分析副本公开内部类、静态化绘制入口，并截获纹理上传、sprite UV、渲染类型及字体环境边界；另加一个直接调用原生私有投影方法的公开静态桥。原生投影、矩阵栈、地图与标记绘制循环、过滤和顶点调用继续执行：330 组投影覆盖离图玩家、无限追踪与缩放边界，1120 组顶点覆盖全部类型、十六档方向和展示框过滤。名称在这份顶点参照中禁用；名称载荷仍由原生旗帜 Codec 导出，字体使用已有资源字体排版，不能把此顶点参照宣称为原生字体像素对照。没有反射、Unsafe、GPU 或游戏窗口，也不修改原始客户端。

现代地图标记类型是客户端内置注册表，Codec 为 `holderByNameCodec`，不臆造数据包或内联 Holder 格式。sprite 来源按实际 `asset_id` 解析为各命名空间的 `map/decorations/`，旧版采用原图集索引；缺失资源与未知注册项正常报告。模组运行时注册表来源仍待扩展。

地图文字与 sprite 按原生 `RenderType.text` 使用无 mip 的纹理采样和 `COLOR_DEPTH_WRITE`，复用已有混合且写深度的阶段；重叠名称的远背景不能覆盖已画的近字形。底图、图标与名称保持原生逐层深度和完整源实体归属。

本批依据准确 Minecraft 1.21.1 客户端的公开 `GsonHelper`、`AnimationMetadataSectionSerializer` 与 `AnimationMetadataSection` 接口，以及同版本 `SpriteContents` 的静态上传规则。参照生成工具为 `tools/generate_java_resource_reference.py` 和 `tools/JavaResourceReference.java`；数据及客户端、依赖和输出哈希位于 `crates/mcedit2-world/tests/fixtures/java-resource-1.21.1.json` 及相邻 `.proof.json`。工具不使用反射，不启动游戏，也不修改原客户端。

## 资源对象与覆盖

模型、方块状态和图片元数据读取首个标准 JSON 对象后返回，与客户端资源入口一致。对象内部的语法错误仍报错；对象后的未消费内容记录字节范围和来源。NBT 文本等已有的整文档解析入口继续使用自己的合同。

资源索引选择上层定义后，旧模型不再参与解析；新模型损坏会产生缺失模型诊断。超过单项安全预算直接报告错误。图片及其 `.mcmeta` 分别保留来源，元数据单独覆盖图片时，诊断同时显示图片文件和实际元数据所在的目录文件或 ZIP/JAR 成员。

## 帧选择与实时动画

生产 Java 资源路径保留动画源，按 20 Hz 时钟播放。初始上传和没有可播放时序的素材遵循客户端规则：

- 缺少 `animation` 时保留整张图片；空动画对象按帧尺寸切分。宽高的未知标记为 `-1`，单独指定一轴时另一轴取原图尺寸。
- 数字字段使用 Java `Number.intValue` 的小数截断和低 32 位转换，避免经过浮点中间值改变帧索引。
- 帧对象的负索引、非正显式时间及错误字段类型报告元数据错误。客户端忽略的空项和数组项不构造帧；序列为空时使用全部顺序帧。
- 越界帧项按客户端规则记录诊断并移出序列，同时报告未使用帧的范围。有效序列至少两项时选择第一项；只剩零项或一项时不创建动画，静态上传从左上角开始。例如两帧图片使用 `[1]` 时，实际静态上传第零帧。
- 物理帧数由已通过图片内存预算的尺寸决定，不设置 4096 个物理帧的固定限制。全顺序时序使用隐式索引；显式帧列表受元数据与工作集预算约束。

每项可使用默认时长或独立时长，周期用 `u64` 计量，跳过未呈现时段后按实际经过的时钟定位，不累积逐帧任务。`interpolate` 按客户端布尔转换读取；插值采用 double 计算并向零截断 RGB，Alpha 保留当前源帧。各源帧的 mip 在加载阶段独立准备，播放复用已准备的帧；插值发生在对应的源帧和 mip 像素之间，覆盖率校正保持当前帧规则。

字体保留完整图片，不把字形图当作动画帧裁剪；材质保留初始帧和共享动画源，两种合同共用一次 PNG 解码。原图 RGBA、时序、元数据原文、来源索引、诊断和在途图片分别计入资源工作集；RGBA 统计不混入时序节点。动画图集另外计量实际使用的源帧 / mip 及当前、交接、下一帧工作空间，不在内存不足时静默冻结或丢帧。工作线程正常传递读取与解码结果；线程提前退出使整次加载失败，不捕获 panic 后继续使用部分结果。

## 图集、视图与拾取

动画保持资源 UV、原页和世界几何身份，快照共享整包不可变 UV 索引。每个源帧先沿用图集的实际缩放和透明边缘处理，再准备独立 mip；小于 16px 的源或受设备合同缩放的高分辨率源不会产生不同的槽布局。GPU 更新变化素材的矩形及其边缘、独立 mip 和覆盖率数据，不重新上传整包图集或重建世界网格。同设备的多个视口共享纹理更新，各自保留相机与中间画面；一次重绘事件使用同一时钟输入。原版 JSON、OBJ、流体、实体和运行时追加图集共用此路径。

字体依赖和位图读取复用模型的纹理 ID 归一化，因此字体可与方块材质共用同一 PNG，同时分别保留整图和动画合同；合法的根目录纹理也参与资源索引。不会按文件夹名称猜测字体图片必须来自 `font/`。

透视、完整世界等轴、限定等轴、俯视、切片、四视图和材质光标按可见控件请求下一次重绘，不轮询世界源。CPU 图片固定使用同一动画快照，防止绘制中途跨时刻混用帧；后台同源图片正在计算时不反复取消，完成前继续显示旧的完整图片。源、相机或范围改变仍取消旧请求。错误保留日志，不能把失败当作新图片发布。

透明通道和邻面证明覆盖实际播放的全部源帧；拾取、悬停去重与笔画采样使用实际显示快照，不能用初始帧或当前时钟替代仍在屏幕上的旧帧。资源文件和编辑世界不会因播放动画被修改。

## 告示牌与资源字体

告示牌读取原始旧 Text1–Text4 或现代 front_text / back_text，保留组件样式、染料、发光和封蜡来源；普通、墙挂、悬挂和墙挂悬挂外形使用已有官方模型层与资源贴图。文字字形来自同一资源栈的 bitmap / space / reference / unihex / TTF 提供器，字体位图保留整图，字体 JSON 复用允许注释的完整文档解析，拒绝未消费的尾部；不套用模型资源的首对象合同。缺少字形时显示原版缺字框并记录诊断；缺资源或无效提供器正常返回错误，不使用系统字体代替。

本批静态核对 [官方 1.21.1 元数据](https://piston-meta.mojang.com/v1/packages/018296bcb207bea6fd25b05f08eee3942dcec406/1.21.1.json) 声明的客户端和映射。复用本机客户端，SHA-1 为 `30c73b1c5da787909b2f73340419fdf13b9def88`；映射读取前按元数据 SHA-1 核对。通过 javap 读取 `FontManager`（flb）的 Gson Reader 完整消费入口、`SignRenderer`（ghn）、`Font` / `StringRenderOutput`（fhx / fhx$b）、`StringSplitter.LineBreakFinder`（fgz$b）、Bitmap / Unihex 和 TrueType Glyph 的真实字节码，没有反射、游戏启动或新分析 JAR。

断行先记录当前空格，再以已读的非零度量判断是否溢出；字形使用实际 advance 和浮点位置，行宽向上取整后做整数居中。描边按字形的 shadow offset 排列八向偏移，unihex 为半像素。正文装饰线在整行字形之后发射，删除线位于 3.5–4.5、下划线位于 8–9，保留独立的 0.01 文字空间深度；描边输出不额外绘制装饰线。牌体、文字、运行时图集和拾取索引保持同一源快照；平面联合光栅与结构预览使用相同四角。

混淆字形动画、双向文字与复杂脚本排版、其他版本的准确字体合同及真实窗口 / 游戏对照仍未闭合。静态字体和字形缓存出现错误时正常传播，不清空中毒缓存后继续运行。

## 三版本实体模型

1.12.2、1.20.1、1.21.1 分别从准确客户端导出 15、204、213 个模型层，包含源部件树、可见性、角点和归一化 UV。1.20.1 与 1.21.1 经过完整公开 API 输出核对的 200 层共用不可变目录；仅竹筏、竹筏运输船、蝙蝠、骆驼四层单独保留，运行时通过 Arc 共享已核对的数据，不复制整个新目录。导出工具为 `generate_entity_layers.py`、`generate_legacy_entity_layers.py`，数据、独立角点参照及摘要均保留在 assets / tests/fixtures。

现代参照直接调用 ModelPart / Cube 与实际 VertexConsumer。旧客户端 ModelBox 和 TexturedQuad 的公开绘制 API 最后会提交 OpenGL，因此只在临时分析副本中将 `bul.a(BufferBuilder)` 的 GPU 提交替换为读取实际 XYZ / UV 并重置缓冲；模型、构造器、ModelBox、TexturedQuad 及其余类文件保持逐字节一致，原客户端不改动。旧版角点姿态另由客户端声明且已核对摘要的 LWJGL Matrix4f 计算，记录适配前后类摘要及独立参照摘要；临时分析副本和编译类随工具退出删除，没有反射或游戏窗口。

幼年参照使用 1.20.1 / 1.21.1 实际 EntityModel.renderToBuffer，各 22 组成年 / 幼年、俯仰角及蜘蛛静止展开顶点；生产绑定只对原版具有对应状态的实体启用幼年。旧牛 / 猪的幼年偏移和 16 色羊毛来自其实际客户端 API；牛头偏移旧版为 8、6，现代为 10、4，几何共享不能代替各版本的渲染参数核对。旧玩家采用 UUID 奇偶默认皮肤，现代采用 18 种默认皮肤。实体和玩家的版本来自自身文件，嵌入玩家仅可使用同文件 Data 的版本；结构预览使用结构保存的版本。未知版本保留诊断，不猜测世界版本。蜘蛛的静止八腿展开读取实际 setupAnim 规则，并追加公开渲染输出核对；旧潜影贝依实际构造尺寸和开盖更新确认 a 为底座、b 为盖子，避免将字段次序误当作部件职责。

## 边界

零字节 PNG、图片 CRC 损坏、对象内部的错误 JSON 和尚未绑定的运行时资源继续报告来源，不能据此声称整个实例资源已经还原。NeoForge OBJ/MTL 已进入生产 Java 路径；旧的实验 GLTF 导入器仍是测试路径，正式插件或显式导入入口尚待接通。动画当前参照 1.21.1 的资源和时序合同；更多版本、运行时生成纹理以及真实窗口 / 游戏对照仍待完成。

## 挂画的注册定义与图像来源

挂画以源 NBT 的 DataVersion 选择 1.12.2 / 1.20.1 / 1.21.1 目录；1.21.1 的定义继续按世界实际启用数据包、过滤器及 overlay 顺序覆盖。资源包只提供所有命名空间的 `textures/painting/` 图像，其 `data/painting_variant/` 不参与世界注册表。旧客户端使用原画集矩形，现代客户端使用实际 asset_id 的独立画面与背板；没有相应资源时报告具体来源错误。

`tools/generate_painting_reference.py` 从准确客户端导出 26 / 30 / 50 项目录、原生角点 / UV 及现代悬挂包围盒。临时分析副本只将无状态数值入口公开为静态函数，并将 UV / 光照 / 绘制提交接到 CPU，不使用反射。来源摘要记录原客户端、适配成员、适配器源码及最终目录 / 顶点文件的 SHA-256；其余归档成员逐字节核实。临时 JAR、class 和分析日志用后清理。

## 悬挂坐标与结构方向

`tools/generate_hanging_reference.py` 通过三个准确客户端的公开构造器和 NBT / 坐标 / 包围盒接口读取展示框数据，不使用反射、私有入口或窗口。1.12.2 的 Facing 为南 / 西 / 北 / 东四向，1.20.1 与 1.21.1 的 Facing 为下 / 上 / 北 / 南 / 西 / 东六向；画作仍使用各自源版本的水平编码。数值参照与客户端 / 输出摘要放在世界测试夹具中。

结构变换保留原版固定尺寸，按锚点重建 Pos；缩放不会把画作或展示框本身拉伸。普通展示物的 ItemRotation 以 45 度步进，地图以 90 度步进，保留地图编码中不影响显示的高位。捕获的注册画作定义随原生结构保存，目标定义不同则报告错误；普通资源包图像不决定尺寸。原版结构的画作 blockPos 使用 Tile 锚点，其余实体使用连续 Pos 所在格；Minecraft / Sponge / legacy 文件中的根、乘客和 Riding 均转换到文件的局部原点。

1.12.2 的 Entity / HangingEntity / ItemFrame 不读写 Invisible NBT，旧版额外同名字段保持原始载荷，但不解释为现代展示框的隐藏状态。

## 展示框与 fixed 物品上下文

`tools/generate_frame_model_reference.py` 从准确 1.12.2 / 1.20.1 / 1.21.1 客户端读取 block/item_frame、地图 / 荧光变体及完整父链，校验客户端 SHA1 后生成 `crates/mcedit2-iced/tests/fixtures/frame-models.json`；不启动窗口，不使用反射。实际渲染读取用户当前资源栈中的这些模型及纹理，依赖裁剪保留框体父链；不以测试导出替代用户资源。

物品模型图接收显示上下文，GUI 和场景 fixed 分别读取 display 及 separate_transforms 对应分支。场景复用普通元素、OBJ、模型层、实际染色和 builtin/entity 专用物品的同一准备实现；先联合运行时图片，再建立整帧图集和同源拾取几何。现代框体依据原版 ItemFrameRenderer 对 Sheets.solidBlockSheet 的直接调用写入不透明深度；旧版框体保持 Alpha Test。普通物品 / 地图旋转、框体偏移、隐藏框深度和六向基底遵循原版调用顺序。

1.12.2 的 stone_slab:2 确实注册为 old_wood_slab，但原客户端没有对应 JSON；生产路径正常返回模型缺失错误，既有图标类别和展示框渲染类别分别验证此结果，不生成替代资源。附魔光效按用户要求不在迁移范围；世界上下文属性继续迁移，挂画及同类对象按真实定义做通用适配。

## 通用生成物品与冻结画作定义

生成物品的侧面按实际 PNG 全部有效动画帧的边界跨度构建，同方向 / 锚点跨空隙和帧合并，UV 保留跨度内的真实像素区间；没有逐物品分支。物理存在但未被实际时序使用的帧不产生轮廓。几何以资源身份缓存，播放只改变图集快照，不重新扫描当前帧。元素和侧面共同使用实际源法向校正绕序，保留原三角对角线。

`tools/generate_generated_item_reference.py` 和 `GeneratedItemCpuAdapter.java` 在准确 1.21.1 分析副本调用原 ItemModelGenerator / FaceBakery，输出 `crates/mcedit2-iced/tests/fixtures/generated-item-1.21.1.json` 及证明：7 组静态、镂空、非二值 Alpha、动画换形和未播放帧，67 面 / 268 角点。CPU 边界提供给定像素、单位 sprite UV 和零收缩比，原生合并、元素生成、实际顶点 / UV 和绕序计算保持不变；不使用反射，不启动窗口，不改原客户端。

原生结构保存的画作注册定义同时进入预览，使用与结构变换相同的 Source.read_bound；资源图像与源尺寸分别取自当前资源栈和捕获定义。旧画集存在明确矩形元数据时使用该元数据，不靠逐画作列举补分支。当前验证不替代真实窗口、GPU 或游戏重载验收。

## 世界旗帜图案定义

旗帜与盾牌根据实际定义的 asset_id 选择命名空间及图像路径，不根据同名 PNG 创建注册项。1.12.2 / 1.20.1 的目录由准确客户端公开枚举 / 注册表导出，1.21.1 使用准确客户端数据及世界启用数据包；内联 Holder 保留自己的 asset_id 与 translation_key。普通资源包中的 data/ 不覆盖世界注册表。绑定世界时使用实际世界 DataVersion，图像包缺少版本文件不丢失世界定义；没有准确来源时正常报告错误。

数据包先按过滤、包顺序和 overlay 确定最终可见的定义，再解析，旗帜和挂画复用同一合并入口。绘制仍遵守前 16 层规则，完整源列表及 Holder 解释保留；未绘制层的未知注册项也不能冒充有效源。缓存绑定完整资源 / 世界来源身份，修改定义后重读，旧快照继续使用原定义。具体离屏验证和未完成的动画 / 窗口范围见 PROGRESS.md。

## SimpleTexture 的采样合同

整图图片与 atlas 精灵共用一次 PNG 解码，但 texture.blur / texture.clamp 只控制整图采样。缺省为 nearest / repeat；布尔原始值按准确客户端 TextureMetadata serializer 的规则解释。错误附带 PNG 与元数据的实际覆盖来源，禁止对应整图绑定，不把元数据读取失败自动视为缺省；atlas 自己的动画合同继续独立解释。

材质 owner 保存采样设置，CPU 与 WGSL 在材质自己的矩形内执行逐 texel 最近点或双线性寻址。图集首尾像素中心编码先还原为实际源坐标，再按真实图片尺寸计算过滤与 LOD；repeat 不跨入其它素材，clamp 保持边缘常值。需要模糊的 cutout 整图保留原始透明 RGB，不使用 atlas 的透明 RGB 扩展；额外物理槽借用源像素排布，绑定与错误索引计入预算。运行时图片身份包括显式采样设置，旧图集快照不原地修改。

模型层、物品 / 图标、静态三角形及元素在源 UV 越界时按重复周期或 0 / 1 边界裁切，保留位置与颜色插值。多输入护甲、纹饰和旗帜的逐层采样与 Alpha Test 不能由预合成结果完全替代，继续迁移；不以已接入采样元数据宣称其等价。所有本节新增实现均未编译或测试，留待完整迁移后的统一验证。

## 护甲逐层提交与深度模式

准确 1.21.1 HumanoidArmorLayer 遍历实际 ArmorMaterial.Layer 后再提交纹饰；Sheets 依据已注册图案的 decal 选择普通 armorCutoutNoCull 或 armorDecalCutoutNoCull。两者具有相同模型、view offset、无背面剔除、无 mip 与 cutout 阈值；decal 额外使用 EQUAL 深度。当前实现分别绑定 SimpleTexture 底图 / 覆盖及 atlas 纹饰，保留逐层过滤、染色和 Alpha Test，纹饰动画直接复用原 sprite 更新。

EQUAL 进入原面与流式常驻面 GPU 管线、指令缓存 / 间接提交和 CPU 光栅；材质页合并同时核对深度模式，有序源层不因多页重排。decal 不提供独立的拾取覆盖。马铠、狼铠、资源包鞘翅直接使用真实整图；已移除不再使用的护甲预合成模块及运行时组合动画分支。

现代旗帜另有独立合同：ModelBakery.BANNER_BASE / SHIELD_BASE 是对应 sheet 的 atlas Material；BannerRenderer 原底图使用 entitySolid，底色与图案使用 entityNoOutline，后者为透明混合、COLOR_WRITE。前一节“旗帜原底图统一整图”的说明只可用于相应旧版读取合同，现代旗帜 / 盾牌必须继续按准确客户端版本拆分，现有预合成路径尚未完成迁移。本节均为源码实现与静态来源记录，未编译或运行验证。

## 旗帜与盾牌的版本化材质计划

现代原底图是对应 sheet 的 atlas sprite，杆架 / 握柄和布面 / 盾面的第一层用 entitySolid；底色及实际源图案用 entityNoOutline，分别保留自己的 sprite 动画、颜色、低阈值 Alpha、无 mip 和透明颜色混合，深度保持只读。物品、世界与结构预览都使用同一个准备结果，绘制限制仍为前 16 层，完整 Holder 和颜色按实际源校验。图像文件不能创建新的注册图案。

旧客户端 cdj / bvi / bwu / buo 的实际逻辑不同：颜色遮罩与原底图尺寸相同，Alpha 非零时用遮罩红通道作合成 Alpha，RGB 来自原底图乘染料；在原图尺寸上 SrcOver 后绑定整套模型，包含杆架和盾柄。当前保留源 PNG 位深 / 颜色类型事实，按已核实的 8 位 RGBA 遮罩处理；错误正常传播，不复刻原生静默忽略、不匹配尺寸或读取失败后继续绘制。缓存保存图层定义及实际旧图片，不保留现代预合成图。

本节为源码与静态客户端证据，不代表运行通过；旧 AWT 中间舍入 / 色彩模型、现代透明面排序、跨页动画、资源切换与旧快照须在最终统一验证中核对。现有注册表 / 图标 / 世界 / 预览类别已同步新接口，没有新增入口，当前未编译、未测试。


## 史莱姆透明外壳与体积类生物

原生 living 配方接收完整 material，实体装配继续通过共享模型层后端，样式负责最终 tint / ordered。普通原生身体采用无 mip 的整图；资源声明可分别指定 alpha_test 和 depth_write，透明混合不再排斥原生 .1 Alpha 阈值与深度写入。史莱姆内部和外壳使用同一实际 PNG、独立模型与深度合同，完整 UV 和源图片 / 动画身份不变。世界与结构预览继续复用同一编译入口及源拾取。

三版 Size、物理尺寸、眼位与耐火在公共适配层集中读取。恶魂采用真实 4.5 倍模型缩放和零时刻触须姿态；烈焰人按原生三圈杆的零时刻 pivot 展开并使用保存 Rotation，岩浆怪使用闭合的实际分节模型。压缩、攻击阶段、鬼脸切换与逐 tick 动画不由存档字段以外的信息猜测。旧模型已补准确导出定义，计划在完整迁移后统一生成；本节所有新增实现未编译、未测试。


## 末影人搬运与独立方块模型

末影人按实际源版本读取 carried / carriedData 或完整 carriedBlockState；旧注册名、数字状态、metadata 和真实内部状态来自同一准确注册表，颜色查询不借用资源模型重命名后的别名。身体、搬运手臂、加色眼睛与方块层沿世界 / 结构共用入口，搬运层保留原生隐形可见性、实体光照和实际矩阵。

独立方块复用现有资源模型、完整源面、UV、材质与采样，使用明确的固定种子和无世界 BlockColors 索引 0；不构造虚假群系或普通立方体。草色取实际草色表的原生默认采样，默认树叶 / 荷叶和无世界水 / 甘蔗按版本规则解释，未知运行时颜色提供器正常报错。原生旧绘制种子为 0、现代为 42，几何和面纹理共享同一选择参数。

旧 1.12.2 模型选择另接准确坐标 long、稳定权重降序、低 int 高 16 位的加权区间，以及逐个已匹配 multipart 部件递增的种子；缓存规划使用相同规则。现代保持原有随机序列，原始选项直接借用，未新增现代排序列表分配。末影人、蠹虫和末影螨的旧准确导出定义已补入，当前尚未生成。以上均未编译或运行验收，仍待完整迁移后的统一验证。


## 村民服装族与流浪商人装备

村民与僵尸村民共用类型 / 职业 / 等级资源解析和帽元数据，PNG 与元数据依赖同时覆盖两个真实资源目录，保留注册资源的命名空间。类型层的帽可见性按实际模型控制，僵尸村民同时控制独立 head / hat / hat_rim；全部服装继续使用各自原始图片、UV、动画及有序提交，不预合成成不可区分的身体贴图。幼年分组、通用 scale 与僵尸村民转化 / 完全冻结的零时刻偏航由 Rig 与装备共同使用。

旧僵尸村民使用实际六种职业贴图及 ModelZombieVillager 的独立身体 / 内甲 / 外甲；其护甲头部形状、身体与腿额外 .1 膨胀和 32 高 UV 均由准确构造导出，不使用普通僵尸几何替代。现代则沿实际 zombie_villager 模型目录绑定对应三层。

流浪商人保留独立模型、.9375 缩放和原生交叉手臂主手层，真实完整物品使用 ground 显示、固定平移和 X(180)，沿既有物品动态材质 / 持有者上下文 / 图集准备；头部物品另走原生挂接，副手、身体护甲及鞘翅不因存在 NBT 就额外显示。幼年身体缩放、头部偏移和旧版本不存在的类型按准确客户端区分。以上均只实现，旧模型参照仍待完整迁移后统一生成，没有提前编译、测试或开窗。
