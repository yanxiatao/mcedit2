//! 文档命令与 Inspector 共用真实多维度来源，编辑结果保留各自确认流程。
use super::*;

#[derive(Debug, Clone)]
pub(super) struct Prepared {
    pub sources: documents::Sources,
    pub preview: Option<world_tool::Candidate>,
    pub generated: Option<Generated>,
}
#[derive(Debug, Clone)]
pub(super) struct Generated {
    pub preview: Option<mcedit2_world::editing::PreparedStructure>,
    pub clipboard: Arc<BlockClipboard>,
}
impl Prepared {
    pub fn matches(&self, app: &App) -> bool {
        self.sources.matches(app)
            && self
                .preview
                .as_ref()
                .is_none_or(|preview| preview.matches(app))
    }
}

pub(super) fn execute(
    executable: &std::path::Path,
    request: &RunRequest,
    limits: &RunLimits,
    documents: world_tool::Documents,
    key: (usize, usize),
    creation: &crate::structure_edit::CreationPanel,
) -> Result<(RunOutput, Prepared), String> {
    let budget = mcedit2_plugins::work_budget(request.work_budget_bytes)?;
    let snapshot = request.snapshot.as_ref();
    let inspecting = request.inspector.is_some();
    let mut context = mcedit2_plugins::tool_rpc::Context::documents(
        documents.worlds,
        documents.structures,
        documents.primary,
        documents.order,
        documents.identities,
        snapshot,
        false,
        budget,
        Arc::clone(&limits.cancelled),
    )?;
    context.bind_application(documents.tabs, documents.active_tab)?;
    context.bind_private_documents(&documents.private_documents, documents.opening_readonly)?;
    if inspecting || request.command.is_none() {
        context.restrict_to_reads()?;
    }
    let native = Arc::new(Mutex::new(context));
    let output = mcedit2_plugins::run_native(executable, request, limits, Arc::clone(&native))?;
    if output.inspector_operation.is_some() && !inspecting {
        return Err("普通命令不能返回 Inspector 专用操作".into());
    }
    if inspecting
        && (output.structure.is_some()
            || !output.keeps_snapshot(snapshot.ok_or("Inspector 缺少实际捕获范围")?)?)
    {
        return Err("Inspector 不能修改世界来源或返回生成结构".into());
    }
    // 先交接实际编辑；没有放置范围时，生成结构交还界面明确选择位置。
    let baseline = &documents.baselines[&key];
    let profile = if output.structure.is_some() && snapshot.is_some() {
        creation.profile(baseline.original_world(), baseline.dimension())?
    } else {
        None
    };
    let completed = native
        .lock()
        .expect("命令原生来源锁被毒化")
        .finish_command(&output, profile.clone())?;
    let sources = documents.sources;
    let mut preview = world_tool::Candidate {
        changes: BTreeMap::new(),
        structures: BTreeMap::new(),
        sources: sources.clone(),
        revision: Arc::new(()),
        clipboard: None,
        effects: session_effects::Effects::default(),
        opened: Default::default(),
    };
    let detached = completed.detached;
    let directory = completed.directory.ok_or("插件命令缺少实际应用目录")?;
    let completed = preview.accept_native(
        completed.output,
        completed.application,
        &directory,
        &documents.views,
        &documents.dimensions,
        &documents.structure_slots,
        budget,
        &limits.cancelled,
        |_| {},
    )?;
    if request.command.is_none() && (preview.effects.pending() || preview.opened.pending()) {
        return Err("加载插件目录不能夹带选区、导航或保存操作".into());
    }
    for change in completed.changes {
        let key = *documents
            .dimensions
            .get(&(change.world, change.dimension))
            .ok_or("普通命令返回了未知维度")?;
        preview.changes.insert(key, change.prepared);
    }
    documents::merge_structures(
        &sources,
        &documents.structure_slots,
        &mut preview.structures,
        completed.structures,
        budget,
    )?;
    let mut generated = None;
    if detached {
        let clip = output.structure.as_ref().expect("独立生成包含实际结构");
        let prepared = if let Some(snapshot) = snapshot {
            Some(mcedit2_plugins::prepare_generated_structure_with_budget(
                baseline,
                snapshot,
                &output,
                profile,
                budget,
                || limits.cancelled.load(Ordering::Acquire),
            )?)
        } else {
            clip.validate_with_budget(baseline.original_world(), baseline.dimension(), budget)
                .map_err(|error| error.to_string())?;
            None
        };
        let clipboard = Arc::new(clip.clone());
        if !preview.has_changes() {
            generated = Some(Generated {
                preview: prepared,
                clipboard,
            });
        } else {
            preview.clipboard = Some(clipboard);
        }
    }
    preview.check_budget(budget)?;
    Ok((
        output,
        Prepared {
            sources,
            preview: preview.has_changes().then_some(preview),
            generated,
        },
    ))
}

pub(super) fn finish(
    app: &mut App,
    key: (usize, usize),
    prepared: Prepared,
    output: RunOutput,
) -> Task<Message> {
    app.plugins.busy = false;
    app.plugins.report = output
        .messages
        .iter()
        .map(|message| format!("[{}] {}", message.level, message.text))
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(preview) = prepared.preview {
        crate::transfer_edit::cancel(app);
        app.clear_block_preview();
        app.nbt_search.invalidate();
        app.object_inspector.invalidate();
        app.pending_import.cancel();
        app.plugins.report.push_str(&format!(
            "\n{}\n{}",
            preview.summary(),
            preview
                .diagnostics()
                .cloned()
                .collect::<Vec<_>>()
                .join("；")
        ));
        app.plugins.structure_preview = Some((key, EditPreview::Snapshot(preview)));
    } else if let Some(generated) = prepared.generated {
        if let Some(preview) = generated.preview {
            app.plugins.report.push_str(&format!(
                "\n结构已预检：将覆盖 {} 个方块实体，其中 {} 个容器物品槽；请确认后暂存。",
                preview.overwritten_block_entities, preview.overwritten_inventory_stacks
            ));
        } else {
            app.plugins
                .report
                .push_str("\n已生成结构，请选择放置位置并确认。");
        }
        return Task::done(Message::PendingClipboard(generated.clipboard));
    } else {
        app.plugins
            .report
            .push_str("\n插件执行完成，没有留下实际内容差异。");
    }
    Task::none()
}
