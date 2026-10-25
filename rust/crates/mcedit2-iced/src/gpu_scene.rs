//! Native iced 0.14 wgpu shader renderer of the source-voxel mesh.
//! Projected face geometry and hit tests stay shared with the Canvas path.
//! Shader avoids an application build, custom window and independent GPU.
use iced::{Rectangle,Size,mouse};
use iced::wgpu;
use iced::widget::shader::{self,Viewport};
use crate::gpu_mesh::{self,Vertex,Quality};
use crate::textures::TextureAtlas;
use std::sync::Arc;
use crate::isometric::IsometricSurface;
use crate::Message;
use std::borrow::Cow;
use std::hash::Hasher;

const WGSL:&str=r#"
struct VInput {
    @location(0) xy: vec2<f32>,
    @location(1) depth: f32,
    @location(2) rgba: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) mode: u32,
};
struct VOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) rgba: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) mode: u32,
};
@vertex fn vs_main(input:VInput)->VOutput{
    var out:VOutput;
    out.position=vec4<f32>(input.xy,input.depth,1.0);
    out.rgba=input.rgba;
    out.uv=input.uv;
    out.mode=input.mode;
    return out;
}
@group(0) @binding(0) var block_atlas:texture_2d<f32>;
@group(0) @binding(1) var block_sampler:sampler;
fn shade(input:VOutput)->vec4<f32>{
    return textureSample(block_atlas,block_sampler,input.uv)*input.rgba;
}
@fragment fn fs_opaque(input:VOutput)->@location(0) vec4<f32>{
    if input.mode==2u{discard;}
    let rgba=shade(input);
    if input.mode==1u && rgba.a<0.5{discard;}
    if rgba.a<=0.0039{discard;}
    return rgba;
}
@fragment fn fs_transparent(input:VOutput)->@location(0) vec4<f32>{
    if input.mode!=2u{discard;}
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
    return textureSample(block_atlas,block_sampler,input.uv);
}
"#;

