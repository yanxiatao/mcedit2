//! 按设备容量和填充成本分组的真正图集数组；原页 UV 与独立材质 mip 保持不变。
use super::{make_binding, make_texture, GpuMip, TextureAtlas};
use iced::wgpu;
use std::sync::{Mutex, OnceLock, Weak};
use std::{ops::Range, sync::Arc};

/// 跨视口共享只读纹理；每个流水线仍使用自己的相机、采样器和绑定。
pub(super) struct SharedGroup {
    // 保活逻辑设备，保证后端设备地址在租约期间不会被重新分配。
    _device: wgpu::Device,
    identity: DeviceIdentity,
    first: usize,
    animation: Weak<crate::textures::atlas_animation::Set>,
    source: Vec<(
        Weak<crate::textures::AtlasPage>,
        Weak<crate::material_mips::MipPage>,
    )>,
    base: wgpu::Texture,
    mips: Vec<GpuMip>,
    animation_stamps: Mutex<
        std::collections::BTreeMap<(u32, usize), crate::textures::atlas_animation::SampleStamp>,
    >,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct DeviceIdentity {
    backend: wgpu::Backend,
    address: usize,
}
fn backend_identity<A: wgpu::hal::Api>(
    device: &wgpu::Device,
    backend: wgpu::Backend,
) -> Option<DeviceIdentity> {
    // SAFETY: 只在有效 guard 内读取后端设备的地址，不调用后端方法、不解引用缓存地址，
    // 不销毁或修改返回资源。SharedGroup 保留 Device 克隆以覆盖地址作为键的整个寿命。
    unsafe {
        device.as_hal::<A>().map(|raw| DeviceIdentity {
            backend,
            address: std::ptr::from_ref(&*raw) as usize,
        })
    }
}
fn device_identity(device: &wgpu::Device) -> Result<DeviceIdentity, String> {
    #[cfg(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd"
    ))]
    if let Some(key) = backend_identity::<wgpu::hal::api::Vulkan>(device, wgpu::Backend::Vulkan) {
        return Ok(key);
    }
    #[cfg(target_os = "windows")]
    if let Some(key) = backend_identity::<wgpu::hal::api::Dx12>(device, wgpu::Backend::Dx12) {
        return Ok(key);
    }
    #[cfg(target_vendor = "apple")]
    if let Some(key) = backend_identity::<wgpu::hal::api::Metal>(device, wgpu::Backend::Metal) {
        return Ok(key);
    }
    #[cfg(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd"
    ))]
    if let Some(key) = backend_identity::<wgpu::hal::api::Gles>(device, wgpu::Backend::Gl) {
        return Ok(key);
    }
    Err("图集共享无法取得当前原生后端的设备身份".into())
}
fn shared_groups() -> &'static Mutex<Vec<Weak<SharedGroup>>> {
    static CACHE: OnceLock<Mutex<Vec<Weak<SharedGroup>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}
