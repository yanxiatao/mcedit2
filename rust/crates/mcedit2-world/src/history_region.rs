//! 旧 Region 内容历史的逐槽来源选择；文件版本来自真实归档，未选槽仍读取原基底。
use super::*;

pub(crate) type RegionSources = BTreeMap<PathBuf, BTreeMap<(i32, i32), Arc<FileVersion>>>;

impl Archive {
    /// 原生旧事务只改已有内部槽，选择其真实文件版本时不把同 Region 其他槽一起换源。
    pub(crate) fn with_region_sources(
        &self,
        sources: Arc<RegionSources>,
    ) -> Result<Self, WorldError> {
        if !self.1.versions.is_empty() {
            return Err(invalid("逐槽逻辑来源不能混入整文件物理版本视图"));
        }
        let mut checked = BTreeSet::new();
        for (path, slots) in sources.iter() {
            if slots.is_empty() || slots.len() > 1024 || !self.0.files.contains_key(path) {
                return Err(invalid("逐槽来源需要已登记的 Region 文件"));
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| invalid("逐槽来源文件名不是 UTF-8"))?;
            let parts = name.split('.').collect::<Vec<_>>();
            if parts.len() != 4 || parts[0] != "r" || !matches!(parts[3], "mca" | "mcr") {
                return Err(invalid("逐槽来源不是 MCA / MCR Region"));
            }
            let x = parts[1]
                .parse::<i32>()
                .map_err(|_| invalid("逐槽来源 Region X 无效"))?;
            let z = parts[2]
                .parse::<i32>()
                .map_err(|_| invalid("逐槽来源 Region Z 无效"))?;
            if name != format!("r.{x}.{z}.{}", parts[3]) {
                return Err(invalid("逐槽来源 Region 文件名不规范"));
            }
            for (chunk, version) in slots {
                if (chunk.0.div_euclid(32), chunk.1.div_euclid(32)) != (x, z)
                    || version.path() != path
                    || version.sha256().is_none()
                {
                    return Err(invalid("逐槽版本与 Region 归属或存在性不同"));
                }
                if checked.insert(version) {
                    self.version_view(std::iter::once(version.as_ref().clone()))?;
                }
            }
        }
        let mut projection = (*self.1).clone();
        projection.regions = sources;
        Ok(Self(Arc::clone(&self.0), Arc::new(projection)))
    }
    pub(crate) fn selected_region_version(
        &self,
        path: &Path,
        chunk: (i32, i32),
    ) -> Option<&FileVersion> {
        self.1.regions.get(path)?.get(&chunk).map(Arc::as_ref)
    }
    pub(super) fn project_region(
        &self,
        path: &Path,
        mut image: Option<Vec<u8>>,
        limit: usize,
        mut reserve: impl FnMut(usize) -> Result<(), WorldError>,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        let Some(sources) = self.1.regions.get(path) else {
            return Ok(image);
        };
        if let Some(bytes) = image.as_ref().filter(|bytes| !bytes.is_empty()) {
            crate::region_plan::parse_slots(bytes)?;
        }
        let mut versions = BTreeMap::<&FileVersion, Vec<(i32, i32)>>::new();
        for (chunk, version) in sources {
            versions.entry(version.as_ref()).or_default().push(*chunk);
        }
        for (version, chunks) in versions {
            let source = self
                .read_version_reserved(version, limit, &mut reserve)?
                .ok_or_else(|| invalid("逐槽历史版本的 Region 文件缺失"))?;
            let slots = crate::region_plan::parse_slots(&source)?;
            if image.as_ref().is_none_or(Vec::is_empty) {
                // 已选版本证明文件及所选槽存在；原逻辑基底缺席的其他槽仍明确为空。
                reserve(8192)?;
                let mut header = source[..8192].to_vec();
                header.fill(0);
                image = Some(header);
            }
            let image = image.as_mut().expect("逐槽来源已有明确 Region 容器");
            for chunk in chunks {
                let index = (chunk.1.rem_euclid(32) * 32 + chunk.0.rem_euclid(32)) as usize;
                let head = index * 4;
                match slots[index] {
                    Some(slot) => {
                        if crate::region_external::is_external(&source, slot)?.is_some() {
                            return Err(invalid(
                                "旧 Region 逐槽内容切换不承载外部 MCC，须使用独立版本证明",
                            ));
                        }
                        let start = slot.start * 4096;
                        let bytes = &source[start..start + slot.sectors * 4096];
                        let current = u32::from_be_bytes(
                            image[head..head + 4].try_into().expect("Region 头已核对"),
                        );
                        let current_start = (current >> 8) as usize * 4096;
                        let same = current != 0
                            && (current & 255) as usize == slot.sectors
                            && image[current_start..current_start + bytes.len()] == *bytes;
                        if !same {
                            let sector = image.len() / 4096;
                            let length = image
                                .len()
                                .checked_add(bytes.len())
                                .filter(|length| *length <= limit)
                                .ok_or_else(|| invalid("逐槽历史视图超过单文件预算"))?;
                            if sector > 0x00ff_ffff {
                                return Err(invalid("逐槽历史视图超过 Region 扇区地址范围"));
                            }
                            reserve(bytes.len())?;
                            image.reserve(length - image.len());
                            image.extend_from_slice(bytes);
                            let location = ((sector as u32) << 8) | slot.sectors as u32;
                            image[head..head + 4].copy_from_slice(&location.to_be_bytes());
                        }
                    }
                    None => image[head..head + 4].fill(0),
                }
                image[4096 + head..4100 + head].copy_from_slice(&source[4096 + head..4100 + head]);
            }
        }
        Ok(image)
    }
    /// 目录只暴露明确选中文件及其父目录，未选文件继续按原始目录视图枚举。
    pub(super) fn region_directory_view(&self) -> Result<VersionView<'_>, WorldError> {
        self.version_view(self.1.regions.values().map(|slots| {
            slots
                .values()
                .next()
                .expect("逐槽来源已经拒绝空范围")
                .as_ref()
                .clone()
        }))
    }
}
