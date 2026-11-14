//! 插件私有目录与原生应用操作；文件加载先准备真实来源，再共同提交目录和内容。
use mcedit2_world::editing::{script, EditWorkBudget};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Directory {
    pub tabs: Vec<Tab>,
    pub active_tab: Option<String>,
}
impl Directory {
    pub fn identities(&self, count: usize) -> Result<Vec<String>, String> {
        let mut ids = std::collections::BTreeSet::new();
        let mut documents = std::collections::BTreeMap::new();
        for tab in &self.tabs {
            if !ids.insert(tab.identity.as_str()) {
                return Err("应用标签身份重复".into());
            }
            if let Some(slot) = tab.document {
                if slot >= count || documents.insert(slot, tab.identity.clone()).is_some() {
                    return Err("应用标签与真实文档槽位不同".into());
                }
            }
        }
        if documents.len() != count
            || self
                .active_tab
                .as_ref()
                .is_some_and(|active| !ids.contains(active.as_str()))
        {
            return Err("应用目录缺少实际文档或活动标签".into());
        }
        Ok(documents.into_values().collect())
    }
    /// 私有目录只提前反映可确定的导航；关闭是否获准仍由真实文档复核决定。
    pub fn navigate(&mut self, action: &Action) -> Result<(), String> {
        let locate = |identity: &str| {
            self.tabs
                .iter()
                .position(|tab| tab.identity == identity)
                .ok_or_else(|| "应用目标标签已经关闭或不属于本次目录".to_owned())
        };
        match action {
            Action::Activate { identity } => {
                locate(identity)?;
                self.active_tab = Some(identity.clone());
            }
            Action::Move {
                identity,
                anchor,
                after,
            } => {
                let source = locate(identity)?;
                let anchor = locate(anchor)?;
                if source != anchor {
                    let tab = self.tabs.remove(source);
                    let target = anchor - usize::from(source < anchor) + usize::from(*after);
                    self.tabs.insert(target, tab);
                }
            }
            _ => (),
        }
        Ok(())
    }
}
/// 两种解释器入口共用的回调内目录；loaded 只在真实文档安装成功后返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub directory: Directory,
    pub loaded: Option<Document>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub slot: usize,
    pub info: serde_json::Value,
}
/// 预检包含真实读取结果、应用目录、请求和返回值；安装前不改变任一来源。
pub struct PreparedOpen {
    revision: Arc<()>,
    document: script::PreparedDocument,
    action: Action,
    reply: Reply,
}
impl PreparedOpen {
    pub fn reply(&self) -> &Reply {
        &self.reply
    }
    pub fn budget(&self, working: &script::Working) -> Result<EditWorkBudget, String> {
        working
            .document_budget(&self.document)
            .map_err(|error| error.to_string())
    }
}
/// 两种解释器入口共用目标校验、路径语义及实际请求预算。
pub struct Requests {
    directory: Directory,
    actions: Vec<Action>,
    scope: u64,
    revision: Arc<()>,
}
impl Requests {
    pub fn new(
        directory: Directory,
        documents: usize,
        budget: EditWorkBudget,
    ) -> Result<Self, String> {
        directory.identities(documents)?;
        let result = Self {
            directory,
            actions: Vec::new(),
            scope: NEXT_SCOPE
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| "应用请求身份序号已耗尽")?,
            revision: Arc::new(()),
        };
        result.check_budget(budget)?;
        Ok(result)
    }
    fn check_budget(&self, budget: EditWorkBudget) -> Result<(), String> {
        self.check_proposed(&self.directory, &self.actions, None, budget)
    }
    fn check_proposed(
        &self,
        directory: &Directory,
        actions: &[Action],
        reply: Option<&Reply>,
        budget: EditWorkBudget,
    ) -> Result<(), String> {
        let bytes =
            crate::transport::serialized_size(&(directory, actions, reply), budget.bytes() as u64)?;
        budget
            .check(
                usize::try_from(bytes)
                    .map_err(|_| "应用请求超出地址范围")?
                    .checked_mul(16)
                    .ok_or("应用请求工作集溢出")?,
            )
            .map_err(|error| error.to_string())
    }
    fn allow_action(&self) -> Result<(), String> {
        if self
            .actions
            .iter()
            .any(|action| matches!(action, Action::Exit))
        {
            return Err("退出请求之后不能继续追加应用操作".into());
        }
        Ok(())
    }
    /// 已有会话的切换和排序立即进入私有目录；关闭仍须等待原生未保存复核。
    pub fn push(&mut self, mut action: Action, budget: EditWorkBudget) -> Result<Reply, String> {
        self.allow_action()?;
        if matches!(action, Action::CloseAll) {
            action = Action::CloseTabs {
                identities: self
                    .directory
                    .tabs
                    .iter()
                    .map(|tab| tab.identity.clone())
                    .collect(),
            };
        }
        let require = |identity: &str| -> Result<(), String> {
            self.directory
                .tabs
                .iter()
                .any(|tab| tab.identity == identity)
                .then_some(())
                .ok_or("应用目标标签已经关闭或不属于本次目录".into())
        };
        match &mut action {
            Action::Open { path, .. } => {
                if path.as_os_str().is_empty() {
                    return Err("打开路径不能为空".into());
                }
                if path.is_relative() {
                    *path = std::env::current_dir()
                        .map_err(|error| error.to_string())?
                        .join(&*path);
                }
            }
            Action::Close { identity } => require(identity)?,
            Action::CloseTabs { identities } => {
                for identity in identities {
                    require(identity)?;
                }
            }
            Action::Activate { .. } | Action::Move { .. } => (),
            Action::CloseAll | Action::Exit | Action::WorldList { .. } => (),
        }
        let mut directory = self.directory.clone();
        directory.navigate(&action)?;
        let reply = Reply {
            directory,
            loaded: None,
        };
        self.actions.push(action);
        if let Err(error) =
            self.check_proposed(&reply.directory, &self.actions, Some(&reply), budget)
        {
            self.actions.pop();
            return Err(error);
        }
        self.directory = reply.directory.clone();
        self.revision = Arc::new(());
        Ok(reply)
    }
    /// 调用方可先对整个宿主响应做预检，再交回同一 Requests / Working 安装。
    /// 目录中的每个新槽位都来自读取器，既有路径保留当前私有编辑和原权限。
    pub fn prepare_open(
        &self,
        working: &script::Working,
        path: &std::path::Path,
        options: script::LoadOptions,
    ) -> Result<PreparedOpen, String> {
        self.allow_action()?;
        self.directory.identities(working.document_count())?;
        let document = working
            .prepare_document(path, options)
            .map_err(|error| error.to_string())?;
        let mut info = working
            .prepared_document_info(&document)
            .map_err(|error| error.to_string())?;
        let mut directory = self.directory.clone();
        let slot = document.slot();
        let identity = if document.created() {
            let identity = format!("script-open:{}:{}:{slot}", std::process::id(), self.scope);
            directory.tabs.push(Tab {
                identity: identity.clone(),
                document: Some(slot),
            });
            identity
        } else {
            directory
                .tabs
                .iter()
                .find(|tab| tab.document == Some(slot))
                .expect("目录覆盖全部实际文档")
                .identity
                .clone()
        };
        directory.active_tab = Some(identity.clone());
        directory.identities(working.document_count() + usize::from(document.created()))?;
        info["identity"] = serde_json::json!(identity);
        let action = Action::Activate { identity };
        let reply = Reply {
            directory,
            loaded: Some(Document { slot, info }),
        };
        let budget = working
            .document_budget(&document)
            .map_err(|error| error.to_string())?;
        let mut actions = self.actions.clone();
        actions.push(action.clone());
        self.check_proposed(&reply.directory, &actions, Some(&reply), budget)?;
        Ok(PreparedOpen {
            revision: Arc::clone(&self.revision),
            document,
            action,
            reply,
        })
    }
    pub fn install_open(
        &mut self,
        working: &mut script::Working,
        prepared: PreparedOpen,
    ) -> Result<Reply, String> {
        if !Arc::ptr_eq(&self.revision, &prepared.revision) {
            return Err("应用目录在文件准备之后已改变，不能安装过期结果".into());
        }
        let mut info = working
            .prepared_document_info(&prepared.document)
            .map_err(|error| error.to_string())?;
        let loaded = prepared
            .reply
            .loaded
            .as_ref()
            .expect("准备打开结果含真实文档元数据");
        info["identity"] = loaded.info["identity"].clone();
        if info != loaded.info {
            return Err("文档会话在文件准备之后已改变，不能发布过期元数据".into());
        }
        let budget = working
            .document_budget(&prepared.document)
            .map_err(|error| error.to_string())?;
        let mut actions = self.actions.clone();
        actions.push(prepared.action.clone());
        self.check_proposed(
            &prepared.reply.directory,
            &actions,
            Some(&prepared.reply),
            budget,
        )?;
        // Working 在自身来源、取消和预算检查全部成功后才安装；此后不再进行可失败步骤。
        working
            .install_document(prepared.document)
            .map_err(|error| error.to_string())?;
        self.directory = prepared.reply.directory.clone();
        self.actions.push(prepared.action);
        self.revision = Arc::new(());
        Ok(prepared.reply)
    }
    pub fn directory(&self) -> &Directory {
        &self.directory
    }
    pub fn finish(self, budget: EditWorkBudget) -> Result<Vec<Action>, String> {
        self.check_budget(budget)?;
        Ok(self.actions)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    pub identity: String,
    pub document: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Open {
        path: PathBuf,
        readonly: bool,
    },
    Activate {
        identity: String,
    },
    Close {
        identity: String,
    },
    Move {
        identity: String,
        anchor: String,
        after: bool,
    },
    CloseAll,
    CloseTabs {
        identities: Vec<String>,
    },
    Exit,
    WorldList {
        visible: bool,
    },
}
