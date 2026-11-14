//! 同一资源快照中的字库只解析一次；缓存完整结果，错误和取消不发布。
use std::{
    collections::{BTreeMap, VecDeque},
    io::{Cursor, Read},
    sync::{Arc, Mutex, Weak},
};

#[derive(Debug, Clone)]
pub(crate) struct Glyph {
    pub width: u8,
    pub rows: [u32; 16],
}
pub(crate) type Index = BTreeMap<u32, Glyph>;
pub(crate) fn bytes(index: &Index) -> Result<usize, String> {
    index
        .len()
        .checked_mul(std::mem::size_of::<Glyph>() + 64)
        .ok_or_else(|| "unihex 索引溢出".into())
}
#[derive(Debug)]
struct Entry {
    source: Weak<Vec<u8>>,
    plain: bool,
    index: Arc<Index>,
    bytes: usize,
}
#[derive(Default, Debug)]
struct Entries {
    values: VecDeque<Entry>,
    bytes: usize,
}
#[derive(Default, Debug)]
pub(crate) struct Cache(Mutex<Entries>);
impl Cache {
    pub fn load(
        &self,
        file: &str,
        binary: &Arc<Vec<u8>>,
        reserve: impl Fn(usize) -> Result<(), String>,
        stop: &dyn Fn() -> bool,
    ) -> Result<Arc<Index>, String> {
        if stop() {
            return Err("字体读取已取消".into());
        }
        let mut cache = self.0.lock().expect("字库缓存锁异常");
        cache.values.retain(|e| e.source.strong_count() > 0);
        cache.bytes = cache.values.iter().map(|e| e.bytes).sum();
        let source = Arc::downgrade(binary);
        let plain = file.ends_with(".hex");
        if let Some(at) = cache
            .values
            .iter()
            .position(|e| e.source.ptr_eq(&source) && e.plain == plain)
        {
            let entry = cache.values.remove(at).expect("字库缓存索引失配");
            let index = Arc::clone(&entry.index);
            let bytes = entry.bytes;
            cache.values.push_back(entry);
            reserve(bytes)?;
            return Ok(index);
        }
        let index = parse(file, binary, &reserve, stop)?;
        let bytes = bytes(&index)?
            .checked_add(std::mem::size_of::<Entry>() + std::mem::size_of::<Index>())
            .ok_or("字库缓存计量溢出")?;
        if stop() {
            return Err("字体读取已取消".into());
        }
        const LIMIT: usize = 32 * 1024 * 1024;
        if bytes <= LIMIT {
            while !cache.values.is_empty() && cache.bytes + bytes > LIMIT {
                let old = cache.values.pop_front().expect("字库缓存队列失配");
                cache.bytes -= old.bytes;
            }
            cache.bytes += bytes;
            cache.values.push_back(Entry {
                source,
                plain,
                index: Arc::clone(&index),
                bytes,
            });
        }
        Ok(index)
    }
}

fn parse(
    file: &str,
    binary: &[u8],
    reserve: impl Fn(usize) -> Result<(), String>,
    stop: &dyn Fn() -> bool,
) -> Result<Arc<Index>, String> {
    let mut entries = BTreeMap::new();
    let mut parse = |bytes: &[u8]| -> Result<(), String> {
        let source = std::str::from_utf8(bytes).map_err(|e| format!("unihex 不是 UTF-8：{e}"))?;
        for (line, row) in source.lines().enumerate() {
            if line % 256 == 0 && stop() {
                return Err("字体读取已取消".into());
            }
            if line % 128 == 0 {
                reserve(
                    entries
                        .len()
                        .checked_add(128)
                        .and_then(|n| n.checked_mul(std::mem::size_of::<Glyph>() + 64))
                        .ok_or("unihex 索引大小溢出")?,
                )?;
            }
            if row.is_empty() {
                continue;
            }
            let (code, bits) = row
                .split_once(':')
                .ok_or_else(|| format!("unihex 第 {} 行缺少分隔符", line + 1))?;
            if !matches!(code.len(), 4..=6) || !matches!(bits.len(), 32 | 64 | 96 | 128) {
                return Err(format!("unihex 第 {} 行码点或字形尺寸错误", line + 1));
            }
            if !code.bytes().all(|b| b.is_ascii_hexdigit())
                || !bits.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(format!("unihex 第 {} 行包含非十六进制字符", line + 1));
            }
            let ch = u32::from_str_radix(code, 16)
                .ok()
                .filter(|code| *code <= 0x10ffff)
                .ok_or("unihex 码点无效")?;
            let digits = bits.len() / 16;
            let mut rows = [0; 16];
            for (y, row) in bits.as_bytes().chunks_exact(digits).enumerate() {
                let row = std::str::from_utf8(row).map_err(|e| e.to_string())?;
                rows[y] =
                    u32::from_str_radix(row, 16).map_err(|e| format!("unihex 像素无效：{e}"))?;
            }
            entries.insert(
                ch,
                Glyph {
                    width: (digits * 4) as u8,
                    rows,
                },
            );
        }
        Ok(())
    };
    if file.ends_with(".hex") {
        parse(binary)?;
    } else {
        let mut archive = zip::ZipArchive::new(Cursor::new(binary))
            .map_err(|e| format!("unihex ZIP 无效：{e}"))?;
        for i in 0..archive.len() {
            if stop() {
                return Err("字体读取已取消".into());
            }
            let mut member = archive.by_index(i).map_err(|e| e.to_string())?;
            if !member.name().ends_with(".hex") {
                continue;
            }
            let length = usize::try_from(member.size()).map_err(|_| "unihex ZIP 成员过大")?;
            reserve(length)?;
            let mut raw = Vec::new();
            raw.try_reserve_exact(length).map_err(|e| e.to_string())?;
            member
                .by_ref()
                .take(length as u64 + 1)
                .read_to_end(&mut raw)
                .map_err(|e| e.to_string())?;
            if raw.len() != length {
                return Err("unihex ZIP 声明尺寸不匹配".into());
            }
            parse(&raw)?;
        }
    }
    let cost = entries
        .len()
        .checked_mul(std::mem::size_of::<Glyph>() + 64)
        .ok_or("unihex 索引溢出")?;
    reserve(cost)?;

    Ok(Arc::new(entries))
}
