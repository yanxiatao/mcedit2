//! 工具宿主在同一请求中同步调用原生来源，返回前后都保留完整候选来源。
use crate::*;
use mcedit2_world::editing::{script, snapshot::Content, SelectionRegion};
use std::sync::{atomic::AtomicBool, Arc};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub generation: u64,
    pub revision: u64,
    pub document: serde_json::Value,
    pub world: usize,
    pub catalog: Option<Catalog>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub documents: Vec<serde_json::Value>,
    pub order: Vec<usize>,
    pub primary: usize,
    pub tabs: Vec<application::Tab>,
    pub active_tab: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Local {
    pub blocks: Vec<State>,
    pub content: Content,
    pub bindings: object_receipt::Bindings,
}
impl Local {
    fn output(output: &RunOutput) -> Result<Self, String> {
        Ok(Self {
            blocks: output.blocks.clone().ok_or("工具输出缺少实际方块")?,
            content: output.content.clone().ok_or("工具输出缺少实际对象内容")?,
            bindings: output
                .object_bindings
                .clone()
                .ok_or("原生插件输出缺少对象引用令牌")?,
        })
    }
    fn validate(&self, budget: EditWorkBudget) -> Result<(), String> {
        self.bindings.validate(&self.content)?;
        let bytes = transport::serialized_size(self, budget.bytes() as u64)?;
        budget
            .check(
                usize::try_from(bytes)
                    .map_err(|_| "工具本地工作集超出地址范围")?
                    .checked_mul(16)
                    .ok_or("工具本地工作集溢出")?,
            )
            .map_err(|error| error.to_string())
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Application {
        generation: u64,
        action: application::Action,
    },
    Invoke {
        generation: u64,
        world: usize,
        dimension: String,
        action: serde_json::Value,
        local: Option<Local>,
    },
    Release {
        generation: u64,
        source: u64,
    },
    Budget {
        generation: u64,
    },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Complete { reply: RunnerReply },
    Native { sequence: u64, request: Request },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub sequence: u64,
    pub result: Result<serde_json::Value, String>,
}

pub struct Context {
    working: Option<script::Working>,
    dimension: String,
    document: serde_json::Value,
    primary: usize,
    catalog: Option<Catalog>,
    local: Option<LocalSource>,
    budget: EditWorkBudget,
    generation: u64,
    revision: u64,
    observed_revision: Arc<()>,
    application: Option<application::Requests>,
    opening_readonly: bool,
}
struct LocalSource {
    source: u64,
    snapshot: BlockClipboard,
}
pub struct Completed {
    pub output: script::Output,
    pub objects: Option<mcedit2_world::editing::snapshot::ObjectResults>,
    pub application: Vec<application::Action>,
    pub directory: Option<application::Directory>,
}
pub struct CommandOutput {
    pub output: script::Output,
    pub detached: bool,
    pub application: Vec<application::Action>,
    pub directory: Option<application::Directory>,
}
impl Context {
    pub fn world(
        session: &Session,
        snapshot: &BlockClipboard,
        selection: Option<SelectionRegion>,
        readonly: bool,
        budget: EditWorkBudget,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let dimension = session.dimension().to_owned();
        let input = script::Input {
            world: session.original_world().clone(),
            dimension: dimension.clone(),
            selections: BTreeMap::from([(dimension.clone(), selection)]),
            sessions: BTreeMap::from([(dimension.clone(), session.clone())]),
            readonly,
        };
        Self::world_documents(input, Some(snapshot), readonly, budget, cancelled)
    }
    /// 接入同一世界的实际暂存会话，不能用磁盘内容替换其他维度的未保存编辑。
    pub fn world_documents(
        input: script::Input,
        snapshot: Option<&BlockClipboard>,
        readonly: bool,
        budget: EditWorkBudget,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let dimension = input.dimension.clone();
        let working = script::Working::new(vec![input], readonly, budget, cancelled)
            .map_err(|error| error.to_string())?;
        Self::new(working, 0, dimension, snapshot, budget)
    }
    pub fn structure(
        document: &mcedit2_world::structure_document::Document,
        snapshot: &BlockClipboard,
        selection: Option<SelectionRegion>,
        readonly: bool,
        budget: EditWorkBudget,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let working = script::Working::new_documents(
            Vec::new(),
            vec![mcedit2_world::structure_document::startup::Input {
                document: document.clone(),
                selection,
            }],
            readonly,
            budget,
            cancelled,
        )
        .map_err(|error| error.to_string())?;
        Self::new(
            working,
            0,
            snapshot.dimension.clone(),
            Some(snapshot),
            budget,
        )
    }
    /// 所有槽位来自实际已打开文档，目录顺序与主文档由调用界面明确提供。
    pub fn documents(
        worlds: Vec<script::Input>,
        structures: Vec<mcedit2_world::structure_document::startup::Input>,
        primary: usize,
        order: Vec<usize>,
        identities: Vec<String>,
        snapshot: Option<&BlockClipboard>,
        readonly: bool,
        budget: EditWorkBudget,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let working =
            script::Working::new_documents(worlds, structures, readonly, budget, cancelled)
                .map_err(|error| error.to_string())?;
        let mut documents = working.info()["worlds"]
            .as_array()
            .expect("原生文档目录为数组")
            .clone();
        if documents.len() != identities.len()
            || identities
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != identities.len()
            || order.len() != documents.len()
            || order
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                != (0..documents.len()).collect()
        {
            return Err("插件文档目录与实际槽位或标签顺序不同".into());
        }
        for (document, identity) in documents.iter_mut().zip(identities) {
            document["identity"] = serde_json::json!(identity);
        }
        let dimension = documents
            .get(primary)
            .and_then(|document| document["dimension"].as_str())
            .ok_or("插件主文档不在实际目录中")?
            .to_owned();
        let local_dimension = snapshot.map_or(dimension, |snapshot| snapshot.dimension.clone());
        let mut context = Self::new(working, primary, local_dimension, snapshot, budget)?;
        context.document = documents[primary].clone();
        let tabs = order
            .iter()
            .map(|slot| application::Tab {
                identity: documents[*slot]["identity"]
                    .as_str()
                    .expect("已绑定真实文档身份")
                    .to_owned(),
                document: Some(*slot),
            })
            .collect();
        let active_tab = Some(
            documents[primary]["identity"]
                .as_str()
                .expect("已绑定主文档身份")
                .to_owned(),
        );
        context.catalog = Some(Catalog {
            documents,
            order,
            primary,
            tabs,
            active_tab,
        });
        let catalog = context.catalog.as_ref().expect("已构造实际应用目录");
        context.bind_application(catalog.tabs.clone(), catalog.active_tab.clone())?;
        Ok(context)
    }
    fn new(
        mut working: script::Working,
        primary: usize,
        dimension: String,
        snapshot: Option<&BlockClipboard>,
        budget: EditWorkBudget,
    ) -> Result<Self, String> {
        let observed_revision = working
            .source_revision(primary, &dimension)
            .map_err(|error| error.to_string())?;
        let mut context = Self {
            document: working.info()["worlds"][primary].clone(),
            primary,
            catalog: None,
            working: Some(working),
            dimension,
            local: None,
            budget,
            generation: 0,
            revision: 0,
            observed_revision,
            application: None,
            opening_readonly: false,
        };
        if let Some(snapshot) = snapshot {
            context.capture_local(snapshot)?;
        }
        Ok(context)
    }
    fn working(&mut self) -> Result<&mut script::Working, String> {
        self.working
            .as_mut()
            .ok_or_else(|| "工具原生来源请求已经结束".into())
    }
    pub fn bind_application(
        &mut self,
        tabs: Vec<application::Tab>,
        active: Option<String>,
    ) -> Result<(), String> {
        if self.generation != 0 {
            return Err("已启动的原生请求不能改变应用目录".into());
        }
        let catalog = self.catalog.as_mut().ok_or("应用操作需要实际文档目录")?;
        let directory = application::Directory {
            tabs,
            active_tab: active,
        };
        let identities = directory.identities(catalog.documents.len())?;
        for (document, identity) in catalog.documents.iter().zip(identities) {
            if document["identity"].as_str() != Some(identity.as_str()) {
                return Err("应用标签与真实文档身份不同".into());
            }
        }
        self.application = Some(application::Requests::new(
            directory.clone(),
            catalog.documents.len(),
            self.budget,
        )?);
        catalog.tabs = directory.tabs;
        catalog.active_tab = directory.active_tab;
        Ok(())
    }
    pub fn bind_private_documents(
        &mut self,
        documents: &[script::OpenedOutput],
        opening_readonly: bool,
    ) -> Result<(), String> {
        if self.generation != 0 {
            return Err("已启动的插件请求不能更换私有文档来源".into());
        }
        self.working()?
            .retain_private_documents(documents)
            .map_err(|error| error.to_string())?;
        self.opening_readonly = opening_readonly;
        Ok(())
    }
    fn finish_application(
        &mut self,
        budget: EditWorkBudget,
    ) -> Result<(Vec<application::Action>, Option<application::Directory>), String> {
        match self.application.take() {
            Some(application) => {
                let directory = application.directory().clone();
                Ok((application.finish(budget)?, Some(directory)))
            }
            None => Ok((Vec::new(), None)),
        }
    }
    fn application_action(
        &mut self,
        action: application::Action,
    ) -> Result<serde_json::Value, String> {
        if let application::Action::Open { path, readonly } = action {
            let working = self.working.as_ref().ok_or("工具原生来源请求已经结束")?;
            let prepared = self
                .application
                .as_ref()
                .ok_or("此来源未绑定实际应用目录")?
                .prepare_open(
                    working,
                    &path,
                    script::LoadOptions {
                        readonly: self.opening_readonly || readonly,
                        dimension: None,
                        legacy_version: None,
                    },
                )?;
            let reply =
                serde_json::to_value(prepared.reply()).map_err(|error| error.to_string())?;
            let mut catalog = self.catalog.clone().ok_or("应用加载缺少实际文档目录")?;
            let loaded = prepared
                .reply()
                .loaded
                .as_ref()
                .expect("文件加载已准备真实文档元数据");
            if loaded.slot == catalog.documents.len() {
                catalog.documents.push(loaded.info.clone());
            } else {
                *catalog
                    .documents
                    .get_mut(loaded.slot)
                    .ok_or("实际加载槽位与插件目录不同")? = loaded.info.clone();
            }
            catalog.tabs = prepared.reply().directory.tabs.clone();
            catalog.active_tab = prepared.reply().directory.active_tab.clone();
            catalog.order = catalog.tabs.iter().filter_map(|tab| tab.document).collect();
            let budget = prepared.budget(working)?;
            let bytes = transport::serialized_size(&(&catalog, &reply), budget.bytes() as u64)?;
            budget
                .check(
                    usize::try_from(bytes)
                        .map_err(|_| "加载目录超出地址范围")?
                        .checked_mul(16)
                        .ok_or("加载目录与返回值工作集溢出")?,
                )
                .map_err(|error| error.to_string())?;
            let working = self.working.as_mut().ok_or("工具原生来源请求已经结束")?;
            self.application
                .as_mut()
                .expect("实际应用加载准备结果归属已核对")
                .install_open(working, prepared)?;
            self.document = catalog.documents[self.primary].clone();
            self.catalog = Some(catalog);
            Ok(reply)
        } else {
            let budget = self
                .working()?
                .transient_budget()
                .map_err(|error| error.to_string())?;
            let reply = self
                .application
                .as_mut()
                .ok_or("此来源未绑定实际应用目录")?
                .push(action, budget)?;
            let catalog = self.catalog.as_mut().expect("应用请求绑定实际文档目录");
            catalog.tabs = reply.directory.tabs.clone();
            catalog.active_tab = reply.directory.active_tab.clone();
            catalog.order = catalog.tabs.iter().filter_map(|tab| tab.document).collect();
            serde_json::to_value(reply).map_err(|error| error.to_string())
        }
    }
    /// 收紧外围调用权限，保留真实文档只读标志供 Inspector 的目标表单使用。
    pub fn restrict_to_reads(&mut self) -> Result<(), String> {
        self.working()?.restrict_to_reads();
        Ok(())
    }
    pub(crate) fn source(&mut self, generation: u64) -> Result<Source, String> {
        self.generation = generation;
        self.working()?;
        Ok(Source {
            generation,
            revision: self.revision,
            document: self.document.clone(),
            world: self.primary,
            catalog: self.catalog.clone(),
        })
    }
    pub fn snapshot(&self) -> Option<&BlockClipboard> {
        self.local.as_ref().map(|local| &local.snapshot)
    }
    pub(crate) fn verify_input(&mut self, request: &RunRequest) -> Result<(), String> {
        let mut points = BTreeMap::new();
        if let Some(event) = &request.tool_event {
            points.insert(event.hit, event.state.clone());
        }
        if let Some(gesture) = &request.gesture {
            for sample in &gesture.samples {
                if let Some(location) = &sample.location {
                    if let Some(state) = &location.state {
                        if points
                            .get(&location.position)
                            .is_some_and(|previous| previous != state)
                        {
                            return Err("同次工具输入记录了不同来源的命中状态".into());
                        }
                        points.insert(location.position, state.clone());
                    }
                }
            }
        }
        for (position, expected) in points {
            let dimension = self.dimension.clone();
            let primary = self.primary;
            let actual = self
                .working()?
                .invoke(script::Call {
                    world: primary,
                    dimension,
                    action: script::Action::Read { position },
                })
                .map_err(|error| error.to_string())?;
            let actual: State =
                serde_json::from_value(actual).map_err(|error| error.to_string())?;
            if actual != expected {
                return Err("工具命中的实际方块来源已改变".into());
            }
        }
        Ok(())
    }
    fn capture_local(&mut self, scope: &BlockClipboard) -> Result<serde_json::Value, String> {
        let mut maximum = scope.source_min;
        for axis in 0..3 {
            maximum[axis] = maximum[axis]
                .checked_add(i32::from(scope.size[axis]) - 1)
                .ok_or("工具捕获范围坐标溢出")?;
        }
        let action = script::Action::CaptureMasked {
            region: Bounds {
                minimum: scope.source_min,
                maximum,
            }
            .into(),
            mask: scope.mask.clone(),
            entities: true,
            biomes: scope.biomes.is_some() || scope.numeric_biomes.is_some(),
        };
        let dimension = self.dimension.clone();
        let primary = self.primary;
        let result = self
            .working()?
            .invoke(script::Call {
                world: primary,
                dimension,
                action,
            })
            .map_err(|error| error.to_string())?;
        self.local = Some(LocalSource {
            source: serde_json::from_value(result["context"].clone())
                .map_err(|error| error.to_string())?,
            snapshot: serde_json::from_value(result["snapshot"].clone())
                .map_err(|error| error.to_string())?,
        });
        Ok(result)
    }
    fn flush(
        &mut self,
        local: &Local,
        generated: Option<Arc<BlockClipboard>>,
        creation: Option<mcedit2_world::chunk_creation::ChunkCreationProfile>,
    ) -> Result<script::AppliedOutput, String> {
        local.validate(self.budget)?;
        let source = self
            .local
            .as_ref()
            .ok_or("没有局部捕获的原生命令不能提交本地快照")?
            .source;
        self.working()?
            .apply_frozen_output(source, &local.blocks, &local.content, generated, creation)
            .map_err(|error| error.to_string())
    }
    pub(crate) fn invoke(&mut self, request: Request) -> Result<serde_json::Value, String> {
        let generation = match &request {
            Request::Invoke { generation, .. }
            | Request::Release { generation, .. }
            | Request::Budget { generation } => *generation,
            Request::Application { generation, .. } => *generation,
        };
        if generation != self.generation {
            return Err("工具冻结来源属于已经结束的输入请求".into());
        }
        match request {
            Request::Application { action, .. } => self.application_action(action),
            Request::Budget { .. } => Ok(serde_json::json!(self
                .working()?
                .transient_budget()
                .map_err(|error| error.to_string())?
                .bytes())),
            Request::Release { source, .. } => {
                if self
                    .local
                    .as_ref()
                    .is_some_and(|local| local.source == source)
                {
                    return Err("工具不能释放宿主当前活动来源".into());
                }
                self.working()?
                    .release_snapshot(source)
                    .map_err(|error| error.to_string())?;
                Ok(serde_json::Value::Null)
            }
            Request::Invoke {
                world,
                dimension,
                action,
                local,
                ..
            } => {
                let action: script::Action =
                    serde_json::from_value(action).map_err(|error| error.to_string())?;
                if matches!(&action, script::Action::Apply { .. }) {
                    return Err("完整快照提交必须使用已捕获的真实来源租约".into());
                }
                let before = self
                    .working()?
                    .source_revision(world, &dimension)
                    .map_err(|error| error.to_string())?;
                let had_local = local.is_some();
                let receipt = if let Some(local) = local {
                    let applied = self.flush(&local, None, None)?;
                    Some(object_receipt::prepare_local(
                        &local.bindings,
                        &local.content,
                        applied.objects.as_ref(),
                        self.budget.bytes() as u64,
                    )?)
                } else {
                    None
                };
                let target = dimension.clone();
                let result = self
                    .working()?
                    .invoke(script::Call {
                        world,
                        dimension,
                        action,
                    })
                    .map_err(|error| error.to_string());
                if let Err(error) = &result {
                    log::error!("[plugin-native] {error}");
                }
                let after = self
                    .working()?
                    .source_revision(world, &target)
                    .map_err(|error| error.to_string())?;
                let target_changed = !Arc::ptr_eq(&before, &after);
                let dimension = self.dimension.clone();
                let primary = self.primary;
                let revision = self
                    .working()?
                    .source_revision(primary, &dimension)
                    .map_err(|error| error.to_string())?;
                let changed = !Arc::ptr_eq(&revision, &self.observed_revision);
                let refreshed = if let Some((source, scope)) = self
                    .local
                    .as_ref()
                    .filter(|_| had_local || changed)
                    .map(|local| (local.source, local.snapshot.clone()))
                {
                    self.working()?
                        .release_snapshot(source)
                        .map_err(|error| error.to_string())?;
                    Some(self.capture_local(&scope)?)
                } else {
                    None
                };
                if changed || target_changed {
                    self.observed_revision = revision;
                    self.revision = self.revision.checked_add(1).ok_or("工具来源修订代次溢出")?;
                }
                Ok(serde_json::json!({"result":result,"receipt":receipt,
                        "snapshot":refreshed.as_ref().map(|view| &view["snapshot"]),
                        "content":refreshed.as_ref().map(|view| &view["content"]),"revision":self.revision}))
            }
        }
    }
    pub fn finish(
        &mut self,
        output: &RunOutput,
        creation: Option<mcedit2_world::chunk_creation::ChunkCreationProfile>,
    ) -> Result<Completed, String> {
        let local = Local::output(output)?;
        let applied = self.flush(&local, output.structure.clone().map(Arc::new), creation)?;
        let remaining = self
            .working()?
            .transient_budget()
            .map_err(|error| error.to_string())?;
        let (application, directory) = self.finish_application(remaining)?;
        let working = self.working.take().ok_or("工具原生来源请求已经结束")?;
        Ok(Completed {
            output: working.finish().map_err(|error| error.to_string())?,
            objects: applied.objects,
            application,
            directory,
        })
    }
    /// 有明确捕获范围时组合编辑与生成；无放置范围的生成结果交还界面选择位置。
    pub fn finish_command(
        &mut self,
        output: &RunOutput,
        creation: Option<mcedit2_world::chunk_creation::ChunkCreationProfile>,
    ) -> Result<CommandOutput, String> {
        if self.local.is_some() {
            self.flush(&Local::output(output)?, None, None)?;
        } else if output.blocks.is_some()
            || output.content.is_some()
            || output.object_bindings.is_some()
        {
            return Err("无局部捕获的原生命令返回了不属于此来源的快照内容".into());
        }
        let detached = output.structure.is_some()
            && (self.local.is_none() || !self.working()?.has_content_changes());
        if let Some(generated) = output.structure.as_ref().filter(|_| !detached) {
            let local = self.local.as_ref().expect("组合放置有实际捕获范围");
            let source = local.source;
            let scope = local.snapshot.clone();
            self.working()?
                .release_snapshot(source)
                .map_err(|error| error.to_string())?;
            self.capture_local(&scope)?;
            let local = self.local.as_ref().expect("已捕获实际放置范围");
            let source = local.source;
            let blocks = local.snapshot.blocks.clone();
            let content = Content::capture(&local.snapshot).map_err(|error| error.to_string())?;
            self.working()?
                .apply_frozen_output(
                    source,
                    &blocks,
                    &content,
                    Some(Arc::new(generated.clone())),
                    creation,
                )
                .map_err(|error| error.to_string())?;
        }
        let remaining = self
            .working()?
            .transient_budget()
            .map_err(|error| error.to_string())?;
        let (application, directory) = self.finish_application(remaining)?;
        let working = self.working.take().ok_or("命令原生来源请求已经结束")?;
        Ok(CommandOutput {
            output: working.finish().map_err(|error| error.to_string())?,
            detached,
            application,
            directory,
        })
    }
}
