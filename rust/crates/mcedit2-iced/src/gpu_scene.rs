//! Native iced 0.14 wgpu shader renderer of the source-voxel mesh.
//! Projected face geometry and hit tests stay shared with the Canvas path.
//! Shader avoids an application build, custom window and independent GPU.
use crate::gpu_mesh::{self, Quality, Vertex};
use crate::isometric::IsometricSurface;
use crate::textures::TextureAtlas;
use crate::Message;
use iced::wgpu;
use iced::widget::shader::{self, Viewport};
use iced::{mouse, Rectangle, Size};
use std::borrow::Cow;
use std::sync::Arc;

const WGSL: &str = r#"
struct VInput {
    @location(0) xy: vec2<f32>,
    @location(1) depth: f32,
    @location(2) rgba: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) mode: u32,
    @location(5) xyz: vec3<f32>,
    @location(6) world_mode: u32,
    @location(7) atlas_page: u32,
};
struct Camera {
    position:vec4<f32>,
    right:vec4<f32>,
    up:vec4<f32>,
    forward:vec4<f32>,
    projection:vec4<f32>,
};
@group(0) @binding(2) var<uniform> camera:Camera;
struct VOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) rgba: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) mode: u32,
    @location(3) @interpolate(flat) atlas_page: u32,
};
@vertex fn vs_main(input:VInput)->VOutput{
    var out:VOutput;
    if input.world_mode==1u {
        let relative=input.xyz-camera.position.xyz;
        let distance=dot(relative,camera.forward.xyz);
        let near=camera.projection.z;
        let far=camera.projection.w;
        out.position=vec4<f32>(
            dot(relative,camera.right.xyz)*camera.projection.x,
            dot(relative,camera.up.xyz)*camera.projection.y,
            far/(far-near)*distance-far*near/(far-near),
            distance);
    } else {
        out.position=vec4<f32>(input.xy,input.depth,1.0);
    }
    out.rgba=input.rgba;
    out.uv=input.uv;
    out.mode=input.mode;
    out.atlas_page=input.atlas_page;
    return out;
}
@group(0) @binding(0) var block_atlas:texture_2d_array<f32>;
@group(0) @binding(1) var block_sampler:sampler;
fn shade(input:VOutput)->vec4<f32>{
    return textureSample(block_atlas,block_sampler,input.uv,i32(input.atlas_page))*input.rgba;
}
// This is Minecraft-style fullbright: texture * authored tint, no light
// intensity attenuation, specular PBR highlights, shadows or time-of-day.
@fragment fn fs_opaque(input:VOutput,@builtin(front_facing) front:bool)->@location(0) vec4<f32>{
    if (input.mode & 3u)==2u{discard;}
    if (input.mode & 4u)!=0u && !front{discard;}
    let rgba=shade(input);
    if (input.mode & 3u)==1u {
        var threshold=0.5;
        if (input.mode & 16u)!=0u {
            threshold=f32((input.mode >> 8u) & 255u)/255.0;
        }
        if rgba.a<threshold {discard;}
    }
    if (input.mode & 8u)!=0u {
        // glTF OPAQUE ignores texture/vertex alpha and writes full depth.
        return vec4<f32>(rgba.rgb,1.0);
    }
    if rgba.a<=0.0039{discard;}
    return rgba;
}
@fragment fn fs_transparent(input:VOutput,@builtin(front_facing) front:bool)->@location(0) vec4<f32>{
    if (input.mode & 3u)!=2u{discard;}
    if (input.mode & 4u)!=0u && !front{discard;}
    let rgba=shade(input);
    if rgba.a<=0.0039{discard;}
    // Offscreen color is premultiplied; otherwise compositing the
    // intermediate scene over iced would multiply transparency twice.
    return vec4<f32>(rgba.rgb*rgba.a,rgba.a);
}
struct CompositeOut {
    @builtin(position) position:vec4<f32>,
    @location(0) uv:vec2<f32>,
};
@vertex fn vs_composite(@builtin(vertex_index) id:u32)->CompositeOut{
    let xy=array<vec2<f32>,6>(
        vec2<f32>(-1.0, 1.0),vec2<f32>(1.0,1.0),vec2<f32>(1.0,-1.0),
        vec2<f32>(-1.0, 1.0),vec2<f32>(1.0,-1.0),vec2<f32>(-1.0,-1.0)
    );
    var out:CompositeOut;
    out.position=vec4<f32>(xy[id],0.0,1.0);
    out.uv=vec2<f32>((xy[id].x+1.0)*0.5,(1.0-xy[id].y)*0.5);
    return out;
}
@fragment fn fs_composite(input:CompositeOut)->@location(0) vec4<f32>{
    return textureSample(block_atlas,block_sampler,input.uv,0);
}
"#;