fn shared(
    atlas: &TextureAtlas,
    group: &Group,
    device: &wgpu::Device,
) -> Result<Option<Arc<SharedGroup>>, String> {
    let identity = device_identity(device)?;
    let mut cache = shared_groups().lock().expect("共享图集资源锁被毒化");
    cache.retain(|value| value.strong_count() != 0);
    Ok(cache.iter().filter_map(Weak::upgrade).find(|entry| {
        entry.identity == identity
            && entry.animation.ptr_eq(
                &atlas
                    .animation
                    .as_ref()
                    .map(Arc::downgrade)
                    .unwrap_or_default(),
            )
            && entry.first == group.pages.start
            && entry.source.len() == group.pages.len()
            && entry
                .source
                .iter()
                .zip(group.pages.clone())
                .all(|((base, mip), i)| {
                    base.ptr_eq(&Arc::downgrade(&atlas.pages[i]))
                        && mip.ptr_eq(&Arc::downgrade(&atlas.mip_pages[i]))
                })
    }))
}
pub(super) fn publish(
    atlas: &TextureAtlas,
    group: &Group,
    device: &wgpu::Device,
    pages: &mut [Page],
) -> Result<(), String> {
    if pages.first().is_none_or(|p| p.2.shared.is_some()) {
        return Ok(());
    }
    let lease = Arc::new(SharedGroup {
        _device: device.clone(),
        identity: device_identity(device)?,
        first: group.pages.start,
        animation: atlas
            .animation
            .as_ref()
            .map(Arc::downgrade)
            .unwrap_or_default(),
        source: group
            .pages
            .clone()
            .map(|i| {
                (
                    Arc::downgrade(&atlas.pages[i]),
                    Arc::downgrade(&atlas.mip_pages[i]),
                )
            })
            .collect(),
        base: pages[0].0.clone(),
        mips: pages
            .iter()
            .map(|p| {
                let mut m = p.2.clone();
                m.shared = None;
                m
            })
            .collect(),
        animation_stamps: Mutex::new(std::collections::BTreeMap::new()),
    });
    shared_groups()
        .lock()
        .expect("共享图集发布锁被毒化")
        .push(Arc::downgrade(&lease));
    for page in pages {
        page.2.shared = Some(Arc::clone(&lease));
    }
    Ok(())
}

pub(super) type Page = (wgpu::Texture, wgpu::BindGroup, GpuMip);
const MATERIAL_SIZE: u64 = std::mem::size_of::<crate::material_mips::GpuMaterial>() as u64;

fn rectangle(uv: [f32; 4], width: u32, height: u32) -> [u32; 4] {
    let x = (uv[0] * width as f32 - 0.5).round() as u32;
    let y = (uv[1] * height as f32 - 0.5).round() as u32;
    let w = ((uv[2] - uv[0]) * width as f32 + 1.).round() as u32;
    let h = ((uv[3] - uv[1]) * height as f32 + 1.).round() as u32;
    [x - 1, y - 1, w + 2, h + 2]
}
fn patch(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layer: u32,
    target: [u32; 4],
    data: &[u8],
    width: u32,
    source: [u32; 4],
) -> Result<(), String> {
    if target[2..] != source[2..] {
        return Err("动画 mip 的源和目标槽尺寸不同".into());
    }
    let offset = ((source[1] as usize) * width as usize + source[0] as usize) * 4;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: target[0],
                y: target[1],
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &data[offset..],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(source[3]),
        },
        wgpu::Extent3d {
            width: target[2],
            height: target[3],
            depth_or_array_layers: 1,
        },
    );
    Ok(())
}
pub(super) type AnimatedMaterials = std::collections::BTreeSet<(u32, usize)>;

/// 从原始 UV 找到实际使用的动画槽。跨槽模型保留所有相交材质。
pub(super) fn animated_materials<const N: usize>(
    atlas: Option<&TextureAtlas>,
    faces: impl IntoIterator<Item = (u32, [[f32; 2]; N])>,
    cancelled: &dyn Fn() -> bool,
) -> Result<AnimatedMaterials, String> {
    let Some(atlas) = atlas else {
        return Ok(AnimatedMaterials::new());
    };
    let Some(animation) = &atlas.animation else {
        return Ok(AnimatedMaterials::new());
    };
    let mut seen = std::collections::HashSet::new();
    let mut used = AnimatedMaterials::new();
    let animated_pages: std::collections::BTreeSet<_> =
        animation.bindings.keys().map(|key| key.0).collect();
    for (index, (page, uv)) in faces.into_iter().enumerate() {
        if index.is_multiple_of(1024) && cancelled() {
            return Ok(used);
        }
        if !animated_pages.contains(&page) {
            continue;
        }
        let bounds = std::array::from_fn::<_, 4, _>(|i| {
            let values = uv.iter().map(|corner| corner[i % 2].clamp(0., 1.));
            if i < 2 {
                values.fold(1., f32::min)
            } else {
                values.fold(0., f32::max)
            }
        });
        if !seen.insert((page, bounds.map(f32::to_bits))) {
            continue;
        }
        let mip = atlas.mip_page(page)?;
        if let Some(owner) = mip.owner_for_bounds(bounds) {
            let key = (page, owner as usize - 1);
            if animation.bindings.contains_key(&key) {
                used.insert(key);
            }
            continue;
        }
        let margin = [1. / mip.base.width as f32, 1. / mip.base.height as f32];
        for (&key, binding) in animation.bindings.range((page, 0)..=(page, usize::MAX)) {
            if (0..2).all(|axis| {
                bounds[axis] <= binding.uv.end[axis] + margin[axis]
                    && bounds[axis + 2] >= binding.uv.start[axis] - margin[axis]
            }) {
                used.insert(key);
            }
        }
    }
    Ok(used)
}