/// Only a render source. The transparent Canvas overlay owns all mouse
/// navigation/selection events and 3D guides; it never picks from GPU pixels.
#[derive(Clone)]
pub(crate) struct GpuScene(pub IsometricSurface);
#[derive(Debug)]
pub(crate) struct Primitive{
    vertices:Arc<Vec<Vertex>>,
    atlas:Option<Arc<TextureAtlas>>,
    quality:Quality,
}
/// WebGPU depth runs 0 (nearest) to 1 (farthest); the isometric
/// projector uses larger camera-space depth for nearer source blocks.
fn to_clip(vertex:&mut Vertex,size:Size,min_depth:f32,max_depth:f32){
    vertex.xy=[
        vertex.xy[0]/size.width.max(1.0)*2.0-1.0,
        1.0-vertex.xy[1]/size.height.max(1.0)*2.0,
    ];
    let span=max_depth-min_depth;
    vertex.depth=if span>1e-6{
        ((max_depth-vertex.depth)/span).clamp(0.0,1.0)
    }else{0.5};
}
impl IsometricSurface{
    pub(crate) fn gpu_vertices_cached(&self,size:Size)->Arc<Vec<Vertex>>{
        {
            let cache=self.gpu_frame_cache.lock()
                .unwrap_or_else(|e|e.into_inner());
            if let Some((cached_size,vertices))=&*cache{
                if *cached_size==size{return Arc::clone(vertices);}
            }
        }
        let mut vertices=gpu_mesh::build(self,size);
        let depth_min=vertices.iter().map(|v|v.depth)
            .fold(f32::INFINITY,f32::min);
        let depth_max=vertices.iter().map(|v|v.depth)
            .fold(f32::NEG_INFINITY,f32::max);
        for vertex in &mut vertices{to_clip(vertex,size,depth_min,depth_max);}
        let vertices=Arc::new(vertices);
        let mut cache=self.gpu_frame_cache.lock()
            .unwrap_or_else(|e|e.into_inner());
        *cache=Some((size,Arc::clone(&vertices)));
        vertices
    }
}
impl shader::Program<Message> for GpuScene{
    type State=();
    type Primitive=Primitive;
    fn draw(&self,_:&Self::State,_:mouse::Cursor,bounds:Rectangle)->Primitive{
        let size=bounds.size();
        crate::mesh_async::enqueue(&self.0,size);
        let vertices=self.0.gpu_frame_cache.lock()
            .unwrap_or_else(|e|e.into_inner())
            .as_ref().filter(|(cached,_)|*cached==size)
            .map(|(_,vertices)|Arc::clone(vertices))
            .unwrap_or_else(||Arc::new(Vec::new()));
        let atlas=self.0.texture_bank.as_ref().and_then(|bank|bank.atlas()).cloned();
        Primitive{vertices,atlas,quality:self.0.gpu_quality}
    }
}
pub(crate) struct Pipeline{
    opaque:wgpu::RenderPipeline,
    transparent:wgpu::RenderPipeline,
    composite:wgpu::RenderPipeline,
    buffer:wgpu::Buffer,
    capacity:u64,
    count:u32,
    last_mesh:Option<(usize,u64)>,
    atlas:Option<Arc<TextureAtlas>>,
    atlas_texture:wgpu::Texture,
    atlas_layout:wgpu::BindGroupLayout,
    nearest:wgpu::Sampler,
    linear:wgpu::Sampler,
    atlas_bind_group:wgpu::BindGroup,
    quality:Quality,
    offscreen_color:wgpu::Texture,
    offscreen_depth:wgpu::Texture,
    offscreen_size:(u32,u32),
    composite_bind_group:wgpu::BindGroup,
}
fn mesh_fingerprint(bytes:&[u8])->(usize,u64){
    let mut hasher=std::collections::hash_map::DefaultHasher::new();
    hasher.write(bytes);
    (bytes.len(),hasher.finish())
}
fn should_upload_mesh(previous:Option<(usize,u64)>,current:(usize,u64),
    buffer_was_resized:bool)->bool{
    buffer_was_resized||previous!=Some(current)
}
fn make_texture(device:&wgpu::Device,width:u32,height:u32)->wgpu::Texture{
    device.create_texture(&wgpu::TextureDescriptor{
        label:Some("MCEdit2 read-only block atlas"),
        size:wgpu::Extent3d{width,height,depth_or_array_layers:1},
        mip_level_count:1,sample_count:1,
        dimension:wgpu::TextureDimension::D2,
        format:wgpu::TextureFormat::Rgba8Unorm,
        usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,
        view_formats:&[],
    })
}
fn make_sampler(device:&wgpu::Device,filter:wgpu::FilterMode)->wgpu::Sampler{
    device.create_sampler(&wgpu::SamplerDescriptor{
        label:Some("MCEdit2 atlas sampler"),
        mag_filter:filter,min_filter:filter,
        address_mode_u:wgpu::AddressMode::ClampToEdge,
        address_mode_v:wgpu::AddressMode::ClampToEdge,
        ..Default::default()
    })
}
fn make_binding(device:&wgpu::Device,layout:&wgpu::BindGroupLayout,
    texture:&wgpu::Texture,sampler:&wgpu::Sampler)->wgpu::BindGroup{
    let view=texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor{
        label:Some("MCEdit2 atlas binding"),layout,
        entries:&[
            wgpu::BindGroupEntry{binding:0,resource:wgpu::BindingResource::TextureView(&view)},
            wgpu::BindGroupEntry{binding:1,resource:wgpu::BindingResource::Sampler(sampler)},
        ],
    })
}
const COLOR_FORMAT:wgpu::TextureFormat=wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT:wgpu::TextureFormat=wgpu::TextureFormat::Depth24Plus;
/// Keep render targets within guaranteed adapter texture dimensions.
/// These are display-only buffers; never inferred source world content.
const MAX_OFFSCREEN_EDGE:u32=4096;
fn make_offscreen_color(device:&wgpu::Device,width:u32,height:u32)->wgpu::Texture{
    device.create_texture(&wgpu::TextureDescriptor{
        label:Some("MCEdit2 offscreen voxel color"),
        size:wgpu::Extent3d{width,height,depth_or_array_layers:1},
        mip_level_count:1,sample_count:1,
        dimension:wgpu::TextureDimension::D2,format:COLOR_FORMAT,
        usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats:&[],
    })
}
fn make_offscreen_depth(device:&wgpu::Device,width:u32,height:u32)->wgpu::Texture{
    device.create_texture(&wgpu::TextureDescriptor{
        label:Some("MCEdit2 offscreen voxel Z buffer"),
        size:wgpu::Extent3d{width,height,depth_or_array_layers:1},
        mip_level_count:1,sample_count:1,
        dimension:wgpu::TextureDimension::D2,format:DEPTH_FORMAT,
        usage:wgpu::TextureUsages::RENDER_ATTACHMENT,view_formats:&[],
    })
}
fn target_dimensions(bounds:&Rectangle)->(u32,u32){
    ((bounds.width.ceil().max(1.0) as u32).min(MAX_OFFSCREEN_EDGE),
     (bounds.height.ceil().max(1.0) as u32).min(MAX_OFFSCREEN_EDGE))
}
fn scene_pipeline(device:&wgpu::Device,module:&wgpu::ShaderModule,
    layout:&wgpu::PipelineLayout,fragment:&str,depth_write:bool)
    ->wgpu::RenderPipeline{
    const ATTRS:[wgpu::VertexAttribute;5]=wgpu::vertex_attr_array![
        0=>Float32x2,1=>Float32,2=>Float32x4,3=>Float32x2,4=>Uint32
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{
        label:Some("MCEdit2 depth-tested original face stage"),
        layout:Some(layout),
        vertex:wgpu::VertexState{
            module,entry_point:Some("vs_main"),
            buffers:&[wgpu::VertexBufferLayout{
                array_stride:std::mem::size_of::<Vertex>() as u64,
                step_mode:wgpu::VertexStepMode::Vertex,attributes:&ATTRS,
            }],
            compilation_options:wgpu::PipelineCompilationOptions::default(),
        },
        fragment:Some(wgpu::FragmentState{
            module,entry_point:Some(fragment),
            targets:&[Some(wgpu::ColorTargetState{
                format:COLOR_FORMAT,
                blend:if depth_write{None}else{Some(wgpu::BlendState{
                    color:wgpu::BlendComponent{
                        src_factor:wgpu::BlendFactor::One,
                        dst_factor:wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation:wgpu::BlendOperation::Add,
                    },
                    alpha:wgpu::BlendComponent{
                        src_factor:wgpu::BlendFactor::One,
                        dst_factor:wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation:wgpu::BlendOperation::Add,
                    },
                })},
                write_mask:wgpu::ColorWrites::ALL,
            })],
            compilation_options:wgpu::PipelineCompilationOptions::default(),
        }),
        primitive:wgpu::PrimitiveState{
            topology:wgpu::PrimitiveTopology::TriangleList,cull_mode:None,
            ..Default::default()
        },
        depth_stencil:Some(wgpu::DepthStencilState{
            format:DEPTH_FORMAT,depth_write_enabled:depth_write,
            depth_compare:wgpu::CompareFunction::LessEqual,
            stencil:wgpu::StencilState::default(),
            bias:wgpu::DepthBiasState::default(),
        }),
        multisample:wgpu::MultisampleState::default(),
        multiview:None,cache:None,
    })
}
impl shader::Pipeline for Pipeline{
    fn new(device:&wgpu::Device,queue:&wgpu::Queue,format:wgpu::TextureFormat)->Self{
        let source=device.create_shader_module(wgpu::ShaderModuleDescriptor{
            label:Some("MCEdit2 original-voxel depth and composition"),
            source:wgpu::ShaderSource::Wgsl(Cow::Borrowed(WGSL)),
        });
        let atlas_layout=device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{
            label:Some("MCEdit2 source atlas and compositing layout"),
            entries:&[
                wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::FRAGMENT,
                    ty:wgpu::BindingType::Texture{
                        sample_type:wgpu::TextureSampleType::Float{filterable:true},
                        view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
                wgpu::BindGroupLayoutEntry{binding:1,visibility:wgpu::ShaderStages::FRAGMENT,
                    ty:wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),count:None},
            ],
        });
        let layout=device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{
            label:Some("MCEdit2 GPU source and depth compositing layout"),
            bind_group_layouts:&[&atlas_layout],push_constant_ranges:&[],
        });
        let opaque=scene_pipeline(device,&source,&layout,"fs_opaque",true);
        let transparent=scene_pipeline(device,&source,&layout,"fs_transparent",false);
        let composite=device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{
            label:Some("MCEdit2 composite depth-tested voxel scene onto iced"),
            layout:Some(&layout),
            vertex:wgpu::VertexState{
                module:&source,entry_point:Some("vs_composite"),
                buffers:&[],
                compilation_options:wgpu::PipelineCompilationOptions::default(),
            },
            fragment:Some(wgpu::FragmentState{
                module:&source,entry_point:Some("fs_composite"),
                targets:&[Some(wgpu::ColorTargetState{
                    format,blend:Some(wgpu::BlendState{
                    color:wgpu::BlendComponent{
                        src_factor:wgpu::BlendFactor::One,
                        dst_factor:wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation:wgpu::BlendOperation::Add,
                    },
                    alpha:wgpu::BlendComponent{
                        src_factor:wgpu::BlendFactor::One,
                        dst_factor:wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation:wgpu::BlendOperation::Add,
                    },
                }),
                    write_mask:wgpu::ColorWrites::ALL,
                })],
                compilation_options:wgpu::PipelineCompilationOptions::default(),
            }),
            primitive:wgpu::PrimitiveState{
                topology:wgpu::PrimitiveTopology::TriangleList,cull_mode:None,
                ..Default::default()
            },
            depth_stencil:None,
            multisample:wgpu::MultisampleState::default(),
            multiview:None,cache:None,
        });
        let capacity=std::mem::size_of::<Vertex>() as u64*6;
        let buffer=device.create_buffer(&wgpu::BufferDescriptor{
            label:Some("MCEdit2 native GPU source mesh"),
            size:capacity,
            usage:wgpu::BufferUsages::VERTEX|wgpu::BufferUsages::COPY_DST,
            mapped_at_creation:false,
        });
        let atlas_texture=make_texture(device,1,1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo{texture:&atlas_texture,mip_level:0,
                origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},
            &[255,255,255,255],wgpu::TexelCopyBufferLayout{
                offset:0,bytes_per_row:Some(4),rows_per_image:Some(1)},
            wgpu::Extent3d{width:1,height:1,depth_or_array_layers:1});
        let nearest=make_sampler(device,wgpu::FilterMode::Nearest);
        let linear=make_sampler(device,wgpu::FilterMode::Linear);
        let atlas_bind_group=make_binding(device,&atlas_layout,&atlas_texture,&nearest);
        let offscreen_color=make_offscreen_color(device,1,1);
        let offscreen_depth=make_offscreen_depth(device,1,1);
        let composite_bind_group=make_binding(device,&atlas_layout,&offscreen_color,&nearest);
        Self{opaque,transparent,composite,buffer,capacity,count:0,
            last_mesh:None,atlas:None,atlas_texture,atlas_layout,nearest,
            linear,atlas_bind_group,quality:Quality::Balanced,
            offscreen_color,offscreen_depth,offscreen_size:(1,1),
            composite_bind_group}
    }
}
impl shader::Primitive for Primitive{
    type Pipeline=Pipeline;
    fn prepare(&self,pipeline:&mut Pipeline,device:&wgpu::Device,queue:&wgpu::Queue,
        bounds:&Rectangle,_:&Viewport){
        let bytes=bytemuck::cast_slice(self.vertices.as_slice());
        let needed=bytes.len() as u64;
        let resized_mesh=needed>pipeline.capacity;
        if resized_mesh{
            pipeline.capacity=needed.next_power_of_two();
            pipeline.buffer=device.create_buffer(&wgpu::BufferDescriptor{
                label:Some("MCEdit2 resized original source GPU mesh"),
                size:pipeline.capacity,
                usage:wgpu::BufferUsages::VERTEX|wgpu::BufferUsages::COPY_DST,
                mapped_at_creation:false,
            });
        }
        let fingerprint=mesh_fingerprint(bytes);
        let changed_mesh=should_upload_mesh(pipeline.last_mesh,fingerprint,resized_mesh);
        if changed_mesh{
            if !bytes.is_empty(){queue.write_buffer(&pipeline.buffer,0,bytes);}
            pipeline.last_mesh=Some(fingerprint);
        }
        pipeline.count=self.vertices.len() as u32;
        let changed_atlas=match (&pipeline.atlas,&self.atlas){
            (Some(old),Some(new))=>!Arc::ptr_eq(old,new),
            (None,None)=>false,_=>true,
        };
        if changed_atlas{
            let (width,height,data):(_,_,&[u8])=self.atlas.as_ref()
                .map(|a|(a.width,a.height,a.rgba.as_slice()))
                .unwrap_or((1,1,&[255,255,255,255]));
            let texture=make_texture(device,width,height);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo{texture:&texture,mip_level:0,
                    origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},
                data,wgpu::TexelCopyBufferLayout{
                    offset:0,bytes_per_row:Some(width*4),rows_per_image:Some(height)},
                wgpu::Extent3d{width,height,depth_or_array_layers:1});
            pipeline.atlas_texture=texture;
            pipeline.atlas=self.atlas.clone();
        }
        let changed_quality=pipeline.quality!=self.quality;
        if changed_atlas||changed_quality{
            pipeline.quality=self.quality;
            let sampler=match self.quality{
                Quality::Detailed=>&pipeline.linear,
                _=>&pipeline.nearest,
            };
            pipeline.atlas_bind_group=make_binding(device,&pipeline.atlas_layout,
                &pipeline.atlas_texture,sampler);
        }
        let target=target_dimensions(bounds);
        let resized_view=target!=pipeline.offscreen_size;
        if resized_view{
            pipeline.offscreen_color=make_offscreen_color(device,target.0,target.1);
            pipeline.offscreen_depth=make_offscreen_depth(device,target.0,target.1);
            pipeline.offscreen_size=target;
            pipeline.composite_bind_group=make_binding(device,&pipeline.atlas_layout,
                &pipeline.offscreen_color,&pipeline.nearest);
        }
        // Re-render the intermediate color and Z targets only if the source
        // faces, source PNG atlas, quality sampler or viewport have changed.
        if !changed_mesh&&!changed_atlas&&!changed_quality&&!resized_view{
            return;
        }
        let color=pipeline.offscreen_color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth=pipeline.offscreen_depth.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder=device.create_command_encoder(&wgpu::CommandEncoderDescriptor{
            label:Some("MCEdit2 original-voxel opaque and translucent depth passes"),
        });
        // Opaque and binary-alpha-cutout fragments populate Z. The exact
        // source model triangles can intersect without painter-order errors.
        {
            let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor{
                label:Some("Original opaque model Z prepass and color"),
                color_attachments:&[Some(wgpu::RenderPassColorAttachment{
                    view:&color,resolve_target:None,depth_slice:None,
                    ops:wgpu::Operations{
                        load:wgpu::LoadOp::Clear(wgpu::Color{
                            r:0.0,g:0.0,b:0.0,a:0.0
                        }),store:wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment{
                    view:&depth,
                    depth_ops:Some(wgpu::Operations{
                        load:wgpu::LoadOp::Clear(1.0),
                        store:wgpu::StoreOp::Store,
                    }),stencil_ops:None,
                }),
                timestamp_writes:None,occlusion_query_set:None,
            });
            if pipeline.count>0{
                pass.set_pipeline(&pipeline.opaque);
                pass.set_vertex_buffer(0,pipeline.buffer.slice(..));
                pass.set_bind_group(0,&pipeline.atlas_bind_group,&[]);
                pass.draw(0..pipeline.count,0..1);
            }
        }
        // Translucent faces remain sorted far-to-near. They depth-test
        // against opaque material but MUST NOT overwrite the depth target.
        {
            let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor{
                label:Some("Original translucent source faces over known opaque Z"),
                color_attachments:&[Some(wgpu::RenderPassColorAttachment{
                    view:&color,resolve_target:None,depth_slice:None,
                    ops:wgpu::Operations{
                        load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment{
                    view:&depth,
                    depth_ops:Some(wgpu::Operations{
                        load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Discard,
                    }),stencil_ops:None,
                }),
                timestamp_writes:None,occlusion_query_set:None,
            });
            if pipeline.count>0{
                pass.set_pipeline(&pipeline.transparent);
                pass.set_vertex_buffer(0,pipeline.buffer.slice(..));
                pass.set_bind_group(0,&pipeline.atlas_bind_group,&[]);
                pass.draw(0..pipeline.count,0..1);
            }
        }
        queue.submit(Some(encoder.finish()));
    }
    fn draw(&self,pipeline:&Pipeline,pass:&mut wgpu::RenderPass<'_>)->bool{
        // iced owns the actual swapchain pass. Only sample the completed
        // depth-tested offscreen result into the shader widget's viewport.
        pass.set_pipeline(&pipeline.composite);
        pass.set_bind_group(0,&pipeline.composite_bind_group,&[]);
        pass.draw(0..6,0..1);
        true
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn identical_projected_mesh_skips_expensive_gpu_buffer_upload(){
        let first=mesh_fingerprint(&[3,1,4,1,5,9]);
        let same=mesh_fingerprint(&[3,1,4,1,5,9]);
        let changed=mesh_fingerprint(&[3,1,4,1,5,8]);
        assert_eq!(first,same);
        assert_ne!(first,changed);
        assert!(!should_upload_mesh(Some(first),same,false));
        assert!(should_upload_mesh(Some(first),changed,false));
        assert!(should_upload_mesh(Some(first),same,true));
        assert!(should_upload_mesh(None,same,false));
    }
    #[test]fn vertex_world_depth_maps_nearest_to_zero_and_farthest_to_one(){
        let mut v=Vertex{
            xy:[50.0,25.0],depth:10.0,
            rgba:[1.0,0.0,0.0,1.0],uv:[0.5,0.5],mode:0
        };
        to_clip(&mut v,Size::new(100.0,50.0),-10.0,10.0);
        assert_eq!(v.xy,[0.0,0.0]);
        assert_eq!(v.depth,0.0);
        let mut far=Vertex{depth:-10.0,..v};
        to_clip(&mut far,Size::new(100.0,50.0),-10.0,10.0);
        assert_eq!(far.depth,1.0);
        let mut flat=Vertex{depth:7.0,..v};
        to_clip(&mut flat,Size::new(200.0,100.0),7.0,7.0);
        assert_eq!(flat.depth,0.5);
        assert_eq!(flat.xy,[-1.0,1.0]);
    }
    #[test]fn depth_pipeline_excludes_blend_from_opaque_and_skips_depth_writes_for_blend(){
        assert!(WGSL.contains("if input.mode==2u{discard;}"));
        assert!(WGSL.contains("if input.mode!=2u{discard;}"));
        assert!(WGSL.contains("input.mode==1u && rgba.a<0.5"));
        assert!(WGSL.contains("vs_composite"));
        assert!(WGSL.contains("rgba.rgb*rgba.a"));
        assert_eq!(COLOR_FORMAT,wgpu::TextureFormat::Rgba8Unorm);
        assert_eq!(DEPTH_FORMAT,wgpu::TextureFormat::Depth24Plus);
    }
    #[test]fn render_target_dimensions_are_capped_and_nonzero(){
        let rect=Rectangle{x:0.0,y:0.0,width:750.2,height:360.5};
        assert_eq!(target_dimensions(&rect),(751,361));
        let empty=Rectangle{x:0.0,y:0.0,width:0.0,height:0.0};
        assert_eq!(target_dimensions(&empty),(1,1));
        let massive=Rectangle{x:0.0,y:0.0,width:9000.0,height:9000.0};
        assert_eq!(target_dimensions(&massive),(4096,4096));
    }
}
