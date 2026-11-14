//! Available memory of the current Linux process, including ancestor cgroups.
use std::path::{Path, PathBuf};

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}
fn unescape(path: &str) -> PathBuf {
    PathBuf::from(
        path.replace("\\040", " ")
            .replace("\\011", "\t")
            .replace("\\012", "\n")
            .replace("\\134", "\\"),
    )
}
fn remaining(path: &Path, v2: bool) -> Result<Option<u64>, String> {
    let (limit_name, used_name) = if v2 {
        ("memory.max", "memory.current")
    } else {
        ("memory.limit_in_bytes", "memory.usage_in_bytes")
    };
    let Some(limit) = read_optional(&path.join(limit_name))? else {
        return Ok(None);
    };
    if limit.trim() == "max" {
        return Ok(None);
    }
    let limit = limit.trim().parse::<u64>().map_err(|e| e.to_string())?;
    let used = std::fs::read_to_string(path.join(used_name))
        .map_err(|e| e.to_string())?
        .trim()
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    Ok(Some(limit.saturating_sub(used)))
}
fn hierarchy_limit(mount: &Path, group: &Path, v2: bool) -> Result<Option<u64>, String> {
    if !group.starts_with(mount) {
        return Err("cgroup path escapes its mount".into());
    }
    let mut available = None;
    for parent in group.ancestors() {
        if let Some(bytes) = remaining(parent, v2)? {
            available = Some(available.map_or(bytes, |old: u64| old.min(bytes)));
        }
        if parent == mount {
            break;
        }
    }
    Ok(available)
}
pub(super) fn available() -> Result<u64, String> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").map_err(|e| e.to_string())?;
    let mut available = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|s| s.split_whitespace().next()?.parse::<u64>().ok())
        .and_then(|kb| kb.checked_mul(1024))
        .ok_or("MemAvailable is missing or invalid")?;
    let memberships = std::fs::read_to_string("/proc/self/cgroup").map_err(|e| e.to_string())?;
    let mounts = std::fs::read_to_string("/proc/self/mountinfo").map_err(|e| e.to_string())?;
    for membership in memberships.lines() {
        let mut fields = membership.splitn(3, ':');
        let _id = fields.next();
        let controllers = fields.next().ok_or("invalid cgroup membership")?;
        let member = Path::new(fields.next().ok_or("missing cgroup path")?);
        let v2 = controllers.is_empty();
        if !v2 && !controllers.split(',').any(|s| s == "memory") {
            continue;
        }
        let mut resolved = false;
        for entry in mounts.lines() {
            let (left, right) = entry.split_once(" - ").ok_or("invalid mountinfo")?;
            let fields: Vec<_> = left.split_whitespace().collect();
            let fs: Vec<_> = right.split_whitespace().collect();
            if fields.len() < 5 || fs.len() < 3 {
                return Err("incomplete mountinfo".into());
            }
            if (v2 && fs[0] != "cgroup2")
                || (!v2 && (fs[0] != "cgroup" || !fs[2].split(',').any(|s| s == "memory")))
            {
                continue;
            }
            let mount_root = unescape(fields[3]);
            let mount = unescape(fields[4]);
            let relative = match member.strip_prefix(&mount_root) {
                Ok(relative) => relative,
                // A cgroup namespace reports paths relative to its own root.
                Err(_) if member == Path::new("/") => Path::new(""),
                Err(_) => continue,
            };
            if relative
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err("cgroup membership contains parent traversal".into());
            }
            resolved = true;
            if let Some(bytes) = hierarchy_limit(&mount, &mount.join(relative), v2)? {
                available = available.min(bytes);
            }
        }
        if !resolved {
            return Err("无法定位当前 cgroup 的内存控制器；请显式指定资源预算".into());
        }
    }
    Ok(available)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ancestor_limits_and_current_usage_bound_the_process() {
        let root = std::env::temp_dir().join(format!("mcedit-cgroup-{}", std::process::id()));
        let child = root.join("child");
        std::fs::create_dir_all(&child).unwrap();
        for (dir, limit, used) in [(&root, "1000", "850"), (&child, "max", "100")] {
            std::fs::write(dir.join("memory.max"), limit).unwrap();
            std::fs::write(dir.join("memory.current"), used).unwrap();
        }
        assert_eq!(hierarchy_limit(&root, &child, true).unwrap(), Some(150));
        std::fs::write(child.join("memory.max"), "200").unwrap();
        assert_eq!(hierarchy_limit(&root, &child, true).unwrap(), Some(100));
        std::fs::write(root.join("memory.current"), "1100").unwrap();
        assert_eq!(hierarchy_limit(&root, &child, true).unwrap(), Some(0));
        std::fs::remove_dir_all(root).unwrap();
    }
}