/// 同设备的多个视口共用已更新的素材；每个流水线独立保存实际显示的帧。
pub(super) fn animate(
    atlas: &TextureAtlas,
    pages: &[Page],
    queue: &wgpu::Queue,
    at: Option<std::time::Instant>,
    used: &AnimatedMaterials,
    previous: Option<&Arc<crate::textures::atlas_animation::Frame>>,
) -> Result<Option<Arc<crate::textures::atlas_animation::Frame>>, String> {
    let Some(set) = &atlas.animation else {
        return Ok(None);
    };
    let tick = atlas.animation_frame.as_ref().map_or_else(
        || set.tick_at(at.unwrap_or_else(std::time::Instant::now)),
        |frame| frame.tick,
    );
    let _span = iced_wgpu::window::frame_statistics::span("world_atlas_animation");
    iced_wgpu::window::frame_statistics::counter("animated_materials", used.len() as u64);
    let frame = match (&atlas.animation_frame, previous) {
        (Some(frame), _) => Arc::new(crate::textures::atlas_animation::Frame {
            tick,
            samples: used
                .iter()
                .filter_map(|key| {
                    frame
                        .samples
                        .get(key)
                        .map(|sample| (*key, Arc::clone(sample)))
                })
                .collect(),
        }),
        (None, Some(frame)) if frame.tick == tick && frame.samples.keys().eq(used.iter()) => {
            Arc::clone(frame)
        }
        (None, previous) => set.frame_for(tick, used.iter().copied(), previous.map(Arc::as_ref))?,
    };
    for (&key, sample) in &frame.samples {
        let gpu = &pages[key.0 as usize];
        let lease = gpu.2.shared.as_ref().ok_or("动画图集尚未发布共享资源")?;
        let mut stamps = lease.animation_stamps.lock().expect("GPU 动画状态锁被毒化");
        let mip = atlas.mip_page(key.0)?;
        if stamps.get(&key) == Some(&sample.stamp) {
            continue;
        }
        let old = mip.materials[key.1];
        let local = &sample.page.mip;
        let current = local.materials[0];
        if old.count != current.count {
            return Err("动画帧改变了材质 mip 布局".into());
        }
        patch(
            queue,
            &gpu.0,
            gpu.2.layer,
            rectangle(old.base_uv, mip.base.width, mip.base.height),
            &local.base.rgba,
            local.base.width,
            rectangle(current.base_uv, local.base.width, local.base.height),
        )?;
        for level in 0..old.count as usize {
            patch(
                queue,
                &gpu.2.aux,
                gpu.2.layer,
                rectangle(old.levels[level], mip.width, mip.height),
                &local.rgba,
                local.width,
                rectangle(current.levels[level], local.width, local.height),
            )?;
        }
        let updated = crate::material_mips::GpuMaterial {
            coverage: current.coverage,
            ..old
        };
        queue.write_buffer(
            &gpu.2.materials,
            gpu.2.material_offset + key.1 as u64 * MATERIAL_SIZE,
            bytemuck::bytes_of(&updated),
        );
        stamps.insert(key, sample.stamp.clone());
    }
    Ok(Some(frame))
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PageInfo {
    // 原图 UV 缩放及原图像素尺寸，供正确计算屏幕导数。
    base: [f32; 4],
    // 独立辅助图 UV 缩放；层间尺寸无需相同。
    aux: [f32; 4],
    owner: [u32; 4],     // 原 owner 宽、高、缩放、实际数组层
    materials: [u32; 4], // 共享表中的起始材质、当前页材质数、保留
}

pub(super) struct Group {
    pub pages: Range<usize>,
    base: [u32; 2],
    aux: [u32; 2],
    owner: [u32; 2],
    material_bytes: u64,
    source_pixels: u64,
}
fn pixel_count(size: [u32; 2]) -> u64 {
    u64::from(size[0]) * u64::from(size[1])
}
fn texture(
    device: &wgpu::Device,
    size: [u32; 2],
    layers: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}
fn page_table(device: &wgpu::Device, start: u32, entries: &[PageInfo]) -> wgpu::Buffer {
    let bits = super::material_hints::layer_bits(entries.len());
    let mask = if bits == 32 {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    };
    let header = [start, entries.len() as u32, bits, mask];
    let bytes = bytemuck::cast_slice(entries);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("MCEdit2 图集数组原页映射"),
        size: 16 + bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: true,
    });
    {
        let mut target = buffer.slice(..).get_mapped_range_mut();
        target[..16].copy_from_slice(bytemuck::cast_slice(&header));
        target[16..].copy_from_slice(bytes);
    }
    buffer.unmap();
    buffer
}
pub(super) fn fallback(device: &wgpu::Device) -> GpuMip {
    GpuMip {
        shared: None,
        binding_page: 0,
        layer: 0,
        group_layers: 1,
        material_offset: 0,
        aux: texture(
            device,
            [1, 1],
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            "MCEdit2 默认 mip 数组",
        ),
        owner: texture(
            device,
            [1, 1],
            1,
            wgpu::TextureFormat::R32Uint,
            "MCEdit2 默认 owner 数组",
        ),
        materials: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MCEdit2 默认材质元数据"),
            size: MATERIAL_SIZE,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        }),
        page_table: page_table(
            device,
            0,
            &[PageInfo {
                base: [1.; 4],
                aux: [1.; 4],
                owner: [1, 1, 1, 0],
                materials: [0; 4],
            }],
        ),
    }
}
/// 组内原页连续，最大填充开销为实际三种图片像素总量的 25%；超限只拆组。
pub(super) fn plan(
    atlas: &TextureAtlas,
    start: usize,
    device: &wgpu::Device,
) -> Result<Group, String> {
    let limits = device.limits();
    let mut group = Group {
        pages: start..start,
        base: [0; 2],
        aux: [0; 2],
        owner: [0; 2],
        material_bytes: 0,
        source_pixels: 0,
    };
    for index in start..atlas.pages.len() {
        let base = &atlas.pages[index];
        let mip = atlas.mip_page(u32::try_from(index).map_err(|_| "图集原页索引超出 u32")?)?;
        let sizes = [
            [base.width, base.height],
            [mip.width, mip.height],
            [mip.owner_width, mip.owner_height],
        ];
        if sizes
            .iter()
            .flatten()
            .any(|&n| n == 0 || n > limits.max_texture_dimension_2d)
            || base.rgba.len() as u64 != pixel_count(sizes[0]) * 4
            || mip.rgba.len() as u64 != pixel_count(sizes[1]) * 4
            || mip.owner.len() as u64 != pixel_count(sizes[2])
            || !matches!(mip.owner_scale, 1 | 2)
            || mip.owner_width != base.width.div_ceil(mip.owner_scale)
            || mip.owner_height != base.height.div_ceil(mip.owner_scale)
            || !Arc::ptr_eq(&mip.base, base)
        {
            return Err(format!("图集原页 {index} 的尺寸、mip 来源或像素长度无效"));
        }
        let own_bytes = (mip.materials.len() as u64)
            .checked_mul(MATERIAL_SIZE)
            .ok_or("材质表大小溢出")?;
        let max_buffer =
            u64::from(limits.max_storage_buffer_binding_size).min(limits.max_buffer_size);
        if own_bytes.max(MATERIAL_SIZE) > max_buffer {
            return Err(format!(
                "单页 {index} 的材质表超出设备 storage binding 容量 {max_buffer} 字节"
            ));
        }
        let layers = index - start + 1;
        let base_size = std::array::from_fn(|i| group.base[i].max(sizes[0][i]));
        let aux_size = std::array::from_fn(|i| group.aux[i].max(sizes[1][i]));
        let owner_size = std::array::from_fn(|i| group.owner[i].max(sizes[2][i]));
        let material_bytes = group
            .material_bytes
            .checked_add(own_bytes)
            .ok_or("材质组大小溢出")?;
        let source_pixels = group.source_pixels + sizes.into_iter().map(pixel_count).sum::<u64>();
        let padded_pixels = layers as u64
            * [base_size, aux_size, owner_size]
                .into_iter()
                .map(pixel_count)
                .sum::<u64>();
        let fits = layers as u64 <= u64::from(limits.max_texture_array_layers)
            && material_bytes.max(MATERIAL_SIZE) <= max_buffer
            && 16 + layers as u64 * std::mem::size_of::<PageInfo>() as u64 <= max_buffer
            && padded_pixels * 4 <= source_pixels * 5;
        if !fits {
            if index == start {
                return Err(format!("设备不能容纳图集原页 {index} 的数组绑定"));
            }
            break;
        }
        group = Group {
            pages: start..index + 1,
            base: base_size,
            aux: aux_size,
            owner: owner_size,
            material_bytes,
            source_pixels,
        };
    }
    if group.pages.is_empty() {
        return Err("图集数组分组没有源页".into());
    }
    Ok(group)
}
/// 只复用完整且所有源页身份均相同的组，绝不修改正在显示的旧数组。
pub(super) fn prepare(
    atlas: &TextureAtlas,
    group: &Group,
    previous: Option<&TextureAtlas>,
    old: &[Page],
    reuse_binding: bool,
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    camera: &wgpu::Buffer,
    lightmap: &wgpu::Buffer,
) -> Result<(Vec<Page>, bool), String> {
    let count = group.pages.len() as u32;
    let first = group.pages.start;
    let same = old
        .get(first)
        .is_some_and(|gpu| gpu.2.binding_page == first && gpu.2.group_layers == count)
        && group.pages.clone().all(|i| {
            let Some(previous) = previous else {
                return false;
            };
            old.get(i)
                .is_some_and(|gpu| gpu.2.binding_page == first && gpu.2.layer == (i - first) as u32)
                && previous
                    .pages
                    .get(i)
                    .is_some_and(|p| Arc::ptr_eq(p, &atlas.pages[i]))
                && previous
                    .mip_pages
                    .get(i)
                    .zip(atlas.mip_pages.get(i))
                    .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
        });
    if same {
        let first_gpu = &old[first];
        let binding = if reuse_binding {
            first_gpu.1.clone()
        } else {
            make_binding(
                device,
                layout,
                &first_gpu.0,
                sampler,
                camera,
                lightmap,
                &first_gpu.2,
            )
        };
        return Ok((
            group
                .pages
                .clone()
                .map(|i| (old[i].0.clone(), binding.clone(), old[i].2.clone()))
                .collect(),
            false,
        ));
    }
    if let Some(shared) = shared(atlas, group, device)? {
        let binding = make_binding(
            device,
            layout,
            &shared.base,
            sampler,
            camera,
            lightmap,
            &shared.mips[0],
        );
        return Ok((
            shared
                .mips
                .iter()
                .map(|mip| {
                    let mut mip = mip.clone();
                    mip.shared = Some(Arc::clone(&shared));
                    (shared.base.clone(), binding.clone(), mip)
                })
                .collect(),
            false,
        ));
    }
    let base = make_texture(device, group.base[0], group.base[1], count);
    let aux = texture(
        device,
        group.aux,
        count,
        wgpu::TextureFormat::Rgba8Unorm,
        "MCEdit2 独立材质 mip 数组",
    );
    let owner = texture(
        device,
        group.owner,
        count,
        wgpu::TextureFormat::R32Uint,
        "MCEdit2 材质 owner 数组",
    );
    let materials = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("MCEdit2 图集组材质元数据"),
        size: group.material_bytes.max(MATERIAL_SIZE),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut metadata = Vec::with_capacity(group.pages.len());
    let mut offset = 0_u32;
    for (layer, index) in group.pages.clone().enumerate() {
        let page = &atlas.pages[index];
        let mip = atlas.mip_page(index as u32)?;
        let count = u32::try_from(mip.materials.len()).map_err(|_| "单页材质数溢出")?;
        metadata.push(PageInfo {
            base: [
                page.width as f32 / group.base[0] as f32,
                page.height as f32 / group.base[1] as f32,
                page.width as f32,
                page.height as f32,
            ],
            aux: [
                mip.width as f32 / group.aux[0] as f32,
                mip.height as f32 / group.aux[1] as f32,
                mip.width as f32,
                mip.height as f32,
            ],
            owner: [
                mip.owner_width,
                mip.owner_height,
                mip.owner_scale,
                layer as u32,
            ],
            materials: [offset, count, 0, 0],
        });
        offset = offset.checked_add(count).ok_or("图集组材质索引溢出")?;
    }
    let page_table = page_table(
        device,
        u32::try_from(first).map_err(|_| "图集组索引溢出")?,
        &metadata,
    );
    let mip = GpuMip {
        shared: None,
        binding_page: first,
        layer: 0,
        group_layers: count,
        material_offset: 0,
        aux,
        owner,
        materials,
        page_table,
    };
    let binding = make_binding(device, layout, &base, sampler, camera, lightmap, &mip);
    Ok((
        metadata
            .iter()
            .map(|entry| {
                let mut local = mip.clone();
                local.layer = entry.owner[3];
                local.material_offset = u64::from(entry.materials[0]) * MATERIAL_SIZE;
                (base.clone(), binding.clone(), local)
            })
            .collect(),
        true,
    ))
}
pub(super) fn upload_full(
    atlas: &TextureAtlas,
    previous: Option<&TextureAtlas>,
    old: &[Page],
    reuse_binding: bool,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    camera: &wgpu::Buffer,
    lightmap: &wgpu::Buffer,
) -> Result<Vec<Page>, String> {
    let mut result = Vec::with_capacity(atlas.pages.len());
    while result.len() < atlas.pages.len() {
        let group = plan(atlas, result.len(), device)?;
        let (mut pages, changed) = prepare(
            atlas,
            &group,
            previous,
            old,
            reuse_binding,
            device,
            layout,
            sampler,
            camera,
            lightmap,
        )?;
        if changed {
            for (index, gpu) in group.pages.clone().zip(&pages) {
                let page = &atlas.pages[index];
                let mip = atlas.mip_page(index as u32)?;
                for (texture, width, height, data) in [
                    (&gpu.0, page.width, page.height, page.rgba.as_slice()),
                    (&gpu.2.aux, mip.width, mip.height, mip.rgba.as_slice()),
                    (
                        &gpu.2.owner,
                        mip.owner_width,
                        mip.owner_height,
                        bytemuck::cast_slice(&mip.owner),
                    ),
                ] {
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: 0,
                                y: 0,
                                z: gpu.2.layer,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        data,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(width * 4),
                            rows_per_image: Some(height),
                        },
                        wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
                if !mip.materials.is_empty() {
                    queue.write_buffer(
                        &gpu.2.materials,
                        gpu.2.material_offset,
                        bytemuck::cast_slice(&mip.materials),
                    );
                }
            }
        }
        publish(atlas, &group, device, &mut pages)?;
        result.extend(pages);
    }
    Ok(result)
}
