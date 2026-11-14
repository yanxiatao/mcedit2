//! 当前实际文档中的动作按键；连续序列携带来源，焦点从原生控件查询。
use super::*;
use crate::keyboard_bindings::{Chord, Owner};
use iced::{
    advanced::widget::{self, operation},
    keyboard::Event,
};
use mcedit2_plugins::tool_pick::ActionState;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct State {
    prefix: Vec<Vec<Chord>>,
    signature: Option<[u8; 32]>,
    began: Option<Instant>,
    focused: Option<String>,
    probing: bool,
    queued: std::collections::VecDeque<Event>,
}
impl State {
    pub(super) fn pending(&self) -> bool {
        self.probing || !self.prefix.is_empty()
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Input {
    event: Event,
    owner: Owner,
    generation: u64,
    input_epoch: u64,
    source: String,
    digest: String,
    signature: [u8; 32],
}
#[derive(Debug, Clone)]
enum Target {
    Command(String),
    Field(form::Key),
}
struct Binding {
    target: Target,
    action: ActionState,
    sequence: Vec<Chord>,
    owner_path: Option<String>,
}
struct Focus {
    focused: Option<widget::Id>,
}
impl widget::Operation<Option<widget::Id>> for Focus {
    fn traverse(&mut self, visit: &mut dyn FnMut(&mut dyn widget::Operation<Option<widget::Id>>)) {
        visit(self);
    }
    fn focusable(
        &mut self,
        id: Option<&widget::Id>,
        _bounds: iced::Rectangle,
        state: &mut dyn operation::Focusable,
    ) {
        if state.is_focused() {
            self.focused = id.cloned();
        }
    }
    fn finish(&self) -> operation::Outcome<Option<widget::Id>> {
        operation::Outcome::Some(self.focused.clone())
    }
}
fn bindings(app: &App, owner: Owner) -> Result<Vec<Binding>, String> {
    let mut output = Vec::new();
    for command in &app.plugins.commands {
        if let Some(state) = &command.action {
            push(
                &mut output,
                Target::Command(command.id.clone()),
                state.clone(),
                None,
            )?;
        }
        if Some(&command.id) == app.plugins.command.as_ref() {
            for (index, spec) in command.options.iter().enumerate() {
                if spec.get("enabled").and_then(serde_json::Value::as_bool) == Some(false)
                    || spec.get("visible").and_then(serde_json::Value::as_bool) == Some(false)
                {
                    continue;
                }
                if let Some(state) = presentation::action(spec)? {
                    push(
                        &mut output,
                        Target::Field(form::key(owner, app.plugins.form_generation, index)),
                        state,
                        spec.get("shortcut_owner")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                    )?;
                }
            }
        }
    }
    Ok(output)
}
fn push(
    output: &mut Vec<Binding>,
    target: Target,
    state: ActionState,
    owner_path: Option<String>,
) -> Result<(), String> {
    if !state.enabled || !state.visible {
        return Ok(());
    }
    for keys in &state.shortcuts {
        let sequence = keys
            .iter()
            .map(|key| Chord::parse_action(key))
            .collect::<Result<Vec<_>, _>>()?;
        output.push(Binding {
            target: target.clone(),
            action: state.clone(),
            sequence,
            owner_path: owner_path.clone(),
        });
    }
    Ok(())
}
fn ready(app: &App) -> Option<Owner> {
    if app.plugins.commands.is_empty()
        || app.plugins.loaded_digest.is_none()
        || app.desktop.show_recent
        || app.plugins.busy
        || app.plugins.tool_stopping
        || app.plugins.pending_pick.is_some()
        || app.workspace.closing.is_some()
        || app.shortcuts.open
        || app.block_picker.open()
    {
        return None;
    }
    let owner = crate::keyboard_bindings::owner(app)?;
    if let Owner::Structure(id) = owner {
        if !crate::structure_workspace::plugin::parameters_ready(app, id) {
            return None;
        }
    }
    Some(owner)
}
fn starts_with(sequence: &[Chord], prefix: &[Vec<Chord>]) -> bool {
    sequence.len() >= prefix.len()
        && sequence
            .iter()
            .zip(prefix)
            .all(|(key, actual)| actual.contains(key))
}
fn signature(app: &App, owner: Owner) -> Result<[u8; 32], String> {
    let source = app.plugins.source()?.0;
    let rows = app
        .plugins
        .commands
        .iter()
        .map(|command| {
            let fields = if app.plugins.command.as_ref() == Some(&command.id) {
                command
                    .options
                    .iter()
                    .map(|spec| {
                        (
                            spec.get("name"),
                            spec.get("action"),
                            spec.get("enabled"),
                            spec.get("visible"),
                            spec.get("shortcut_owner"),
                            spec.get("widget_path"),
                        )
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            (&command.id, &command.action, fields)
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&(
        format!("{owner:?}"),
        app.input_epoch,
        source.id,
        source.digest,
        rows,
    ))
    .map_err(|error| error.to_string())?;
    Ok(Sha256::digest(bytes).into())
}
fn error(app: &mut App, message: String) -> Task<Message> {
    app.plugins.shortcuts = State::default();
    log::error!("[plugin-shortcut] {message}");
    app.plugins.report = message;
    Task::none()
}
pub(super) fn keyboard(app: &mut App, event: &Event) -> Option<Task<Message>> {
    let Event::KeyPressed { .. } = event else {
        return None;
    };
    let owner = match ready(app) {
        Some(owner) => owner,
        None => {
            app.plugins.shortcuts = State::default();
            return None;
        }
    };
    let chords = Chord::action_event(event);
    if chords.is_empty() {
        return None;
    }
    if chords.iter().all(Chord::is_modifier) {
        return None;
    }
    if app.plugins.shortcuts.probing {
        let result = app.block_edit.budget.parse().and_then(|budget| {
            budget
                .check(
                    (app.plugins.shortcuts.queued.len() + 1)
                        .saturating_mul(std::mem::size_of::<Event>() + 256),
                )
                .map_err(|error| error.to_string())
        });
        if let Err(message) = result {
            return Some(error(app, message));
        }
        app.plugins.shortcuts.queued.push_back(event.clone());
        return Some(Task::none());
    }
    if app.plugins.shortcuts.prefix.is_empty() && app.shortcuts.binds_key(event) {
        app.plugins.shortcuts = State::default();
        return None;
    }
    let result = (|| {
        let signature = signature(app, owner)?;
        if app.plugins.shortcuts.signature != Some(signature)
            || app
                .plugins
                .shortcuts
                .began
                .is_some_and(|began| began.elapsed() > Duration::from_secs(2))
        {
            app.plugins.shortcuts = State {
                signature: Some(signature),
                ..State::default()
            };
        }
        let mut prefix = app.plugins.shortcuts.prefix.clone();
        prefix.push(chords);
        if !bindings(app, owner)?
            .iter()
            .any(|binding| starts_with(&binding.sequence, &prefix))
        {
            app.plugins.shortcuts = State::default();
            return Ok(None);
        }
        let source = app.plugins.source()?.0;
        app.plugins.shortcuts.probing = true;
        let input = Input {
            event: event.clone(),
            owner,
            generation: app.plugins.form_generation,
            input_epoch: app.input_epoch,
            source: source.id,
            digest: source.digest,
            signature,
        };
        Ok(Some(
            iced::advanced::widget::operate(Focus { focused: None }).map(move |focused| {
                Message::Plugin(Action::ShortcutFocused(input.clone(), focused))
            }),
        ))
    })();
    match result {
        Ok(task) => task,
        Err(message) => Some(error(app, message)),
    }
}
pub(super) fn focused(app: &mut App, input: Input, focused: Option<widget::Id>) -> Task<Message> {
    let mut queued = std::mem::take(&mut app.plugins.shortcuts.queued);
    app.plugins.shortcuts.probing = false;
    let mut tasks = vec![focused_inner(app, input.clone(), focused)];
    while let Some(event) = queued.pop_front() {
        if crate::keyboard_bindings::owner(app) != Some(input.owner)
            || app.input_epoch != input.input_epoch
        {
            break;
        }
        match signature(app, input.owner) {
            Ok(signature) if signature == input.signature => (),
            Ok(_) => break,
            Err(message) => {
                tasks.push(error(app, message));
                break;
            }
        }
        tasks.push(crate::keyboard_bindings::keyboard(app, event));
        if app.plugins.shortcuts.probing {
            app.plugins.shortcuts.queued.extend(queued);
            break;
        }
    }
    Task::batch(tasks)
}
fn focused_inner(app: &mut App, input: Input, focused: Option<widget::Id>) -> Task<Message> {
    let result = (|| {
        if ready(app) != Some(input.owner)
            || signature(app, input.owner)? != input.signature
            || app.input_epoch != input.input_epoch
        {
            return Err("快捷键所属文档或表单已改变".into());
        }
        let source = app.plugins.source()?.0;
        if source.id != input.source || source.digest != input.digest {
            return Err("快捷键插件来源已改变".into());
        }
        let Event::KeyPressed { repeat, .. } = &input.event else {
            unreachable!("仅派发按下事件");
        };
        let chords = Chord::action_event(&input.event);
        if *repeat && !app.plugins.shortcuts.prefix.is_empty() {
            return Ok(Some(Task::none()));
        }
        let focus_path = app
            .plugins
            .commands
            .iter()
            .find(|command| Some(&command.id) == app.plugins.command.as_ref())
            .and_then(|command| {
                command
                    .options
                    .iter()
                    .enumerate()
                    .find_map(|(index, spec)| {
                        ((focused.as_ref()
                            == Some(&form::focus_id(form::key(
                                input.owner,
                                app.plugins.form_generation,
                                index,
                            ))))
                            || focused.as_ref()
                                == Some(&form::focus_id(form::key(
                                    input.owner,
                                    input.generation,
                                    index,
                                ))))
                        .then(|| spec.get("widget_path").and_then(serde_json::Value::as_str))
                        .flatten()
                    })
            });
        let focus_key = focus_path
            .map(str::to_owned)
            .or_else(|| focused.as_ref().map(|id| format!("{id:?}")));
        if app.plugins.shortcuts.focused != focus_key {
            app.plugins.shortcuts.prefix.clear();
            app.plugins.shortcuts.began = None;
        }
        app.plugins.shortcuts.focused = focus_key;
        let mut prefix = app.plugins.shortcuts.prefix.clone();
        prefix.push(chords);
        let eligible = bindings(app, input.owner)?
            .into_iter()
            .filter(|binding| {
                let scope = match binding.action.shortcut_context {
                    0 => binding
                        .owner_path
                        .as_deref()
                        .is_some_and(|path| focus_path == Some(path)),
                    3 => binding.owner_path.as_deref().is_some_and(|path| {
                        focus_path.is_some_and(|focus| {
                            focus == path || focus.starts_with(&format!("{path}_"))
                        })
                    }),
                    _ => true,
                };
                scope
                    && (!*repeat || binding.action.auto_repeat)
                    && starts_with(&binding.sequence, &prefix)
            })
            .collect::<Vec<_>>();
        if eligible.is_empty() {
            app.plugins.shortcuts = State::default();
            return Ok(None);
        }
        let mut exact = BTreeMap::new();
        for binding in &eligible {
            if binding.sequence.len() == prefix.len() {
                let identity = binding.action.action_id.map_or_else(
                    || format!("{:?}", binding.target),
                    |id| format!("action:{id}"),
                );
                exact.insert(identity, binding.target.clone());
            }
        }
        if exact.len() > 1
            || (!exact.is_empty()
                && eligible
                    .iter()
                    .any(|binding| binding.sequence.len() > prefix.len()))
        {
            return Err("插件快捷键存在重复或前缀冲突，请修改动作绑定".into());
        }
        if let Some((_, target)) = exact.into_iter().next() {
            app.plugins.shortcuts = State::default();
            return Ok(Some(match target {
                Target::Field(key) => form::update(app, form::Action::Invoke(key))?,
                Target::Command(command) => {
                    if command.starts_with("tool:") {
                        if tool_active(app)
                            && app.plugins.command.as_ref() == Some(&command)
                            && app.plugins.class_tool_selected()
                        {
                            update(app, Action::ToolAction)
                        } else {
                            pick::handoff_command_action(app, command, true)
                        }
                    } else {
                        pick::handoff_command_action(app, command, false)
                    }
                }
            }));
        }
        app.plugins.shortcuts.prefix = prefix;
        app.plugins.shortcuts.began = Some(Instant::now());
        Ok(Some(Task::none()))
    })();
    match result {
        Ok(Some(task)) => task,
        Ok(None) => crate::keyboard_bindings::keyboard_continuation(app, input.event),
        Err(message) => error(app, message),
    }
}