/// Only a render source. The transparent Canvas overlay owns all mouse
/// navigation/selection events and 3D guides; it never picks from GPU pixels.
#[derive(Clone)]
pub(crate) enum SceneSource {
    Isometric(IsometricSurface),
    Spectator(crate::spectator::SpectatorScene),
}
#[derive(Clone)]
pub(crate) struct GpuScene(pub SceneSource);
#[derive(Debug)]
pub(crate) struct Primitive {
    vertices: Arc<Vec<Vertex>>,
    atlas: Option<Arc<TextureAtlas>>,
    quality: Quality,
    camera: Option<crate::spectator::CameraUniform>,
}
/// WebGPU depth runs 0 (nearest) to 1 (farthest); the isometric
/// projector uses larger camera-space depth for nearer source blocks.
fn to_clip(vertex: &mut Vertex, size: Size, min_depth: f32, max_depth: f32) {
    vertex.xy = [
        vertex.xy[0] / size.width.max(1.0) * 2.0 - 1.0,
        1.0 - vertex.xy[1] / size.height.max(1.0) * 2.0,
    ];
    let span = max_depth - min_depth;
    vertex.depth = if span > 1e-6 {
        ((max_depth - vertex.depth) / span).clamp(0.0, 1.0)
    } else {
        0.5
    };
}
impl IsometricSurface {
    #[cfg(test)]
    pub(crate) fn gpu_vertices_cached(&self, size: Size) -> Arc<Vec<Vertex>> {
        self.gpu_vertices_cancellable(size, &|| false)
            .expect("uninterruptible isometric GPU mesh")
    }
    /// Only committed complete meshes enter the shared per-view cache.
    pub(crate) fn gpu_vertices_cancellable(
        &self,
        size: Size,
        cancelled: &impl Fn() -> bool,
    ) -> Option<Arc<Vec<Vertex>>> {
        if cancelled() {
            return None;
        }
        {
            let cache = self
                .gpu_frame_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some((cached_size, vertices)) = &*cache {
                if *cached_size == size {
                    return Some(Arc::clone(vertices));
                }
            }
        }
        let mut vertices = gpu_mesh::build_cancellable(self, size, cancelled)?;
        let depth_min = vertices
            .iter()
            .map(|v| v.depth)
            .fold(f32::INFINITY, f32::min);
        let depth_max = vertices
            .iter()
            .map(|v| v.depth)
            .fold(f32::NEG_INFINITY, f32::max);
        for (index, vertex) in vertices.iter_mut().enumerate() {
            if index % 1024 == 0 && cancelled() {
                return None;
            }
            to_clip(vertex, size, depth_min, depth_max);
        }
        if cancelled() {
            return None;
        }
        let vertices = Arc::new(vertices);
        let mut cache = self
            .gpu_frame_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if cancelled() {
            return None;
        }
        *cache = Some((size, Arc::clone(&vertices)));
        Some(vertices)
    }
}
/// Render only complete world-space geometry and preserve its original
/// source coordinate basis while the next streamed mesh builds.
fn choose_spectator_mesh(
    current: Option<Arc<Vec<Vertex>>>,
    current_surface: &Arc<mcedit2_world::surface::SurfaceView>,
    fallback: Option<&(Arc<Vec<Vertex>>, Arc<mcedit2_world::surface::SurfaceView>)>,
) -> (Arc<Vec<Vertex>>, Arc<mcedit2_world::surface::SurfaceView>) {
    if let Some(vertices) = current {
        (vertices, Arc::clone(current_surface))
    } else if let Some((vertices, original_surface)) = fallback {
        (Arc::clone(vertices), Arc::clone(original_surface))
    } else {
        (Arc::new(Vec::new()), Arc::clone(current_surface))
    }
}
impl shader::Program<Message> for GpuScene {
    type State = ();
    type Primitive = Primitive;
    fn draw(&self, _: &Self::State, _: mouse::Cursor, bounds: Rectangle) -> Primitive {
        let size = bounds.size();
        match &self.0 {
            SceneSource::Isometric(scene) => {
                crate::mesh_async::enqueue(scene, size);
                let vertices = scene
                    .gpu_frame_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .filter(|(cached, _)| *cached == size)
                    .map(|(_, v)| Arc::clone(v))
                    .unwrap_or_else(|| Arc::new(Vec::new()));
                Primitive {
                    vertices,
                    atlas: scene.texture_bank.as_ref().and_then(|b| b.atlas()).cloned(),
                    quality: scene.gpu_quality,
                    camera: None,
                }
            }
            SceneSource::Spectator(scene) => {
                crate::mesh_async::enqueue_spectator(scene);
                let ready = scene.cache.lock()
                    .unwrap_or_else(|e| e.into_inner()).as_ref().cloned();
                let (vertices, source) = choose_spectator_mesh(
                    ready, &scene.surface, scene.fallback.as_ref());
                Primitive {
                    vertices,
                    atlas: scene.texture_bank.as_ref().and_then(|b| b.atlas()).cloned(),
                    quality: scene.quality,
                    camera: Some(scene.camera.uniform(source.as_ref(), size)),
                }
            }
        }
    }
}
pub(crate) struct Pipeline {
    opaque: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    capacity: u64,
    count: u32,
    /// Mesh buffers split only at the adapter's real max_buffer_size.
    /// This replaces the previous fixed global face cap.
    segments: Vec<(wgpu::Buffer, u32)>,
    /// Last *uploaded* vertex order, including transparency sorting. Retaining
    /// the Arc is cheap for ordinary source meshes and permits suffix writes
    /// instead of re-uploading unchanged exact chunks each stream update.
    last_uploaded_vertices: Option<Arc<Vec<Vertex>>>,
    /// Hold the original Arc to make mesh identity pointer comparisons safe:
    /// dropping it would let a later allocator reuse the same address.
    last_source_mesh: Option<Arc<Vec<Vertex>>>,
    /// Immutable for a given source-mesh Arc; a camera move should not scan
    /// every opaque face just to rediscover whether sorting is necessary.
    has_transparent_world_faces: bool,
    atlas: Option<Arc<TextureAtlas>>,
    atlas_texture: wgpu::Texture,
    atlas_layout: wgpu::BindGroupLayout,
    nearest: wgpu::Sampler,
    linear: wgpu::Sampler,
    atlas_bind_group: wgpu::BindGroup,
    quality: Quality,
    offscreen_color: wgpu::Texture,
    offscreen_depth: wgpu::Texture,
    offscreen_size: (u32, u32),
    composite_bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    last_camera: Option<crate::spectator::CameraUniform>,
}
/// Respect the active adapter's maximum vertex buffer allocation. A scene
/// can contain more geometry than a single device buffer can hold; split at
/// complete quads (six vertices) without dropping faces or changing order.
fn gpu_segment_vertices(max_buffer_size: u64) -> usize {
    let stride = std::mem::size_of::<Vertex>() as u64;
    ((max_buffer_size / stride).min(u32::MAX as u64) / 6 * 6) as usize
}
/// Determine whether a redraw is needed and the first vertex whose bytes
/// must be uploaded. Only a common prefix may safely stay on an existing
/// wgpu buffer: insertions or removals shift all subsequent vertex offsets.
/// The entire remainder is uploaded as one contiguous, aligned write.
fn mesh_upload_suffix(
    previous: Option<&[Vertex]>,
    current: &[Vertex],
    buffer_resized: bool,
) -> (bool, Option<usize>) {
    if buffer_resized || previous.is_none() {
        return (true, (!current.is_empty()).then_some(0));
    }
    let previous = previous.expect("checked above");
    let shared = previous.len().min(current.len());
    let change = previous[..shared]
        .iter()
        .zip(&current[..shared])
        .position(|(a, b)| bytemuck::bytes_of(a) != bytemuck::bytes_of(b));
    let first_upload = change.or_else(|| (current.len() > previous.len()).then_some(shared));
    // Removing a trailing chunk only changes draw count; the GPU prefix is
    // already correct and writing an empty range would be invalid in wgpu.
    (
        first_upload.is_some() || previous.len() != current.len(),
        first_upload,
    )
}
/// Return byte-aligned GPU ranges rather than uploading everything after
/// the first changed chunk. For equal-length meshes, compare whole faces so
/// two unchanged source chunks around a dirty chunk retain their GPU bytes.
///
/// Nearby dirty faces are coalesced when fewer bytes are transferred than the
/// overhead of issuing many queue.write_buffer calls. If the length changes,
/// fall back to the safe shifted suffix: offsets following inserted/removed
/// geometry are no longer stable. No visible vertices are discarded.
fn mesh_upload_ranges(
    previous: Option<&[Vertex]>,
    current: &[Vertex],
    buffer_resized: bool,
) -> (bool, Vec<std::ops::Range<usize>>) {
    // The common equal-length edit case needs only one linear comparison
    // pass; calling the old suffix finder first doubled the scan bandwidth.
    if buffer_resized || previous.is_none_or(|old|
        old.len() != current.len() || current.len() % 6 != 0)
    {
        let (changed, first) = mesh_upload_suffix(previous, current, buffer_resized);
        return (changed, first.map(|i| vec![i..current.len()]).unwrap_or_default());
    }
    let previous = previous.expect("validated equal-length mesh above");
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    for begin in (0..current.len()).step_by(6) {
        let end = begin + 6;
        let changed = bytemuck::cast_slice::<Vertex, u8>(&previous[begin..end])
            != bytemuck::cast_slice::<Vertex, u8>(&current[begin..end]);
        if !changed {
            continue;
        }
        if let Some(last) = ranges.last_mut() {
            // Each GPU write has a staging and dispatch cost. Merge dirty
            // faces separated by at most 4 KiB of unchanged source vertices.
            let unchanged_bytes =
                (begin - last.end) * std::mem::size_of::<Vertex>();
            if unchanged_bytes <= 4096 {
                last.end = end;
                continue;
            }
        }
        ranges.push(begin..end);
    }
    if ranges.is_empty() {
        return (false, ranges);
    }
    // If fragmentation outweighs the bytes saved, one suffix write is
    // cheaper. This is an upload heuristic, never a geometry budget.
    if ranges.len() > 1
        && ranges.len().saturating_mul(4096)
            > current.len().saturating_mul(std::mem::size_of::<Vertex>())
    {
        let first = ranges[0].start;
        return (true, vec![first..current.len()]);
    }
    (true, ranges)
}
/// Perspective transparency is emitted as exactly two adjacent triangles
/// per model face. Sort whole six-vertex quads, never individual triangles:
/// a sloped fluid quad has different triangle centroids and splitting it
/// makes the two halves draw on opposite sides of nearby transparent faces.
/// Opaque and alpha-cutout quads remain in their original source positions.
fn ordered_transparent_vertices(
    vertices: &[Vertex],
    camera: &crate::spectator::CameraUniform,
) -> Vec<Vertex> {
    let mut sorted = vertices.to_vec();
    let mut slots = Vec::new();
    let mut faces = Vec::new();
    for (index, quad) in vertices.chunks_exact(6).enumerate() {
        if !quad
            .iter()
            .all(|vertex| (vertex.mode & 3) == 2 && vertex.world_mode == 1)
        {
            continue;
        }
        // Original quad order: 0,1,2,0,2,3. Count every source corner
        // exactly once, rather than weighting duplicated diagonal vertices.
        let center = std::array::from_fn::<_, 3, _>(|axis| {
            (quad[0].xyz[axis] + quad[1].xyz[axis] + quad[2].xyz[axis] + quad[5].xyz[axis]) * 0.25
        });
        let depth = (0..3)
            .map(|i| (center[i] - camera.position[i]) * camera.forward[i])
            .sum::<f32>();
        slots.push(index * 6);
        faces.push((depth, index * 6));
    }
    // Stable tie handling: camera crossing a plane must not oscillate the
    // order of equal-depth glass quads from one frame to the next.
    faces.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    for (destination, (_, source)) in slots.into_iter().zip(faces) {
        sorted[destination..destination + 6].copy_from_slice(&vertices[source..source + 6]);
    }
    sorted
}
fn make_texture(device: &wgpu::Device, width: u32, height: u32, layers: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("MCEdit2 read-only block atlas"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}
fn make_sampler(device: &wgpu::Device, filter: wgpu::FilterMode) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("MCEdit2 atlas sampler"),
        mag_filter: filter,
        min_filter: filter,
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        ..Default::default()
    })
}
fn make_binding(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    texture: &wgpu::Texture,
    sampler: &wgpu::Sampler,
    camera: &wgpu::Buffer,
) -> wgpu::BindGroup {
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("MCEdit2 atlas binding"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: camera.as_entire_binding(),
            },
        ],
    })
}
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
/// Render targets are limited by the actual wgpu adapter, not an arbitrary
/// 4096px cap that silently reduces viewport resolution on high-DPI displays.
fn make_offscreen_color(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("MCEdit2 offscreen voxel color"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}
fn make_offscreen_depth(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("MCEdit2 offscreen voxel Z buffer"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}
fn target_dimensions(bounds: &Rectangle, device_limit: u32) -> (u32, u32) {
    let limit = device_limit.max(1);
    (
        (bounds.width.ceil().max(1.0) as u32).min(limit),
        (bounds.height.ceil().max(1.0) as u32).min(limit),
    )
}
fn scene_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    fragment: &str,
    depth_write: bool,
) -> wgpu::RenderPipeline {
    const ATTRS: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
        0=>Float32x2,1=>Float32,2=>Float32x4,3=>Float32x2,4=>Uint32,
        5=>Float32x3,6=>Uint32,7=>Uint32
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("MCEdit2 depth-tested original face stage"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &ATTRS,
            }],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: if depth_write {
                    None
                } else {
                    Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    })
                },
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: depth_write,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}
impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let source = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("MCEdit2 original-voxel depth and composition"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(WGSL)),
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MCEdit2 source atlas and compositing layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("MCEdit2 GPU source and depth compositing layout"),
            bind_group_layouts: &[&atlas_layout],
            push_constant_ranges: &[],
        });
        let opaque = scene_pipeline(device, &source, &layout, "fs_opaque", true);
        let transparent = scene_pipeline(device, &source, &layout, "fs_transparent", false);
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("MCEdit2 composite depth-tested voxel scene onto iced"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &source,
                entry_point: Some("vs_composite"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &source,
                entry_point: Some("fs_composite"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let capacity = std::mem::size_of::<Vertex>() as u64 * 6;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MCEdit2 native GPU source mesh"),
            size: capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let atlas_texture = make_texture(device, 1, 1, 1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MCEdit2 spectator camera uniform"),
            size: std::mem::size_of::<crate::spectator::CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &camera_buffer,
            0,
            bytemuck::bytes_of(&crate::spectator::CameraUniform::default()),
        );
        let nearest = make_sampler(device, wgpu::FilterMode::Nearest);
        let linear = make_sampler(device, wgpu::FilterMode::Linear);
        let atlas_bind_group = make_binding(
            device,
            &atlas_layout,
            &atlas_texture,
            &nearest,
            &camera_buffer,
        );
        let offscreen_color = make_offscreen_color(device, 1, 1);
        let offscreen_depth = make_offscreen_depth(device, 1, 1);
        let composite_bind_group = make_binding(
            device,
            &atlas_layout,
            &offscreen_color,
            &nearest,
            &camera_buffer,
        );
        Self {
            opaque,
            transparent,
            composite,
            buffer,
            capacity,
            count: 0,
            segments: Vec::new(),
            last_uploaded_vertices: None,
            last_source_mesh: None,
            has_transparent_world_faces: false,
            atlas: None,
            atlas_texture,
            atlas_layout,
            nearest,
            linear,
            atlas_bind_group,
            quality: Quality::Balanced,
            offscreen_color,
            offscreen_depth,
            offscreen_size: (1, 1),
            composite_bind_group,
            camera_buffer,
            last_camera: None,
        }
    }
}
impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;
    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        _: &Viewport,
    ) {
        let mesh_identity_changed = pipeline
            .last_source_mesh
            .as_ref()
            .is_none_or(|previous| !Arc::ptr_eq(previous, &self.vertices));
        let changed_camera = pipeline.last_camera != self.camera;
        if mesh_identity_changed {
            pipeline.has_transparent_world_faces = self.vertices.iter()
                .any(|v| (v.mode & 3) == 2 && v.world_mode == 1);
        }
        let needs_transparent_sort = self.camera.is_some()
            && pipeline.has_transparent_world_faces;
        let resort = needs_transparent_sort && (changed_camera || mesh_identity_changed);
        // Transparent ordering is itself the uploaded representation. Reuse
        // it across unchanged camera frames instead of replacing it with
        // original (unsorted) vertex order on a redraw.
        let unchanged_scene = !mesh_identity_changed && !resort;
        let uploaded = if unchanged_scene {
            None
        } else if resort {
            self.camera.as_ref().map(|camera| {
                Arc::new(ordered_transparent_vertices(
                    self.vertices.as_slice(),
                    camera,
                ))
            })
        } else {
            Some(Arc::clone(&self.vertices))
        };
        let stride = std::mem::size_of::<Vertex>() as u64;
        let needed = (self.vertices.len() as u64).saturating_mul(stride);
        let max_buffer = device.limits().max_buffer_size;
        let segmented = needed > max_buffer || self.vertices.len() > u32::MAX as usize;
        let changed_mesh: bool;
        if segmented {
            // This is device segmentation, not an arbitrary scene quota:
            // draw all the uploaded quads in stable opaque/translucent order.
            let chunk_len = gpu_segment_vertices(max_buffer);
            if chunk_len == 0 {
                // The adapter cannot fit even one model quad.
                return;
            }
            let rebuild = !unchanged_scene || pipeline.segments.is_empty();
            if rebuild {
                let current = uploaded.as_ref().unwrap_or(&self.vertices);
                let mut chunks = Vec::new();
                for part in current.chunks(chunk_len) {
                    let bytes = bytemuck::cast_slice::<Vertex, u8>(part);
                    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("MCEdit2 original world mesh segment"),
                        size: bytes.len() as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    queue.write_buffer(&buffer, 0, bytes);
                    chunks.push((buffer, part.len() as u32));
                }
                pipeline.segments = chunks;
                pipeline.last_uploaded_vertices = Some(
                    uploaded.unwrap_or_else(|| Arc::clone(&self.vertices))
                );
            }
            pipeline.count = 0;
            changed_mesh = rebuild;
        } else {
            let was_segmented = !pipeline.segments.is_empty();
            if was_segmented {
                pipeline.segments.clear();
                pipeline.last_uploaded_vertices = None;
            }
            let resized_mesh = needed > pipeline.capacity;
            if resized_mesh {
                pipeline.capacity = needed.next_power_of_two().min(max_buffer);
                pipeline.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("MCEdit2 resized original source GPU mesh"),
                    size: pipeline.capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            let (modified, upload_ranges) = if unchanged_scene && !resized_mesh && !was_segmented {
                (false, Vec::new())
            } else {
                let current = uploaded.as_ref().unwrap_or(&self.vertices);
                mesh_upload_ranges(
                    pipeline.last_uploaded_vertices.as_deref().map(Vec::as_slice),
                    current,
                    resized_mesh || was_segmented,
                )
            };
            if !upload_ranges.is_empty() {
                let current = uploaded.as_ref().unwrap_or(&self.vertices);
                for range in upload_ranges {
                    let offset = range.start * std::mem::size_of::<Vertex>();
                    let bytes = bytemuck::cast_slice::<Vertex, u8>(&current[range]);
                    if !bytes.is_empty() {
                        queue.write_buffer(&pipeline.buffer, offset as u64, bytes);
                    }
                }
            }
            if let Some(vertices) = uploaded {
                pipeline.last_uploaded_vertices = Some(vertices);
            }
            pipeline.count = self.vertices.len() as u32;
            changed_mesh = modified || was_segmented;
        }
        pipeline.last_source_mesh = Some(Arc::clone(&self.vertices));
        let changed_atlas = match (&pipeline.atlas, &self.atlas) {
            (Some(old), Some(new)) => !Arc::ptr_eq(old, new),
            (None, None) => false,
            _ => true,
        };
        if changed_atlas {
            // Rebuild the array only when layout/layer count changes.
            // Sprite animation replaces just the touched Arc atlas pages.
            let target = self
                .atlas
                .as_ref()
                .and_then(|a| {
                    a.pages
                        .first()
                        .map(|p| (p.width, p.height, a.page_count() as u32))
                })
                .unwrap_or((1, 1, 1));
            let previous = pipeline
                .atlas
                .as_ref()
                .and_then(|a| {
                    a.pages
                        .first()
                        .map(|p| (p.width, p.height, a.page_count() as u32))
                })
                .unwrap_or((1, 1, 1));
            let layout_changed = target != previous;
            if layout_changed {
                pipeline.atlas_texture = make_texture(device, target.0, target.1, target.2);
            }
            if let Some(atlas) = self.atlas.as_ref() {
                for (layer, page) in atlas.pages.iter().enumerate() {
                    if !layout_changed
                        && pipeline
                            .atlas
                            .as_ref()
                            .and_then(|old| old.pages.get(layer))
                            .is_some_and(|prev| Arc::ptr_eq(prev, page))
                    {
                        continue;
                    }
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &pipeline.atlas_texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: 0,
                                y: 0,
                                z: layer as u32,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        &page.rgba,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(page.width * 4),
                            rows_per_image: Some(page.height),
                        },
                        wgpu::Extent3d {
                            width: page.width,
                            height: page.height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
            } else {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &pipeline.atlas_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &[255, 255, 255, 255],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4),
                        rows_per_image: Some(1),
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
            }
            pipeline.atlas = self.atlas.clone();
        }
        if changed_camera {
            queue.write_buffer(
                &pipeline.camera_buffer,
                0,
                bytemuck::bytes_of(&self.camera.unwrap_or_default()),
            );
            pipeline.last_camera = self.camera;
        }
        let changed_quality = pipeline.quality != self.quality;
        if changed_atlas || changed_quality {
            pipeline.quality = self.quality;
            let sampler = match self.quality {
                Quality::Detailed => &pipeline.linear,
                _ => &pipeline.nearest,
            };
            pipeline.atlas_bind_group = make_binding(
                device,
                &pipeline.atlas_layout,
                &pipeline.atlas_texture,
                sampler,
                &pipeline.camera_buffer,
            );
        }
        let target = target_dimensions(bounds, device.limits().max_texture_dimension_2d);
        let resized_view = target != pipeline.offscreen_size;
        if resized_view {
            pipeline.offscreen_color = make_offscreen_color(device, target.0, target.1);
            pipeline.offscreen_depth = make_offscreen_depth(device, target.0, target.1);
            pipeline.offscreen_size = target;
            pipeline.composite_bind_group = make_binding(
                device,
                &pipeline.atlas_layout,
                &pipeline.offscreen_color,
                &pipeline.nearest,
                &pipeline.camera_buffer,
            );
        }
        // Re-render the intermediate color and Z targets only if the source
        // faces, source PNG atlas, quality sampler or viewport have changed.
        if !changed_mesh && !changed_atlas && !changed_quality && !changed_camera && !resized_view {
            return;
        }
        let color = pipeline
            .offscreen_color
            .create_view(&wgpu::TextureViewDescriptor::default());
        let depth = pipeline
            .offscreen_depth
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("MCEdit2 original-voxel opaque and translucent depth passes"),
        });
        // Opaque and binary-alpha-cutout fragments populate Z. The exact
        // source model triangles can intersect without painter-order errors.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Original opaque model Z prepass and color"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: if self.camera.is_some() { 0.47 } else { 0.0 },
                            g: if self.camera.is_some() { 0.68 } else { 0.0 },
                            b: if self.camera.is_some() { 0.94 } else { 0.0 },
                            a: if self.camera.is_some() { 1.0 } else { 0.0 },
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if pipeline.count > 0 || !pipeline.segments.is_empty() {
                pass.set_pipeline(&pipeline.opaque);
                pass.set_bind_group(0, &pipeline.atlas_bind_group, &[]);
                if pipeline.segments.is_empty() {
                    pass.set_vertex_buffer(0, pipeline.buffer.slice(..));
                    pass.draw(0..pipeline.count, 0..1);
                } else {
                    for (buffer, count) in &pipeline.segments {
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        pass.draw(0..*count, 0..1);
                    }
                }
            }
        }
        // Translucent faces remain sorted far-to-near. They depth-test
        // against opaque material but MUST NOT overwrite the depth target.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Original translucent source faces over known opaque Z"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if pipeline.count > 0 || !pipeline.segments.is_empty() {
                pass.set_pipeline(&pipeline.transparent);
                pass.set_bind_group(0, &pipeline.atlas_bind_group, &[]);
                if pipeline.segments.is_empty() {
                    pass.set_vertex_buffer(0, pipeline.buffer.slice(..));
                    pass.draw(0..pipeline.count, 0..1);
                } else {
                    for (buffer, count) in &pipeline.segments {
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        pass.draw(0..*count, 0..1);
                    }
                }
            }
        }
        queue.submit(Some(encoder.finish()));
    }
    fn draw(&self, pipeline: &Pipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        // iced owns the actual swapchain pass. Only sample the completed
        // depth-tested offscreen result into the shader widget's viewport.
        pass.set_pipeline(&pipeline.composite);
        pass.set_bind_group(0, &pipeline.composite_bind_group, &[]);
        pass.draw(0..6, 0..1);
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spectator_mesh_handoff_retains_previous_complete_vertices_and_origin() {
        use mcedit2_world::surface::SurfaceView;
        let source = |x: i32| Arc::new(SurfaceView {
            x, z: 0, y_min: 64, y_max: 111, width: 16, depth: 16,
            voxels: vec![], source_tiles: vec![], source_chunks: 0,
            missing_columns: 0,
        });
        let old_origin = source(0);
        let new_origin = source(64);
        let old_mesh = Arc::new(vec![Vertex {
            xy: [0.0;2], depth: 0.0, rgba: [1.0;4], uv: [0.0;2],
            mode: 0, xyz: [0.5, 0.0, 0.0], world_mode: 1, atlas_page: 0,
        }]);
        let fallback = (Arc::clone(&old_mesh), Arc::clone(&old_origin));
        let (during, origin) = choose_spectator_mesh(None, &new_origin, Some(&fallback));
        assert!(Arc::ptr_eq(&during, &old_mesh));
        assert!(Arc::ptr_eq(&origin, &old_origin));
        let replacement = Arc::new(vec![old_mesh[0], old_mesh[0]]);
        let (ready, origin) = choose_spectator_mesh(
            Some(Arc::clone(&replacement)), &new_origin, Some(&fallback));
        assert!(Arc::ptr_eq(&ready, &replacement));
        assert!(Arc::ptr_eq(&origin, &new_origin));
    }
    fn test_camera(forward: [f32; 3]) -> crate::spectator::CameraUniform {
        crate::spectator::CameraUniform {
            forward: [forward[0], forward[1], forward[2], 0.0],
            ..Default::default()
        }
    }
    fn test_quad(id: f32, depth: f32, mode: u32, world_mode: u32) -> [Vertex; 6] {
        let corners = [
            [0.0, 0.0, depth],
            [1.0, 0.0, depth + 8.0],
            [1.0, 1.0, depth],
            [0.0, 1.0, depth],
        ];
        std::array::from_fn(|i| Vertex {
            xy: [id, i as f32],
            depth: id,
            rgba: [1.0; 4],
            uv: [0.0; 2],
            mode,
            xyz: corners[[0, 1, 2, 0, 2, 3][i]],
            world_mode,
            atlas_page: id as u32,
        })
    }
    #[test]
    fn transparent_quads_sort_as_whole_original_model_faces() {
        let mut vertices = Vec::new();
        vertices.extend(test_quad(10.0, 0.0, 2, 1));
        vertices.extend(test_quad(20.0, 4.0, 0, 1)); // opaque stays in place
        vertices.extend(test_quad(30.0, 4.0, 2, 1));
        vertices.extend(test_quad(40.0, 4.0, 2, 0)); // isometric untouched
        let sorted = ordered_transparent_vertices(&vertices, &test_camera([0.0, 0.0, 1.0]));
        // The first quad has a sloped triangle, but its entire model face
        // must still be sorted by its unique four-corner centroid.
        assert!(sorted[0..6].iter().all(|v| v.atlas_page == 30));
        assert!(sorted[6..12].iter().all(|v| v.atlas_page == 20));
        assert!(sorted[12..18].iter().all(|v| v.atlas_page == 10));
        assert!(sorted[18..24].iter().all(|v| v.atlas_page == 40));
        assert_eq!(sorted.len(), vertices.len());
        // Reverse direction reverses the two separate translucent faces.
        let reversed = ordered_transparent_vertices(&vertices, &test_camera([0.0, 0.0, -1.0]));
        assert!(reversed[0..6].iter().all(|v| v.atlas_page == 10));
        assert!(reversed[12..18].iter().all(|v| v.atlas_page == 30));
    }
    #[test]
    fn gltf_single_sided_blend_mode_keeps_back_to_front_transparency_sort() {
        let encoded = crate::material_mode::encode(0, Some("BLEND"), 0.5, false);
        let mut vertices = Vec::new();
        vertices.extend(test_quad(10.0, 2.0, encoded, 1));
        vertices.extend(test_quad(30.0, 8.0, encoded, 1));
        vertices.extend(test_quad(20.0, 4.0, 0, 1)); // opaque always fixed
        let sorted = ordered_transparent_vertices(&vertices, &test_camera([0., 0., 1.]));
        assert!(sorted[..6].iter().all(|v| v.atlas_page == 30));
        assert!(sorted[6..12].iter().all(|v| v.atlas_page == 10));
        assert!(sorted[12..].iter().all(|v| v.atlas_page == 20));
    }
    #[test]
    fn transparent_equal_depth_is_deterministic_and_keeps_triangle_pairs() {
        let mut vertices = Vec::new();
        vertices.extend(test_quad(1.0, 3.0, 2, 1));
        vertices.extend(test_quad(2.0, 3.0, 2, 1));
        let first = ordered_transparent_vertices(&vertices, &test_camera([0.0, 0.0, 1.0]));
        assert_eq!(first, vertices);
        assert_eq!(
            ordered_transparent_vertices(&vertices, &test_camera([0.0, 0.0, 1.0])),
            first
        );
    }
    fn gpu_test_quad(id: f32) -> [Vertex; 6] {
        [Vertex {
            xy: [0.0; 2],
            depth: id,
            rgba: [1.0; 4],
            uv: [0.0; 2],
            mode: 0,
            xyz: [id, 0.0, 0.0],
            world_mode: 1,
            atlas_page: 0,
        }; 6]
    }
    #[test]
    fn hardware_mesh_segmentation_preserves_complete_quads() {
        let stride = std::mem::size_of::<Vertex>() as u64;
        assert_eq!(gpu_segment_vertices(stride * 6), 6);
        assert_eq!(gpu_segment_vertices(stride * 16), 12);
        assert_eq!(gpu_segment_vertices(stride * 5), 0);
        let full = (0..30).collect::<Vec<_>>();
        let rebuilt = full.chunks(gpu_segment_vertices(stride * 12))
            .flat_map(|chunk| chunk.iter().copied()).collect::<Vec<_>>();
        assert_eq!(rebuilt, full);
    }
    #[test]
    fn gpu_upload_avoids_rewriting_unchanged_prefix_and_handles_shorter_mesh() {
        let a = gpu_test_quad(1.0);
        let b = gpu_test_quad(2.0);
        let c = gpu_test_quad(3.0);
        let original = [a.as_slice(), b.as_slice()].concat();
        let appended = [a.as_slice(), b.as_slice(), c.as_slice()].concat();
        assert_eq!(
            mesh_upload_suffix(Some(&original), &original, false),
            (false, None)
        );
        assert_eq!(
            mesh_upload_suffix(Some(&original), &appended, false),
            (true, Some(12))
        );
        assert_eq!(
            mesh_upload_suffix(Some(&appended), &original, false),
            (true, None)
        );
        assert_eq!(mesh_upload_suffix(None, &original, false), (true, Some(0)));
        assert_eq!(
            mesh_upload_suffix(Some(&original), &original, true),
            (true, Some(0))
        );
    }
    #[test]
    fn equal_length_chunk_edits_only_upload_local_dirty_faces() {
        let original: Vec<Vertex> = (0..256)
            .flat_map(|i| gpu_test_quad(i as f32))
            .collect();
        let mut updated = original.clone();
        updated[6].atlas_page = 12;
        updated[255 * 6].atlas_page = 34;
        let (changed, ranges) = mesh_upload_ranges(Some(&original), &updated, false);
        assert!(changed);
        assert_eq!(ranges, vec![6..12, 255 * 6..256 * 6]);
        assert!(ranges.iter().map(|r| r.len()).sum::<usize>() < updated.len());
        assert_eq!(mesh_upload_ranges(Some(&updated), &updated, false), (false, vec![]));
        assert_eq!(mesh_upload_ranges(None, &updated, false), (true, vec![0..updated.len()]));
    }
    #[test]
    fn shifted_geometry_uploads_suffix_and_shrinking_only_updates_draw_count() {
        let a = gpu_test_quad(1.0);
        let b = gpu_test_quad(2.0);
        let c = gpu_test_quad(3.0);
        let old = [a.as_slice(), b.as_slice(), c.as_slice()].concat();
        let shifted = [a.as_slice(), c.as_slice()].concat();
        assert_eq!(mesh_upload_ranges(Some(&old), &shifted, false),
            (true, vec![6..12]));
        let removed_end = [a.as_slice(), b.as_slice()].concat();
        assert_eq!(mesh_upload_ranges(Some(&old), &removed_end, false),
            (true, vec![]));
        assert_eq!(mesh_upload_ranges(Some(&old), &old, true),
            (true, vec![0..old.len()]));
    }
    #[test]
    fn replacing_middle_chunk_uploads_entire_shifted_suffix() {
        let a = gpu_test_quad(1.0);
        let b = gpu_test_quad(2.0);
        let c = gpu_test_quad(3.0);
        let d = gpu_test_quad(4.0);
        let old = [a.as_slice(), b.as_slice(), c.as_slice()].concat();
        let new = [a.as_slice(), d.as_slice(), c.as_slice()].concat();
        assert_eq!(mesh_upload_suffix(Some(&old), &new, false), (true, Some(6)));
        let shifted = [a.as_slice(), c.as_slice()].concat();
        assert_eq!(
            mesh_upload_suffix(Some(&old), &shifted, false),
            (true, Some(6))
        );
    }
    #[test]
    fn bitwise_vertex_comparison_is_stable_across_identical_allocations() {
        let original = gpu_test_quad(1.0).to_vec();
        let another = original.clone();
        assert_eq!(
            mesh_upload_suffix(Some(&original), &another, false),
            (false, None)
        );
        // NaN channels have unequal float comparisons, but identical Pod
        // bytes must not trigger needless uploads between scene snapshots.
        let mut nan = original.clone();
        nan[0].depth = f32::from_bits(0x7fc0_0001);
        let same_nan = nan.clone();
        assert_eq!(
            mesh_upload_suffix(Some(&nan), &same_nan, false),
            (false, None)
        );
        assert_eq!(
            mesh_upload_suffix(Some(&original), &nan, false),
            (true, Some(0))
        );
    }
    #[test]
    fn vertex_world_depth_maps_nearest_to_zero_and_farthest_to_one() {
        let mut v = Vertex {
            xy: [50.0, 25.0],
            depth: 10.0,
            rgba: [1.0, 0.0, 0.0, 1.0],
            uv: [0.5, 0.5],
            mode: 0,
            xyz: [0.0; 3],
            world_mode: 0,
            atlas_page: 0,
        };
        to_clip(&mut v, Size::new(100.0, 50.0), -10.0, 10.0);
        assert_eq!(v.xy, [0.0, 0.0]);
        assert_eq!(v.depth, 0.0);
        let mut far = Vertex { depth: -10.0, ..v };
        to_clip(&mut far, Size::new(100.0, 50.0), -10.0, 10.0);
        assert_eq!(far.depth, 1.0);
        let mut flat = Vertex { depth: 7.0, ..v };
        to_clip(&mut flat, Size::new(200.0, 100.0), 7.0, 7.0);
        assert_eq!(flat.depth, 0.5);
        assert_eq!(flat.xy, [-1.0, 1.0]);
    }
    #[test]
    fn depth_pipeline_excludes_blend_from_opaque_and_skips_depth_writes_for_blend() {
        assert!(WGSL.contains("(input.mode & 3u)==2u"));
        assert!(WGSL.contains("(input.mode & 3u)!=2u"));
        assert!(WGSL.contains("rgba.a<threshold"));
        assert!(WGSL.contains("vec4<f32>(rgba.rgb,1.0)"));
        assert!(WGSL.contains("@builtin(front_facing) front:bool"));
        assert!(WGSL.contains("vs_composite"));
        assert!(WGSL.contains("rgba.rgb*rgba.a"));
        assert!(WGSL.contains("texture_2d_array<f32>"));
        assert!(WGSL.contains("i32(input.atlas_page)"));
        assert!(WGSL.contains("@interpolate(flat) atlas_page"));
        assert!(WGSL.contains("textureSample(block_atlas,block_sampler,input.uv,0)"));
        assert_eq!(COLOR_FORMAT, wgpu::TextureFormat::Rgba8Unorm);
        assert_eq!(DEPTH_FORMAT, wgpu::TextureFormat::Depth24Plus);
    }
    #[test]
    fn render_target_dimensions_are_capped_and_nonzero() {
        let rect = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 750.2,
            height: 360.5,
        };
        assert_eq!(target_dimensions(&rect, 8192), (751, 361));
        let empty = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
        assert_eq!(target_dimensions(&empty, 8192), (1, 1));
        let massive = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 9000.0,
            height: 9000.0,
        };
        assert_eq!(target_dimensions(&massive, 4096), (4096, 4096));
        assert_eq!(target_dimensions(&massive, 8192), (8192, 8192));
    }
}
