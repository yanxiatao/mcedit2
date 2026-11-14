//! 自动派生字段的显示值与显式赋值意图；历史身份只用于内存中的保存点比较。
use crate::{
    history::StepId,
    nbt_document::{self, Operation, Value as NbtValue},
    sections::parse_root,
    WorldError,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
type Coord = (i32, i32);
fn err(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Field {
    LightOn,
    LightPopulated,
    Status,
}
impl Field {
    pub(crate) const ALL: [Self; 3] = [Self::LightOn, Self::LightPopulated, Self::Status];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::LightOn => "isLightOn",
            Self::LightPopulated => "LightPopulated",
            Self::Status => "Status",
        }
    }
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.name() == name)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum Value {
    Absent,
    Flag(i8),
    Status(String),
}
impl Value {
    fn nbt(&self, field: Field) -> Result<Option<NbtValue>, WorldError> {
        Ok(match (field, self) {
            (_, Self::Absent) => None,
            (Field::LightOn | Field::LightPopulated, Self::Flag(value @ (0 | 1))) => {
                Some(NbtValue::Byte(*value))
            }
            (Field::Status, Self::Status(value)) if !value.is_empty() => {
                Some(NbtValue::String(value.clone()))
            }
            _ => return Err(err("完成字段的赋值类型或范围无效")),
        })
    }
    pub(crate) fn flag(&self) -> Option<bool> {
        match self {
            Self::Flag(value) => Some(*value != 0),
            Self::Absent => None,
            _ => unreachable!("完成标记仅保存 Byte"),
        }
    }
    pub(crate) fn status(&self) -> Option<String> {
        match self {
            Self::Status(value) => Some(value.clone()),
            Self::Absent => None,
            _ => unreachable!("完成状态仅保存 String"),
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Self::Status(value) => value.len(),
            _ => 1,
        }
    }
}
pub(crate) type Values = BTreeMap<Field, Value>;
pub(crate) fn read(raw: &[u8]) -> Result<Values, WorldError> {
    let root = parse_root(raw).map_err(|error| err(error.to_string()))?;
    let container = root.field("Level").unwrap_or(&root);
    Field::ALL
        .into_iter()
        .map(|field| {
            let mut matches = container
                .fields
                .iter()
                .filter(|(name, _)| *name == field.name());
            let value = matches.next().map(|(_, value)| value);
            if matches.next().is_some() {
                return Err(err("完成字段重复"));
            }
            let value = match (field, value) {
                (_, None) => Value::Absent,
                (Field::Status, Some(value)) => {
                    Value::Status(value.string().map_err(|error| err(error.to_string()))?)
                }
                (_, Some(value)) => {
                    Value::Flag(value.byte().map_err(|error| err(error.to_string()))?)
                }
            };
            value.nbt(field)?;
            Ok((field, value))
        })
        .collect()
}
pub(crate) fn write(raw: &[u8], values: &Values) -> Result<Vec<u8>, WorldError> {
    let previous = read(raw)?;
    let root = parse_root(raw).map_err(|error| err(error.to_string()))?;
    let parent = if root.field("Level").is_some() {
        "/Level"
    } else {
        ""
    };
    let mut operations = Vec::new();
    for (&field, value) in values {
        if &previous[&field] == value {
            continue;
        }
        let path = format!("{parent}/{}", field.name());
        operations.push(
            match (previous[&field] == Value::Absent, value.nbt(field)?) {
                (_, None) => Operation::Remove { path },
                (true, Some(value)) => Operation::InsertField {
                    path: parent.into(),
                    name: field.name().into(),
                    value,
                },
                (false, Some(value)) => Operation::Replace { path, value },
            },
        );
    }
    if operations.is_empty() {
        return Ok(raw.to_vec());
    }
    nbt_document::patch_batch(raw, &operations)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Basis {
    pub field: Field,
    pub source: Value,
    pub display: Value,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct Intent {
    identity: StepId,
    value: Value,
}
type Intents = BTreeMap<Field, Intent>;
#[derive(Debug, Clone)]
pub(crate) struct Change {
    lease: StepId,
    before: Intents,
    after: Intents,
    initial: Values,
    physical: Values,
}
impl Change {
    pub(crate) fn verify_physical(&self, raw: &[u8]) -> Result<(), WorldError> {
        if read(raw)? != self.physical {
            return Err(err("完成字段的整组重放来源在预检后已改变"));
        }
        Ok(())
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        (self.before.len() + self.after.len() + self.initial.len() + self.physical.len()) * 128
            + self
                .before
                .values()
                .chain(self.after.values())
                .map(|intent| intent.value.bytes())
                .sum::<usize>()
            + self
                .initial
                .values()
                .chain(self.physical.values())
                .map(Value::bytes)
                .sum::<usize>()
    }
}
#[derive(Debug, Clone)]
struct Chunk {
    lease: StepId,
    initial: Values,
    physical: Values,
    base_present: bool,
    saved: Intents,
    current: Intents,
    anchors: Vec<(Field, StepId, Value)>,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct State {
    chunks: BTreeMap<Coord, Chunk>,
}
impl State {
    pub(crate) fn take_chunks(&mut self, chunks: &BTreeSet<Coord>) -> Self {
        let mut selected = BTreeMap::new();
        for chunk in chunks {
            if let Some(value) = self.chunks.remove(chunk) {
                selected.insert(*chunk, value);
            }
        }
        Self { chunks: selected }
    }
    pub(crate) fn restore_chunks(&mut self, chunks: &BTreeSet<Coord>, selected: &Self) {
        self.chunks.retain(|chunk, _| !chunks.contains(chunk));
        self.chunks.extend(
            selected
                .chunks
                .iter()
                .map(|(chunk, value)| (*chunk, value.clone())),
        );
    }
    pub(crate) fn contains(&self, chunk: Coord) -> bool {
        self.chunks.contains_key(&chunk)
    }
    pub(crate) fn has_history(&self) -> bool {
        !self.chunks.is_empty()
    }
    pub(crate) fn prepare(
        &self,
        chunk: Coord,
        fields: BTreeSet<Field>,
        shown: &[u8],
        after: &[u8],
        physical: &[u8],
    ) -> Result<Option<Change>, WorldError> {
        if fields.is_empty() {
            return Ok(None);
        }
        let initial = read(shown)?;
        let physical = read(physical)?;
        let values = read(after)?;
        let fields = fields
            .into_iter()
            .filter(|field| initial[field] != values[field])
            .collect::<BTreeSet<_>>();
        if fields.is_empty() {
            return Ok(None);
        }
        let before = self
            .chunks
            .get(&chunk)
            .map_or_else(BTreeMap::new, |chunk| chunk.current.clone());
        let mut next = before.clone();
        let identity = StepId::new();
        for field in &fields {
            next.insert(
                *field,
                Intent {
                    identity: identity.clone(),
                    value: values[field].clone(),
                },
            );
        }
        let lease = self
            .chunks
            .get(&chunk)
            .map_or_else(StepId::new, |chunk| chunk.lease.clone());
        Ok(Some(Change {
            lease,
            before,
            after: next,
            initial: fields
                .into_iter()
                .map(|field| (field, initial[&field].clone()))
                .collect(),
            physical,
        }))
    }
    pub(crate) fn check(
        &self,
        chunk: Coord,
        change: &Change,
        after: bool,
    ) -> Result<(), WorldError> {
        let expected = if after { &change.after } else { &change.before };
        let equal = match self.chunks.get(&chunk) {
            Some(chunk) => &chunk.current == expected,
            None => expected.is_empty(),
        };
        if !equal {
            return Err(err("完成字段的历史意图已改变"));
        }
        Ok(())
    }
    pub(crate) fn apply(&mut self, coordinate: Coord, change: &Change) -> Result<(), WorldError> {
        self.check(coordinate, change, false)?;
        let chunk = self.chunks.entry(coordinate).or_insert_with(|| Chunk {
            lease: change.lease.clone(),
            initial: Values::new(),
            physical: Values::new(),
            base_present: true,
            saved: Intents::new(),
            current: Intents::new(),
            anchors: Vec::new(),
        });
        for (field, value) in &change.initial {
            chunk.initial.entry(*field).or_insert_with(|| value.clone());
        }
        chunk.physical = change.physical.clone();
        chunk.base_present = true;
        chunk.current = change.after.clone();
        Ok(())
    }
    pub(crate) fn restore(&mut self, coordinate: Coord, change: &Change, after: bool) {
        self.chunks
            .get_mut(&coordinate)
            .expect("字段历史已准入")
            .current = if after {
            change.after.clone()
        } else {
            change.before.clone()
        };
    }
    fn desired(&self, coordinate: Coord, field: Field) -> Option<&Value> {
        let chunk = self.chunks.get(&coordinate)?;
        if chunk.current.get(&field).map(|intent| &intent.identity)
            == chunk.saved.get(&field).map(|intent| &intent.identity)
        {
            return None;
        }
        Some(match chunk.current.get(&field) {
            Some(intent) => chunk
                .anchors
                .iter()
                .find(|(key, identity, _)| *key == field && identity == &intent.identity)
                .map_or(&intent.value, |(_, _, value)| value),
            None => &chunk.initial[&field],
        })
    }
    pub(crate) fn effective(&self, coordinate: Coord, field: Field, physical: &Value) -> Value {
        self.desired(coordinate, field).unwrap_or(physical).clone()
    }
    pub(crate) fn desired_value(&self, coordinate: Coord, field: Field) -> Option<Value> {
        self.desired(coordinate, field).cloned()
    }
    pub(crate) fn tracked(&self, coordinate: Coord, field: Field) -> bool {
        self.chunks
            .get(&coordinate)
            .is_some_and(|chunk| chunk.initial.contains_key(&field))
    }
    pub(crate) fn pending(&self, coordinate: Coord, field: Field) -> Option<&Value> {
        let desired = self.desired(coordinate, field)?;
        (desired != &self.chunks[&coordinate].physical[&field]).then_some(desired)
    }
    pub(crate) fn has_pending(&self, coordinate: Coord) -> bool {
        Field::ALL
            .into_iter()
            .any(|field| self.pending(coordinate, field).is_some())
    }
    pub(crate) fn hash(&self, coordinate: Coord, hash: &mut Sha256) {
        for field in Field::ALL {
            if let Some(value) = self.pending(coordinate, field) {
                hash.update([32, field as u8]);
                match value {
                    Value::Absent => hash.update([0]),
                    Value::Flag(value) => hash.update([1, *value as u8]),
                    Value::Status(value) => {
                        hash.update([2]);
                        hash.update((value.len() as u64).to_be_bytes());
                        hash.update(value.as_bytes());
                    }
                }
            }
        }
    }
    pub(crate) fn project(
        &self,
        coordinate: Coord,
        raw: &[u8],
        source_base: bool,
    ) -> Result<Vec<u8>, WorldError> {
        let Some(chunk) = self.chunks.get(&coordinate) else {
            return Ok(raw.to_vec());
        };
        let base = if chunk.base_present && !source_base {
            chunk.physical.clone()
        } else {
            read(raw)?
        };
        let values = chunk
            .initial
            .keys()
            .map(|&field| (field, self.effective(coordinate, field, &base[&field])))
            .collect();
        write(raw, &values)
    }
    pub(crate) fn coordinates(&self) -> impl Iterator<Item = Coord> + '_ {
        self.chunks.keys().copied()
    }
    pub(crate) fn saved(&mut self, physical: BTreeMap<Coord, Option<Values>>) {
        for (coordinate, values) in physical {
            let chunk = self
                .chunks
                .get_mut(&coordinate)
                .expect("完成字段来源已捕获");
            chunk.base_present = values.is_some();
            let present = values.is_some();
            let values = match values {
                Some(values) => values,
                None => Field::ALL
                    .into_iter()
                    .map(|field| (field, Value::Absent))
                    .collect(),
            };
            for field in chunk
                .initial
                .keys()
                .copied()
                .filter(|_| present)
                .collect::<Vec<_>>()
            {
                if let Some(intent) = chunk.current.get(&field) {
                    if let Some((_, _, value)) = chunk
                        .anchors
                        .iter_mut()
                        .find(|(key, identity, _)| *key == field && identity == &intent.identity)
                    {
                        *value = values[&field].clone();
                    } else {
                        chunk.anchors.push((
                            field,
                            intent.identity.clone(),
                            values[&field].clone(),
                        ));
                    }
                } else {
                    chunk.initial.insert(field, values[&field].clone());
                }
            }
            chunk.physical = values;
            chunk.saved = chunk.current.clone();
        }
        self.prune();
    }
    pub(crate) fn observed(&mut self, physical: BTreeMap<Coord, Option<Values>>) {
        for (coordinate, values) in physical {
            let chunk = self
                .chunks
                .get_mut(&coordinate)
                .expect("完成字段来源已捕获");
            chunk.base_present = values.is_some();
            chunk.physical = match values {
                Some(values) => values,
                None => Field::ALL
                    .into_iter()
                    .map(|field| (field, Value::Absent))
                    .collect(),
            };
        }
    }
    pub(crate) fn prune(&mut self) {
        for chunk in self.chunks.values_mut() {
            chunk
                .anchors
                .retain(|(_, identity, _)| identity.is_shared());
        }
        self.chunks.retain(|_, chunk| {
            chunk.lease.is_shared() || !chunk.saved.is_empty() || !chunk.current.is_empty()
        });
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        self.chunks
            .values()
            .map(|chunk| {
                256 + (chunk.initial.len()
                    + chunk.physical.len()
                    + chunk.saved.len()
                    + chunk.current.len()
                    + chunk.anchors.len())
                    * 128
                    + chunk
                        .initial
                        .values()
                        .chain(chunk.physical.values())
                        .map(Value::bytes)
                        .sum::<usize>()
                    + chunk
                        .saved
                        .values()
                        .chain(chunk.current.values())
                        .map(|intent| intent.value.bytes())
                        .sum::<usize>()
                    + chunk
                        .anchors
                        .iter()
                        .map(|(_, _, value)| value.bytes())
                        .sum::<usize>()
            })
            .sum()
    }
}
