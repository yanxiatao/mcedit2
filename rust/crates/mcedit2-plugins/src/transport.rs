//! 带类型、顺序和完整性摘要的插件进程分帧协议。
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
const FRAME_BYTES: usize = 1024 * 1024;
const PART_BYTES: usize = 64 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    seq: u64,
    kind: String,
    data: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    root: Value,
    fields: Vec<(String, usize)>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct End {
    frames: u64,
    sha256: String,
}
fn field_mut<'a>(v: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    let mut p = v;
    for part in path.split('.') {
        p = p.get_mut(part)?;
    }
    Some(p)
}
fn paths(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for prefix in [
        "",
        "reply.",
        "request.local.",
        "request.",
        "result.Ok.",
        "result.Ok.result.Ok.",
    ] {
        for field in [
            "snapshot.blocks",
            "snapshot.block_entities",
            "snapshot.entities",
            "snapshot.biomes.cells",
            "snapshot.numeric_biomes.cells",
            "snapshot.metadata",
            "snapshot.mask",
            "output.blocks",
            "output.content.block_entities",
            "output.content.entities",
            "output.content.biomes.cells",
            "output.content.numeric_biomes.cells",
            "output.structure.blocks",
            "output.structure.block_entities",
            "output.structure.entities",
            "output.structure.biomes.cells",
            "output.structure.numeric_biomes.cells",
            "output.structure.metadata",
            "output.structure.mask",
            "output.object_bindings.entities",
            "output.object_bindings.block_entities",
            "blocks",
            "content.entities",
            "content.block_entities",
            "content.biomes.cells",
            "content.numeric_biomes.cells",
            "content.scheduled_ticks",
            "bindings.entities",
            "bindings.block_entities",
            "object_receipt.entities",
            "object_receipt.block_entities",
            "receipt.entities",
            "receipt.block_entities",
            "action.mask",
        ] {
            let p = format!("{prefix}{field}");
            let mut q = v;
            let mut found = true;
            for part in p.split('.') {
                if let Some(next) = q.get(part) {
                    q = next;
                } else {
                    found = false;
                    break;
                }
            }
            if found && q.is_array() {
                out.push(p);
            }
        }
    }
    out
}
fn put_frame(
    w: &mut impl Write,
    frame: &Frame,
    hash: &mut Sha256,
    total: &mut u64,
    budget: u64,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(frame).map_err(|e| e.to_string())?;
    if bytes.len() > FRAME_BYTES {
        return Err("插件单帧超过预算".into());
    }
    *total = total
        .checked_add(bytes.len() as u64 + 4)
        .ok_or("插件传输计数溢出")?;
    if *total > budget {
        return Err("插件传输超过总预算".into());
    }
    let len = u32::try_from(bytes.len())
        .map_err(|e| e.to_string())?
        .to_be_bytes();
    w.write_all(&len)
        .and_then(|_| w.write_all(&bytes))
        .map_err(|e| e.to_string())?;
    hash.update(len);
    hash.update(&bytes);
    Ok(())
}
struct BudgetCounter {
    remaining: u64,
}
impl Write for BudgetCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.remaining = self
            .remaining
            .checked_sub(bytes.len() as u64)
            .ok_or_else(|| std::io::Error::other("插件消息超过总预算"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct Parts<'a, W: Write> {
    writer: &'a mut W,
    kind: String,
    seq: &'a mut u64,
    hash: &'a mut Sha256,
    total: &'a mut u64,
    budget: u64,
    pending: Vec<u8>,
}
impl<W: Write> Parts<'_, W> {
    fn finish_part(&mut self) -> std::io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let data = std::mem::take(&mut self.pending);
        put_frame(
            self.writer,
            &Frame {
                seq: *self.seq,
                kind: self.kind.clone(),
                data,
            },
            self.hash,
            self.total,
            self.budget,
        )
        .map_err(std::io::Error::other)?;
        *self.seq += 1;
        Ok(())
    }
}
impl<W: Write> Write for Parts<'_, W> {
    fn write(&mut self, mut bytes: &[u8]) -> std::io::Result<usize> {
        let len = bytes.len();
        while !bytes.is_empty() {
            let n = (PART_BYTES - self.pending.len()).min(bytes.len());
            self.pending.extend_from_slice(&bytes[..n]);
            bytes = &bytes[n..];
            if self.pending.len() == PART_BYTES {
                self.finish_part()?;
            }
        }
        Ok(len)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.finish_part()
    }
}
/// 宿主在分配 / 发送前按同一序列化合同核对载荷预算，不建立第二份 JSON 缓冲。
pub fn serialized_size<T: Serialize>(value: &T, budget: u64) -> Result<u64, String> {
    let mut counter = BudgetCounter { remaining: budget };
    serde_json::to_writer(&mut counter, value).map_err(|error| error.to_string())?;
    Ok(budget - counter.remaining)
}
pub fn write<T: Serialize>(w: &mut impl Write, value: &T, budget: u64) -> Result<(), String> {
    serde_json::to_writer(&mut BudgetCounter { remaining: budget }, value)
        .map_err(|e| e.to_string())?;
    let mut root = serde_json::to_value(value).map_err(|e| e.to_string())?;
    let mut fields = Vec::new();
    let mut pieces = Vec::new();
    for path in paths(&root) {
        let item = field_mut(&mut root, &path).ok_or("分帧路径不存在")?;
        let array = std::mem::replace(item, Value::Array(Vec::new()));
        let count = array.as_array().ok_or("分帧字段不是数组")?.len();
        fields.push((path.to_string(), count));
        pieces.push((path.to_string(), array));
    }
    let header = serde_json::to_vec(&Header { root, fields }).map_err(|e| e.to_string())?;
    let mut seq = 0;
    let mut hash = Sha256::new();
    let mut total = 0;
    for chunk in header.chunks(PART_BYTES) {
        put_frame(
            w,
            &Frame {
                seq,
                kind: "header".into(),
                data: chunk.to_vec(),
            },
            &mut hash,
            &mut total,
            budget,
        )?;
        seq += 1;
    }
    for (kind, array) in pieces {
        let mut writer = Parts {
            writer: &mut *w,
            kind,
            seq: &mut seq,
            hash: &mut hash,
            total: &mut total,
            budget,
            pending: Vec::with_capacity(PART_BYTES),
        };
        serde_json::to_writer(&mut writer, &array).map_err(|e| e.to_string())?;
        writer.finish_part().map_err(|e| e.to_string())?;
    }
    let end = End {
        frames: seq,
        sha256: format!("{:x}", hash.finalize()),
    };
    let mut ignored = Sha256::new();
    put_frame(
        w,
        &Frame {
            seq,
            kind: "end".into(),
            data: serde_json::to_vec(&end).map_err(|e| e.to_string())?,
        },
        &mut ignored,
        &mut total,
        budget,
    )?;
    w.flush().map_err(|e| e.to_string())
}
pub fn read<T: serde::de::DeserializeOwned>(r: &mut impl Read, budget: u64) -> Result<T, String> {
    let mut seq = 0u64;
    let mut total = 0u64;
    let mut hash = Sha256::new();
    let mut header = Vec::new();
    let mut values = std::collections::BTreeMap::<String, Value>::new();
    let mut current = String::new();
    let mut pending = Vec::<u8>::new();
    let mut order = Vec::new();
    loop {
        let mut len = [0u8; 4];
        r.read_exact(&mut len)
            .map_err(|e| format!("插件分帧不完整：{e}"))?;
        let n = u32::from_be_bytes(len) as usize;
        if n == 0 || n > FRAME_BYTES {
            return Err("插件帧长度无效".into());
        }
        total = total.checked_add(n as u64 + 4).ok_or("插件传输计数溢出")?;
        if total > budget {
            return Err("插件传输超过总预算".into());
        }
        let mut raw = vec![0; n];
        r.read_exact(&mut raw).map_err(|e| e.to_string())?;
        let frame: Frame = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
        if frame.seq != seq {
            return Err("插件帧序号不连续".into());
        }
        if frame.kind == "end" {
            let end: End = serde_json::from_slice(&frame.data).map_err(|e| e.to_string())?;
            if end.frames != seq || end.sha256 != format!("{:x}", hash.finalize()) {
                return Err("插件分帧摘要或数量不匹配".into());
            }
            let mut extra = [0u8; 1];
            if r.read(&mut extra).map_err(|e| e.to_string())? != 0 {
                return Err("插件结束帧后存在额外数据".into());
            }
            if !current.is_empty() {
                let value =
                    serde_json::from_slice(&pending).map_err(|e| format!("插件字段无效：{e}"))?;
                values.insert(current.clone(), value);
            }
            break;
        }
        hash.update(len);
        hash.update(&raw);
        seq += 1;
        if frame.kind == "header" {
            if !current.is_empty() {
                return Err("插件头帧顺序无效".into());
            }
            header.extend(frame.data);
        } else {
            if frame.kind != current {
                if values.contains_key(&frame.kind) {
                    return Err("插件字段帧不连续".into());
                }
                if !current.is_empty() {
                    let value = serde_json::from_slice(&pending)
                        .map_err(|e| format!("插件字段无效：{e}"))?;
                    values.insert(current.clone(), value);
                    pending.clear();
                }
                current = frame.kind.clone();
                order.push(current.clone());
            }
            pending.extend(frame.data);
        }
    }
    let mut h: Header = serde_json::from_slice(&header).map_err(|e| format!("插件头无效：{e}"))?;
    let expected = paths(&h.root);
    if h.fields.len() != expected.len()
        || h.fields
            .iter()
            .map(|(p, _)| p.as_str())
            .ne(expected.iter().map(String::as_str))
    {
        return Err("插件字段声明不匹配".into());
    }
    if order
        .iter()
        .map(String::as_str)
        .ne(h.fields.iter().map(|(p, _)| p.as_str()))
    {
        return Err("插件字段帧顺序不匹配".into());
    }
    for (path, count) in h.fields {
        let value = values.remove(&path).ok_or("插件字段缺失")?;
        if value.as_array().is_none_or(|a| a.len() != count) {
            return Err("插件字段数量不匹配".into());
        }
        *field_mut(&mut h.root, &path).ok_or("插件字段路径无效")? = value;
    }
    if !values.is_empty() {
        return Err("插件出现未声明字段".into());
    }
    serde_json::from_value(h.root).map_err(|e| format!("插件消息无效：{e}"))
}
