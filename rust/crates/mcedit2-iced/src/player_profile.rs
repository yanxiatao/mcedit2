//! 玩家列表与皮肤加载共用真实公共资料，姓名不依赖是否具有自定义皮肤。
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::Mutex,
    time::Duration,
};

#[derive(Clone, Debug)]
pub(crate) struct Profile {
    pub name: String,
    pub textures: Option<String>,
    pub cached: bool,
}

pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(10)))
            .build(),
    )
}

pub(crate) fn read_cache(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取玩家资料缓存元数据失败：{error}")),
    };
    if !metadata.is_file() {
        return Err("玩家资料缓存不是普通文件".into());
    }
    if metadata
        .modified()
        .map_err(|error| format!("读取玩家资料缓存时间失败：{error}"))?
        .elapsed()
        .map_err(|error| format!("玩家资料缓存时间无效：{error}"))?
        >= Duration::from_secs(24 * 60 * 60)
    {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|error| format!("打开玩家资料缓存失败：{error}"))?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取玩家资料缓存失败：{error}"))?;
    if bytes.len() > limit {
        return Err(format!("玩家资料缓存超过 {limit} 字节"));
    }
    Ok(Some(bytes))
}

pub(crate) fn write_cache(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::create_dir_all(path.parent().expect("玩家缓存具有父目录"))
        .map_err(|error| format!("创建玩家资料缓存目录失败：{error}"))?;
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).map_err(|error| error.to_string())?;
    let nonce = nonce
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let temporary = path.with_extension(format!("{nonce}.part"));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("创建玩家资料临时缓存失败：{error}"))?;
    let result = (|| -> Result<(), std::io::Error> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        if let Err(error) = fs::remove_file(&temporary) {
            log::error!("[player-profile] 清理本次临时缓存失败：{error}");
        }
    }
    result.map_err(|error| format!("保存玩家资料缓存失败：{error}"))
}

pub(crate) fn load(uuid: [u8; 16]) -> Result<Profile, String> {
    load_with(uuid, &agent())
}

/// 只读取有效的本地公共资料；缺失或按规则过期时不发出网络请求。
pub(crate) fn read_cached(uuid: [u8; 16]) -> Result<Option<Profile>, String> {
    let id = uuid
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let path = crate::player_skin::cache_dir()?.join(format!("{id}-profile.json"));
    read_cache(&path, 2 * 1024 * 1024)?
        .map(|bytes| decode(&id, &bytes, true))
        .transpose()
}

pub(crate) fn load_with(uuid: [u8; 16], agent: &ureq::Agent) -> Result<Profile, String> {
    // 同进程的列表与场景请求共享查询；磁盘使用完整文件替换交接给其他编辑器进程。
    static QUERY: Mutex<()> = Mutex::new(());
    let _query = QUERY.lock().expect("玩家资料查询锁被毒化");
    let id = uuid
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let path = crate::player_skin::cache_dir()?.join(format!("{id}-profile.json"));
    let (bytes, cached) = match read_cache(&path, 2 * 1024 * 1024)? {
        Some(bytes) => (bytes, true),
        None => (
            crate::player_skin::read_bytes(
                agent,
                &format!("https://sessionserver.mojang.com/session/minecraft/profile/{id}"),
            )?,
            false,
        ),
    };
    let profile = decode(&id, &bytes, cached)?;
    if !cached {
        write_cache(&path, &bytes)?;
    }
    Ok(profile)
}

fn decode(id: &str, bytes: &[u8], cached: bool) -> Result<Profile, String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| format!("玩家资料不是有效 JSON：{error}"))?;
    if value.get("id").and_then(|value| value.as_str()) != Some(id) {
        return Err("玩家资料 UUID 不匹配".into());
    }
    let name = value
        .get("name")
        .and_then(|value| value.as_str())
        .ok_or("玩家资料缺少姓名")?
        .to_owned();
    let properties = value
        .get("properties")
        .and_then(|value| value.as_array())
        .ok_or("玩家资料缺少属性列表")?;
    let mut textures = None;
    for property in properties {
        if property.get("name").and_then(|value| value.as_str()) == Some("textures") {
            if textures.is_some() {
                return Err("玩家资料的纹理属性重复".into());
            }
            textures = Some(
                property
                    .get("value")
                    .and_then(|value| value.as_str())
                    .ok_or("玩家纹理属性缺少实际值")?
                    .to_owned(),
            );
        }
    }
    Ok(Profile {
        name,
        textures,
        cached,
    })
}
