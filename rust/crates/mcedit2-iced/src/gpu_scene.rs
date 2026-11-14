//! Native iced 0.14 wgpu shader renderer of the source-voxel mesh.
//! Projected face geometry and hit tests stay shared with the Canvas path.
//! Shader avoids an application build, custom window and independent GPU.
use crate::gpu_mesh::{self, Quality, Vertex};
use crate::isometric::IsometricSurface;
use crate::material_mode::Pass;
use crate::textures::TextureAtlas;
use crate::Message;
use iced::wgpu;
use iced::widget::shader::{self, Viewport};
use iced::{mouse, Rectangle, Size};
use std::borrow::Cow;
use std::sync::Arc;
#[path = "gpu_atlas.rs"]
mod atlas;
#[path = "gpu_commands.rs"]
mod commands;
#[path = "gpu_quad_encoding.rs"]
mod encoding;
#[path = "gpu_material_hints.rs"]
mod material_hints;
#[path = "gpu_transparent_residency.rs"]
mod residency;
#[path = "gpu_transparent.rs"]
mod transparent;
#[path = "gpu_upload.rs"]
mod upload;
#[path = "gpu_world_mesh.rs"]
mod world;
use crate::visibility_index as visibility;

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
    @location(8) light: u32,
};
struct Camera {
    position:vec4<f32>,
    right:vec4<f32>,
    up:vec4<f32>,
    forward:vec4<f32>,
    projection:vec4<f32>,
};
@group(0) @binding(2) var<uniform> camera:Camera;
@group(1) @binding(0) var<uniform> chunk_offset:vec4<f32>;
struct RegionOrigin { origin:vec4<i32>, frame:vec4<i32> };
@group(1) @binding(9) var<uniform> region_origin:RegionOrigin;
// MCEDIT2_TRANSPARENT_RESIDENCY
struct WInput {
    @location(0) p0:vec3<f32>, @location(1) p1:vec3<f32>,
    @location(2) p2:vec3<f32>, @location(3) p3:vec3<f32>,
    @location(4) uv0:vec2<f32>, @location(5) uv1:vec2<f32>,
    @location(6) uv2:vec2<f32>, @location(7) uv3:vec2<f32>,
    @location(8) c0:vec4<f32>, @location(9) c1:vec4<f32>,
    @location(10) c2:vec4<f32>, @location(11) c3:vec4<f32>,
    @location(12) mode:u32, @location(13) atlas_page:u32,
    @location(14) light:u32,
};
struct VOutput {
    @invariant @builtin(position) position: vec4<f32>,
    @location(0) rgba: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) mode: u32,
    @location(3) @interpolate(flat) atlas_page: u32,
};
// 区域 X/Z 余数各七位；先还原整数源原点，再精确求有符号距离。
fn region_delta(packed:u32)->vec3<f32> {
    let origin=region_origin.origin.xyz+vec3<i32>(i32(packed&127u),0,i32((packed>>7u)&127u));
    let base=region_origin.frame.xyz;
    let negative=origin<base;
    let a=bitcast<vec3<u32>>(origin);
    let b=bitcast<vec3<u32>>(base);
    let magnitude=select(a-b,b-a,negative);
    return select(vec3<f32>(magnitude),-vec3<f32>(magnitude),negative);
}
// 朝向相机仅改变显示位置；源面和四角材质保留同一身份及缓冲步幅。
fn billboard_point(point:vec3<f32>,anchor:vec3<f32>,mode:u32)->vec3<f32> {
    if (mode&262144u)==0u {return point;}
    let local=point-anchor;
    if (mode&234881024u)==234881024u {
        let right=vec3<f32>(camera.right.x,0.0,camera.right.z);
        return anchor+right*local.x+vec3<f32>(0.0,1.0,0.0)*local.y+vec3<f32>(-right.z,0.0,right.x)*local.z;
    }
    return anchor+camera.right.xyz*local.x+camera.up.xyz*local.y+camera.forward.xyz*local.z;
}
fn guardian_eye_point(point:vec3<f32>,encoded:bool,offset:vec3<f32>,eye:vec3<f32>,axis_x:vec3<f32>,axis_y:vec3<f32>,look:vec3<f32>)->vec3<f32> {
    if !encoded { return point; }
    let viewer=camera.position.xyz-offset;
    let delta=eye.xz-viewer.xz;
    let distance=length(delta);
    var direction=vec2<f32>(0.0);
    if distance>=0.0001 { direction=delta/distance; }
    // 两版 Vec3.yRot(π/2) 的原生三角表值。
    let rotated=vec2<f32>(direction.x*1.2246469e-16+direction.y,direction.y*1.2246469e-16-direction.x);
    let d=dot(look.xz,rotated);
    let x=sqrt(abs(d))*2.0*sign(d);
    let y=select(1.0,0.0,viewer.y>eye.y);
    return point+axis_x*x+axis_y*y;
}
fn project_at(xyz:vec3<f32>,mode:u32,offset:vec3<f32>)->vec4<f32>{
        let view_scale=select(1.0,0.999755859375,(mode&64u)!=0u);
        let relative=(xyz+offset-camera.position.xyz)*view_scale;
        let distance=dot(relative,camera.forward.xyz);
        let near=camera.projection.z;
        let far=camera.projection.w;
        if camera.position.w==1.0 {
            return vec4<f32>(
                dot(relative,camera.right.xyz)*camera.projection.x+camera.right.w,
                dot(relative,camera.up.xyz)*camera.projection.y+camera.up.w,
                (distance-near)/(far-near), 1.0);
        }
        return vec4<f32>(
            dot(relative,camera.right.xyz)*camera.projection.x,
            dot(relative,camera.up.xyz)*camera.projection.y,
            far/(far-near)*distance-far*near/(far-near),
            distance);
}
fn project_world(xyz:vec3<f32>,mode:u32)->vec4<f32> {
    return project_at(xyz,mode,chunk_offset.xyz);
}
fn visibility_at(anchor:vec3<f32>,mode:u32,offset:vec3<f32>)->f32 {
    var distance=4096.0;
    if (mode&536870912u)!=0u { distance=256.0; }
    else if (mode&524288u)==0u { return 1.0; }
    let delta=anchor+offset-camera.position.xyz;
    return select(0.0,1.0,dot(delta,delta)<distance);
}
fn material_visibility(anchor:vec3<f32>,mode:u32)->f32 {
    return visibility_at(anchor,mode,chunk_offset.xyz);
}
struct Lightmap { control:vec4<u32>, colors:array<vec4<f32>,256> };
@group(0) @binding(7) var<uniform> lightmap:Lightmap;
fn material_color(color:vec4<f32>,mode:u32,light:u32)->vec4<f32> {
    var rgba=color;
    if color.a == -1.0 { rgba=vec4<f32>(1.0); }
    else if (mode&1610612736u)!=0u { rgba=vec4<f32>(1.0,1.0,1.0,color.a); }
    if lightmap.control.x!=0u && (light&2147483647u)!=0u {
        let index=(light&15u)*16u+((light>>4u)&15u);
        let next_index=min((light&15u)+1u,15u)*16u+((light>>4u)&15u);
        let fraction=f32((light>>12u)&15u)/16.0;
        let illumination=mix(lightmap.colors[index].rgb,lightmap.colors[next_index].rgb,fraction);
        rgba=vec4<f32>(rgba.rgb*illumination,rgba.a);
    }
    return rgba;
}

fn world_vertex(input:WInput,vertex:u32,offset:vec3<f32>)->VOutput{
    let corner=vertex;
    let positions=array<vec3<f32>,4>(input.p0,input.p1,input.p2,input.p3);
    let uvs=array<vec2<f32>,4>(input.uv0,input.uv1,input.uv2,input.uv3);
    let colors=array<vec4<f32>,4>(input.c0,input.c1,input.c2,input.c3);
    var out:VOutput;
    var point=positions[corner];
    if (input.mode&262144u)!=0u && corner==3u {point=input.p0+input.p2-input.p1;}
    point=guardian_eye_point(point,input.c0.a == -1.0,offset,input.c0.rgb,input.c1.rgb,input.c2.rgb,input.c3.rgb);
    out.position=project_at(billboard_point(point,input.p3,input.mode),input.mode,offset);
    let anchor=select(colors[corner].rgb,input.p3,(input.mode&524288u)!=0u);
    out.rgba=material_color(colors[corner],input.mode,input.light); out.rgba.a*=visibility_at(anchor,input.mode,offset); out.uv=uvs[corner];
    out.mode=input.mode; out.atlas_page=input.atlas_page;
    return out;
}
@vertex fn vs_world(input:WInput,@builtin(vertex_index) vertex:u32)->VOutput {
    return world_vertex(input,vertex,chunk_offset.xyz);
}
@vertex fn vs_world_region(input:WInput,@location(15) origin_xz:u32,@builtin(vertex_index) vertex:u32)->VOutput {
    return world_vertex(input,vertex,region_delta(origin_xz));
}
struct CInput {
    @location(0) minimum:vec3<f32>, @location(1) maximum:vec3<f32>,
    @location(2) uv_minimum:vec2<f32>, @location(3) uv_maximum:vec2<f32>,
    @location(4) rgba:vec4<f32>, @location(5) corners:u32,
    @location(6) mode:u32, @location(7) atlas_page:u32,
    @location(8) light:u32,
};
fn compact_position(input:CInput,corner:u32)->vec3<f32> {
    return select(input.minimum,input.maximum,(vec3<u32>(corner)&vec3<u32>(1u,2u,4u))!=vec3<u32>(0u));
}
fn compact_uv(input:CInput,corner:u32)->vec2<f32> {
    return select(input.uv_minimum,input.uv_maximum,(vec2<u32>(corner)&vec2<u32>(8u,16u))!=vec2<u32>(0u));
}
struct FanPoint { xyz:vec3<f32>, uv:vec2<f32> };
// 单位边界顶点与未合并网格完全相同，光栅化不会在长边上引入 T 接缝。
fn fan_point(input:CInput,ordinal:u32)->FanPoint {
    let extent=input.maximum-input.minimum;
    let half_perimeter=extent.x+extent.y+extent.z;
    let first=dot(abs(compact_position(input,input.corners>>5u)-compact_position(input,input.corners)),vec3<f32>(1.0));
    let second=half_perimeter-first;
    var step=ordinal%u32(half_perimeter*2.0);
    var edge=0u;
    if step>=u32(half_perimeter+first) {
        edge=3u;step-=u32(half_perimeter+first);
    } else if step>=u32(half_perimeter) {
        edge=2u;step-=u32(half_perimeter);
    } else if step>=u32(first) {
        edge=1u;step-=u32(first);
    }
    let c0=input.corners>>(edge*5u);
    let c1=input.corners>>(((edge+1u)%4u)*5u);
    let p0=compact_position(input,c0);let p1=compact_position(input,c1);
    let uv0=compact_uv(input,c0);let uv1=compact_uv(input,c1);
    let length=select(first,second,(edge&1u)!=0u);
    var out:FanPoint;
    out.xyz=p0+sign(p1-p0)*f32(step);
    out.uv=uv0+(uv1-uv0)/length*f32(step);
    return out;
}
fn compact_vertex(input:CInput,vertex:u32,offset:vec3<f32>)->VOutput{
    var xyz=(input.minimum+input.maximum)*0.5;
    var uv=(input.uv_minimum+input.uv_maximum)*0.5;
    if (input.mode&131072u)!=0u {
        let extent=input.maximum-input.minimum;
        let perimeter=u32(2.0*(extent.x+extent.y+extent.z));
        if vertex>0u && vertex<=perimeter+1u {
            let point=fan_point(input,vertex-1u);
            xyz=point.xyz;uv=point.uv;
        }
    } else {
        let corner=input.corners>>(vertex*5u);
        xyz=compact_position(input,corner);
        uv=compact_uv(input,corner);
    }
    var out:VOutput;
    out.position=project_at(xyz,input.mode,offset);
    out.rgba=material_color(input.rgba,input.mode,input.light);
    out.rgba.a*=visibility_at(input.rgba.rgb,input.mode,offset);
    out.uv=uv; out.mode=input.mode; out.atlas_page=input.atlas_page;
    return out;
}
@vertex fn vs_world_compact(input:CInput,@builtin(vertex_index) vertex:u32)->VOutput {
    return compact_vertex(input,vertex,chunk_offset.xyz);
}
@vertex fn vs_world_region_compact(input:CInput,@location(9) origin_xz:u32,@builtin(vertex_index) vertex:u32)->VOutput {
    return compact_vertex(input,vertex,region_delta(origin_xz));
}
@vertex fn vs_world_indexed(@location(0) instance:vec2<u32>,@builtin(vertex_index) vertex:u32)->VOutput{
    let corner=transparent_corner(instance.x,vertex);
    var out:VOutput;
    out.position=project_world(billboard_point(corner.position,corner.anchor,corner.mode)+transparent_offsets[instance.y].xyz,corner.mode);
    out.uv=corner.uv;out.rgba=material_color(corner.rgba,corner.mode,corner.light);
    let anchor=select(corner.rgba.rgb,corner.anchor,(corner.mode&524288u)!=0u);
    out.rgba.a*=material_visibility(anchor+transparent_offsets[instance.y].xyz,corner.mode);
    out.mode=corner.mode;out.atlas_page=corner.atlas_page;
    return out;
}
@vertex fn vs_main(input:VInput)->VOutput{
    var out:VOutput;
    if input.world_mode==1u {
        out.position=project_world(billboard_point(input.xyz,vec3<f32>(input.xy,input.depth),input.mode),input.mode);
    } else {
        out.position=vec4<f32>(input.xy,input.depth,1.0);
    }
    out.rgba=material_color(input.rgba,input.mode,input.light);
    if input.world_mode==1u {
        let anchor=select(input.rgba.rgb,vec3<f32>(input.xy,input.depth),(input.mode&524288u)!=0u);
        out.rgba.a*=material_visibility(anchor,input.mode);
    }
    out.uv=input.uv;
    out.mode=input.mode;
    out.atlas_page=input.atlas_page;
    return out;
}
@group(0) @binding(0) var block_atlas:texture_2d_array<f32>;
@group(0) @binding(1) var block_sampler:sampler;
struct MipMaterial {base_uv:vec4<f32>,levels:array<vec4<f32>,16>,coverage:array<vec4<f32>,16>,count:u32,p0:u32,p1:u32,p2:u32};
@group(0) @binding(3) var mip_aux:texture_2d_array<f32>;
@group(0) @binding(4) var mip_owner:texture_2d_array<u32>;
@group(0) @binding(5) var<storage,read> mip_materials:array<MipMaterial>;
struct AtlasPageInfo {base:vec4<f32>,aux:vec4<f32>,owner:vec4<u32>,materials:vec4<u32>};
struct AtlasPageTable {first:u32,count:u32,padding:vec2<u32>,pages:array<AtlasPageInfo>};
@group(0) @binding(6) var<storage,read> atlas_pages:AtlasPageTable;
// 只规范化距整数 texel 边界数个 f32 ULP 的值，避免不同三角划分选到两侧像素。
fn stable_uv(uv:vec2<f32>,dimensions:vec2<f32>,uncertainty:vec2<f32>)->vec2<f32> {
    let pixel=uv*dimensions;
    let nearest=round(pixel);
    let error=max(abs(pixel),vec2<f32>(1.0))*0.000000476837158203125+uncertainty;
    return select(pixel,nearest,abs(pixel-nearest)<=error)/dimensions;
}
fn sample_base(page:AtlasPageInfo,uv:vec2<f32>,error:vec2<f32>)->vec4<f32>{
    let half_texel=vec2<f32>(0.5)/page.base.zw;
    let source_uv=clamp(uv,half_texel,vec2<f32>(1.0)-half_texel);
    return textureSampleLevel(block_atlas,block_sampler,stable_uv(source_uv,page.base.zw,error)*page.base.xy,i32(page.owner.w),0.0);
}
fn coverage_scale(c:vec4<f32>,mode:u32)->f32 {
    if (mode&3u)!=1u{return 1.0;}
    if (mode&32u)!=0u{return c.y;}
    if (mode&16u)!=0u {let cutoff=(mode>>8u)&255u;if cutoff==127u || cutoff==128u{return c.x;}if cutoff==25u || cutoff==26u{return c.y;}return 1.0;}
    return c.x;
}
// SimpleTexture 只在自身材质矩形内寻址；全图集共用 repeat sampler 会串入其它素材。
fn native_address(pixel:vec2<i32>,size:vec2<i32>,clamped:bool)->vec2<i32>{
    return select(((pixel%size)+size)%size,clamp(pixel,vec2<i32>(0),size-vec2<i32>(1)),clamped);
}
fn native_texel(page:AtlasPageInfo,pixel:vec2<i32>,base:bool)->vec4<f32>{
    if base {return textureLoad(block_atlas,pixel,i32(page.owner.w),0);}
    return textureLoad(mip_aux,pixel,i32(page.owner.w),0);
}
fn sample_native(page:AtlasPageInfo,rect:vec4<f32>,local:vec2<f32>,flags:u32,base:bool)->vec4<f32>{
    let dimensions=select(page.aux.zw,page.base.zw,base);
    let start=vec2<i32>(round(rect.xy*dimensions-vec2<f32>(0.5)));
    let size=vec2<i32>(round((rect.zw-rect.xy)*dimensions))+vec2<i32>(1);
    let blur=(flags&1u)!=0u;let clamped=(flags&2u)!=0u;
    let p=stable_uv(local,vec2<f32>(size),vec2<f32>(0.0))*vec2<f32>(size)-select(vec2<f32>(0.0),vec2<f32>(0.5),blur);
    let pixel=vec2<i32>(floor(p));
    let a=native_texel(page,start+native_address(pixel,size,clamped),base);
    if !blur {return a;}
    let b=native_texel(page,start+native_address(pixel+vec2<i32>(1,0),size,clamped),base);
    let c=native_texel(page,start+native_address(pixel+vec2<i32>(0,1),size,clamped),base);
    let d=native_texel(page,start+native_address(pixel+vec2<i32>(1,1),size,clamped),base);
    return mix(mix(a,b,fract(p.x)),mix(c,d,fract(p.x)),fract(p.y));
}
fn sample_material_level(page:AtlasPageInfo,id:u32,level:u32,uv:vec2<f32>,local:vec2<f32>,mode:u32,error:vec2<f32>)->vec4<f32>{
    let native=(mode&1048576u)!=0u || (mip_materials[id].p0&8u)!=0u;
    if level==0u {
        if native {return sample_native(page,mip_materials[id].base_uv,local,mip_materials[id].p0,true);}
        return sample_base(page,uv,error*abs(mip_materials[id].base_uv.zw-mip_materials[id].base_uv.xy)*page.base.zw);
    }
    let rect=mip_materials[id].levels[level-1u];
    var result:vec4<f32>;
    if native {result=sample_native(page,rect,local,mip_materials[id].p0,false);}
    else {result=textureSampleLevel(mip_aux,block_sampler,stable_uv(mix(rect.xy,rect.zw,local),page.aux.zw,error*abs(rect.zw-rect.xy)*page.aux.zw)*page.aux.xy,i32(page.owner.w),0.0);}
    result.a=min(1.0,result.a*coverage_scale(mip_materials[id].coverage[level-1u],mode));
    return result;
}
// 固定全亮度：只采样材质与自身颜色，不叠加世界光照、AO 或方向明暗。
fn texel_color(texel:vec4<f32>,color:vec4<f32>)->vec4<f32>{
    var vertex=color;
    if color.a > -1.0 && color.a < 0.0 {
        if texel.a < 0.1 {discard;}
        vertex.a = -color.a;
    }
    return texel*vertex;
}
fn shade(input:VOutput)->vec4<f32>{
    let hinted=(input.mode&2147483648u)!=0u;
    let repeated=(input.mode&65536u)!=0u;
    let page_index=select(input.atlas_page-atlas_pages.first,input.atlas_page&atlas_pages.padding.y,hinted);
    let invalid_page=(!hinted && input.atlas_page<atlas_pages.first) || page_index>=atlas_pages.count;
    let page=atlas_pages.pages[min(page_index,atlas_pages.count-1u)];
    let dims=page.base.zw;
    var owner=input.atlas_page>>atlas_pages.padding.x;
    let hint_valid=hinted && owner>0u && owner<=page.materials.y && page.materials.x+owner<=arrayLength(&mip_materials);
    var repeat_rect=vec4<f32>(0.0);
    if repeated && hint_valid { repeat_rect=mip_materials[page.materials.x+owner-1u].base_uv; }
    let repeat_extent=repeat_rect.zw-repeat_rect.xy;
    // 重复坐标的导数不经过 fract，跨格边界不产生错误的 mip 级别。
    let scale=select(dims,repeat_extent*dims,repeated);
    let uv_dx=dpdx(input.uv);let uv_dy=dpdy(input.uv);
    let dx=uv_dx*scale;let dy=uv_dy*scale;
    var lod=max(0.0,log2(max(max(length(dx),length(dy)),0.000001)));
    if invalid_page || (repeated && !hint_valid) {return vec4<f32>(1.0,0.0,1.0,1.0);}
    var uv=input.uv;
    let repeat_error=select(vec2<f32>(0.0),max(abs(input.uv),vec2<f32>(1.0))*0.00000095367431640625,repeated);
    let repeat_position=select(input.uv,round(input.uv),abs(input.uv-round(input.uv))<=repeat_error);
    var repeat_local=fract(repeat_position);
    if repeated {
        // 屏幕先 X 后 Y 的无限小位移，遵守原单位面的 top-left 边界归属。
        let direction=select(dy,dx,dx!=vec2<f32>(0.0));
        repeat_local=select(repeat_local,vec2<f32>(1.0),(repeat_local==vec2<f32>(0.0)) & (direction<vec2<f32>(0.0)));
        uv=mix(repeat_rect.xy,repeat_rect.zw,repeat_local);
    }
    if !hinted {
        let pixel=vec2<u32>(clamp(uv*dims,vec2<f32>(0.0),dims-vec2<f32>(1.0)))/page.owner.z;
        owner=textureLoad(mip_owner,vec2<i32>(pixel),i32(page.owner.w),0).r;
    }
    if owner==0u || owner>page.materials.y || page.materials.x+owner>arrayLength(&mip_materials) {
        if (input.mode&1048576u)!=0u {return vec4<f32>(1.0,0.0,1.0,1.0);}
        return texel_color(sample_base(page,uv,repeat_error*abs(repeat_extent)*dims),input.rgba);
    }
    let id=page.materials.x+owner-1u;let count=mip_materials[id].count;
    let base_uv=mip_materials[id].base_uv;let extent=base_uv.zw-base_uv.xy;
    let native=(input.mode&1048576u)!=0u || (mip_materials[id].p0&8u)!=0u;
    var local=select(vec2<f32>(0.5),(uv-base_uv.xy)/max(abs(extent),vec2<f32>(0.00000001)),abs(extent)>vec2<f32>(0.00000001));
    if native {
        let size=round(abs(extent)*dims)+vec2<f32>(1.0);
        let native_scale=select(vec2<f32>(0.0),size/max(abs(extent),vec2<f32>(0.00000001)),abs(extent)>vec2<f32>(0.00000001));
        let native_dx=uv_dx*select(native_scale,size,repeated);let native_dy=uv_dy*select(native_scale,size,repeated);
        lod=max(0.0,log2(max(max(length(native_dx),length(native_dy)),0.000001)));
    } else {local=clamp(local,vec2<f32>(0.0),vec2<f32>(1.0));}
    if (input.mode&128u)!=0u {lod=0.0;}
    let clamped=clamp(lod,0.0,f32(count));let lo=u32(floor(clamped));let hi=min(lo+1u,count);
    let a=sample_material_level(page,id,lo,uv,local,input.mode,repeat_error);
    if hi==lo || fract(clamped)==0.0 {return texel_color(a,input.rgba);}
    let b=sample_material_level(page,id,hi,uv,local,input.mode,repeat_error);
    return texel_color(mix(a,b,fract(clamped)),input.rgba);
}
// 编辑器固定全亮：纹理乘显式材质着色，不采样天空光、方块光或 AO。
// 相机朝向、昼夜和世界光照均不改变亮度，不提供动态光照模式。
fn opaque_color(input:VOutput,front:bool)->vec4<f32>{
    if (input.mode & 3u)==2u{discard;}
    if (input.mode & 4u)!=0u && !front{discard;}
    let rgba=shade(input);
    if (input.mode & 3u)==1u {
        var threshold=0.5;
        if (input.mode & 32u)!=0u {
            threshold=0.1;
        } else if (input.mode & 16u)!=0u {
            threshold=f32((input.mode >> 8u) & 255u)/255.0;
        }
        if rgba.a<threshold {discard;}
        // Alpha masking is binary: accepted texels cover the pixel fully.
        return vec4<f32>(rgba.rgb,1.0);
    }
    if (input.mode & 8u)!=0u {
        // Declared solid / glTF OPAQUE ignore texture/vertex alpha and write full depth.
        return vec4<f32>(rgba.rgb,1.0);
    }
    if rgba.a<=0.0039{discard;}
    return rgba;
}
@fragment fn fs_opaque(input:VOutput,@builtin(front_facing) front:bool)->@location(0) vec4<f32>{return opaque_color(input,front);}
@fragment fn fs_decal(input:VOutput,@builtin(front_facing) front:bool)->@location(0) vec4<f32>{return opaque_color(input,front);}
fn blend_color(input:VOutput,front:bool)->vec4<f32>{
    if (input.mode & 3u)!=2u{discard;}
    if (input.mode & 4u)!=0u && !front{discard;}
    let rgba=shade(input);
    if !(input.rgba.a > -1.0 && input.rgba.a < 0.0) {
        if (input.mode & 32u)!=0u && rgba.a<0.1 {discard;}
        if (input.mode & 16u)!=0u && rgba.a<f32((input.mode >> 8u) & 255u)/255.0 {discard;}
        if rgba.a<=0.0039{discard;}
    }
    // Offscreen color is premultiplied; otherwise compositing the
    // intermediate scene over iced would multiply transparency twice.
    return vec4<f32>(rgba.rgb*rgba.a,rgba.a);
}
@fragment fn fs_transparent(input:VOutput,@builtin(front_facing) front:bool)->@location(0) vec4<f32>{
    if (input.mode&4194304u)!=0u {discard;}
    if (input.mode&268435456u)!=0u {discard;}
    return blend_color(input,front);
}
@fragment fn fs_tripwire(input:VOutput,@builtin(front_facing) front:bool)->@location(0) vec4<f32>{
    if (input.mode&4194304u)!=0u {discard;}
    if (input.mode&268435456u)==0u {discard;}
    return blend_color(input,front);
}
fn additive_color(input:VOutput,front:bool)->vec4<f32>{
    if (input.mode&4194304u)==0u {discard;}
    if (input.mode&4u)!=0u && !front {discard;}
    let rgba=shade(input);
    if (input.mode&32u)!=0u && rgba.a<0.1 {discard;}
    if (input.mode&16u)!=0u && rgba.a<f32((input.mode>>8u)&255u)/255.0 {discard;}
    return rgba;
}
@fragment fn fs_additive(input:VOutput,@builtin(front_facing) front:bool)->@location(0) vec4<f32>{
    if (input.mode&268435456u)!=0u {discard;}
    return additive_color(input,front);
}
@fragment fn fs_additive_write(input:VOutput,@builtin(front_facing) front:bool)->@location(0) vec4<f32>{
    if (input.mode&268435456u)==0u {discard;}
    return additive_color(input,front);
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
@fragment fn fs_sky(input:CompositeOut)->@location(0) vec4<f32>{
    let daylight=bitcast<f32>(lightmap.control.y);
    let top=floor(vec3<f32>(138.0,175.0,255.0)*daylight)/255.0;
    let bottom=floor(vec3<f32>(72.0,73.0,186.0)*daylight)/255.0;
    return vec4<f32>(mix(top,bottom,clamp(input.uv.y,0.0,1.0)),1.0);
}
"#;

/// Only a render source. The transparent Canvas overlay owns all mouse
/// navigation/selection events and 3D guides; it never picks from GPU pixels.
#[derive(Clone)]
pub(crate) enum SceneSource {
    Isometric(IsometricSurface),
    Spectator(crate::spectator::SpectatorScene),
    WorldOrthographic(
        crate::spectator::SpectatorScene,
        crate::orthographic_world::View,
    ),
}
#[derive(Clone)]
pub(crate) struct GpuScene(pub SceneSource);
#[derive(Default)]
pub(crate) struct State {
    frame_tick: Option<std::time::Instant>,
    last_presented: std::sync::Weak<crate::spectator::StreamMesh>,
    light_report: Arc<crate::lighting::Report>,
}
#[derive(Clone, Debug)]
enum WorldView {
    Free(crate::spectator::Camera),
    Orthographic(crate::orthographic_world::View),
}
#[derive(Clone, Debug)]
struct Presentation {
    columns: Option<crate::spectator_stream::Columns>,
    view: WorldView,
    mesh: Arc<std::sync::Mutex<Option<Arc<crate::spectator::StreamMesh>>>>,
    frame: crate::spectator::PresentedViewCache,
    section_order: bool,
    edit_refresh: bool,
    awaiting_source: bool,
}
type IsometricPresentation = (
    Arc<std::sync::Mutex<Option<Arc<crate::projected_entities::Rendered>>>>,
    Option<Arc<crate::projected_entities::Rendered>>,
);
#[derive(Clone, Debug)]
pub(crate) struct Primitive {
    frame_tick: Option<std::time::Instant>,
    vertices: Arc<Vec<Vertex>>,
    atlas: Option<Arc<TextureAtlas>>,
    quality: Quality,
    lighting: Arc<crate::lighting::Table>,
    light_report: std::sync::Weak<crate::lighting::Report>,
    camera: Option<crate::spectator::CameraUniform>,
    source_origin: Option<[i32; 3]>,
    /// Direct source-owned chunk frame, never a flattened world vertex list.
    stream_mesh: Option<Arc<crate::spectator::StreamMesh>>,
    presentation: Option<Presentation>,
    iso_presentation: Option<IsometricPresentation>,
}
/// 光标独立绘制，不发布源拾取帧，也不把光标几何合并进真实地形。
pub(crate) fn cursor_primitive(
    mesh: Option<Arc<crate::spectator::StreamMesh>>,
    camera: Option<crate::spectator::CameraUniform>,
    vertices: Arc<Vec<Vertex>>,
    quality: Quality,
    frame_tick: Option<std::time::Instant>,
) -> Primitive {
    let atlas = mesh.as_ref().and_then(|m| m.atlas.clone());
    let source_origin = camera
        .as_ref()
        .and_then(|_| mesh.as_ref().map(|m| m.origin));
    Primitive {
        frame_tick,
        vertices,
        atlas,
        quality,
        camera,
        source_origin,
        stream_mesh: if source_origin.is_some() { mesh } else { None },
        presentation: None,
        iso_presentation: None,
        lighting: crate::lighting::Table::unlit(),
        light_report: std::sync::Weak::new(),
    }
}
pub(crate) fn clip_cursor_vertices(vertices: &mut [Vertex], size: Size) {
    let min = vertices
        .iter()
        .map(|v| v.depth)
        .fold(f32::INFINITY, f32::min);
    let max = vertices
        .iter()
        .map(|v| v.depth)
        .fold(f32::NEG_INFINITY, f32::max);
    for vertex in vertices {
        to_clip(vertex, size, min, max);
    }
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
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Arc<Vec<Vertex>>> {
        if cancelled() {
            return None;
        }
        if let Some(rendered) = self
            .rendered_cache
            .lock()
            .expect("GPU 场景与呈现状态锁被毒化")
            .as_ref()
            .filter(|r| r.viewport.logical == size)
        {
            return Some(Arc::clone(&rendered.vertices));
        }
        let mut vertices = gpu_mesh::build_cancellable(self, size, cancelled)?;
        let terrain = Arc::new(vertices.clone());
        let mut atlas = self.texture_bank.as_ref().and_then(|b| b.atlas()).cloned();
        let projected = if let Some(frame) = &self.entity_frame {
            if atlas
                .as_ref()
                .is_none_or(|base| !crate::textures::atlas_compatible(&frame.atlas, base))
            {
                log::error!("[entities] 等轴投影图集与源地形不兼容");
                return None;
            }
            atlas = Some(Arc::clone(&frame.atlas));
            let projection = match self.prepare_projection(size, cancelled)? {
                Ok(p) => p,
                Err(error) => {
                    log::error!("[entities] 等轴投影失败：{error}");
                    return None;
                }
            };
            let clip = self.entity_clip.unwrap_or_else(|| {
                (
                    [
                        f64::from(self.surface.x) - 0.5,
                        f64::from(self.surface.y_min) - 0.5,
                        f64::from(self.surface.z) - 0.5,
                    ],
                    [
                        f64::from(self.surface.x) + f64::from(self.surface.width) - 0.5,
                        f64::from(self.surface.y_max) + 0.5,
                        f64::from(self.surface.z) + f64::from(self.surface.depth) - 0.5,
                    ],
                )
            });
            match crate::projected_entities::project(
                Arc::clone(frame),
                crate::projected_entities::Mapping::Isometric(projection),
                clip,
                cancelled,
            ) {
                Ok(Some(p)) => {
                    let p = Arc::new(p);
                    p.append_vertices(&mut vertices);
                    Some(p)
                }
                Ok(None) => return None,
                Err(error) => {
                    log::error!("[entities] 等轴实体编译失败：{error}");
                    return None;
                }
            }
        } else {
            None
        };
        if projected.is_some() {
            // 不透明源面先绘制，村民等共面层保持源顺序，透明面全局远至近。
            let mut normal = Vec::with_capacity(vertices.len());
            let mut ordered = Vec::new();
            let mut blended = Vec::<[Vertex; 6]>::new();
            for face in vertices.chunks_exact(6) {
                if face[0].mode & 3 == 2 {
                    blended.push(face.try_into().expect("完整六顶点面"));
                } else if face[0].mode & crate::material_mode::ORDERED_OPAQUE != 0 {
                    ordered.extend_from_slice(face);
                } else {
                    normal.extend_from_slice(face);
                }
            }
            blended.sort_by(|a, b| {
                crate::projected_entities::face_depth(a)
                    .total_cmp(&crate::projected_entities::face_depth(b))
            });
            normal.extend(ordered);
            normal.extend(blended.into_iter().flatten());
            vertices = normal;
        }
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
        let rendered = Arc::new(crate::projected_entities::Rendered {
            viewport: crate::render_viewport::RenderViewport::one_to_one(size),
            identity: Arc::new(()),
            vertices: Arc::clone(&vertices),
            atlas,
            projected,
            terrain,
        });
        let mut cache = self
            .gpu_frame_cache
            .lock()
            .expect("GPU 场景与呈现状态锁被毒化");
        if cancelled() {
            return None;
        }
        *cache = Some((size, Arc::clone(&vertices)));
        *self
            .rendered_cache
            .lock()
            .expect("GPU 场景与呈现状态锁被毒化") = Some(rendered);
        Some(vertices)
    }
}
/// Render only complete world-space geometry and preserve its original
/// source coordinate basis while the next streamed mesh builds.
pub(crate) fn choose_spectator_mesh(
    current: Option<Arc<crate::spectator::StreamMesh>>,
    current_surface: &Arc<mcedit2_world::surface::SurfaceView>,
    current_atlas: Option<Arc<TextureAtlas>>,
    current_stream: Option<Arc<crate::spectator::StreamMesh>>,
    fallback: Option<&crate::spectator::CompletedFrame>,
) -> crate::spectator::CompletedFrame {
    if let Some(stream_mesh) = current_stream {
        if !stream_mesh.complete {
            if let Some(previous) = fallback.filter(|f| {
                f.stream_mesh
                    .as_ref()
                    .is_some_and(|old| !stream_mesh.covers(old))
            }) {
                // 新轮次的第一个区块不能抹掉已显示的完整场景；图集和原点一同保留。
                let mut previous = previous.clone();
                previous.atlas = previous.stream_mesh.as_ref().and_then(|m| m.atlas.clone());
                return previous;
            }
        }
        let atlas = stream_mesh.atlas.clone();
        crate::spectator::CompletedFrame {
            vertices: Arc::new(Vec::new()),
            stream_mesh: Some(stream_mesh),
            surface: Arc::clone(current_surface),
            atlas,
        }
    } else if let Some(mesh) = current {
        crate::spectator::CompletedFrame {
            vertices: Arc::new(Vec::new()),
            atlas: mesh.atlas.clone(),
            stream_mesh: Some(mesh),
            surface: Arc::clone(current_surface),
        }
    } else if let Some(last) = fallback {
        let mut last = last.clone();
        if let Some(mesh) = &last.stream_mesh {
            last.atlas = mesh.atlas.clone();
        }
        last
    } else {
        crate::spectator::CompletedFrame {
            vertices: Arc::new(Vec::new()),
            stream_mesh: None,
            surface: Arc::clone(current_surface),
            atlas: current_atlas,
        }
    }
}
impl shader::Program<Message> for GpuScene {
    type State = State;
    type Primitive = Primitive;
    fn update(
        &self,
        state: &mut State,
        event: &iced::Event,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Option<shader::Action<Message>> {
        let iced::Event::Window(iced::window::Event::RedrawRequested(at)) = event else {
            return None;
        };
        state.frame_tick = Some(*at);
        if bounds.width < 1. || bounds.height < 1. {
            return None;
        }
        let complete = match &self.0 {
            SceneSource::Isometric(scene) => {
                crate::mesh_async::enqueue(scene, bounds.size());
                scene
                    .rendered_cache
                    .lock()
                    .expect("GPU 场景与呈现状态锁被毒化")
                    .as_ref()
                    .is_some_and(|rendered| rendered.viewport.logical == bounds.size())
            }
            SceneSource::Spectator(scene) | SceneSource::WorldOrthographic(scene, _) => {
                let presented = scene
                    .presented
                    .lock()
                    .expect("GPU 场景与呈现状态锁被毒化")
                    .clone();
                if !same_arc(state.last_presented.upgrade().as_ref(), presented.as_ref()) {
                    state.last_presented =
                        presented.as_ref().map(Arc::downgrade).unwrap_or_default();
                    return Some(shader::Action::publish(Message::WorldMeshPresented));
                }
                crate::mesh_async::enqueue_spectator(scene);
                let ready = world_candidate(scene).stream_mesh;
                let presented = scene.presented.lock().expect("GPU 场景与呈现状态锁被毒化");
                scene.mesh_complete()
                    && ready.as_ref().is_some_and(|mesh| {
                        presented.as_ref().is_some_and(|old| {
                            crate::spectator::StreamMesh::same_terrain(old, mesh)
                                && same_arc(old.entities.as_ref(), mesh.entities.as_ref())
                        })
                    })
            }
        };
        // GPU 分帧上传及透明顺序交接继续安排绘制；后台 CPU 结果另有合并通知。
        // 完成后停止，静态场景无需永久空转。
        if !complete || CAMERA_SORT_PENDING.load(std::sync::atomic::Ordering::Acquire) > 0 {
            return Some(shader::Action::request_redraw_at(
                *at + std::time::Duration::from_millis(16),
            ));
        }
        let deadline = match &self.0 {
            SceneSource::Isometric(scene) => scene
                .presented
                .lock()
                .expect("投影显示锁被毒化")
                .as_ref()
                .and_then(|shown| shown.atlas.as_ref())
                .and_then(|atlas| atlas.presented_animation_deadline()),
            SceneSource::Spectator(scene) | SceneSource::WorldOrthographic(scene, _) => scene
                .presented_view
                .lock()
                .expect("世界显示锁被毒化")
                .as_ref()
                .and_then(|shown| shown.atlas.as_ref())
                .and_then(|atlas| atlas.presented_animation_deadline()),
        };
        deadline.map(shader::Action::request_redraw_at)
    }
    fn draw(&self, state: &Self::State, _: mouse::Cursor, bounds: Rectangle) -> Primitive {
        let size = bounds.size();
        match &self.0 {
            SceneSource::Isometric(scene) => {
                crate::mesh_async::enqueue(scene, size);
                let rendered = scene
                    .rendered_cache
                    .lock()
                    .expect("GPU 场景与呈现状态锁被毒化")
                    .as_ref()
                    .filter(|rendered| rendered.viewport.logical == size)
                    .cloned();
                Primitive {
                    frame_tick: state.frame_tick,
                    vertices: rendered
                        .as_ref()
                        .map(|r| Arc::clone(&r.vertices))
                        .unwrap_or_else(|| Arc::new(Vec::new())),
                    atlas: rendered.as_ref().and_then(|r| r.atlas.clone()),
                    quality: scene.gpu_quality,
                    camera: None,
                    source_origin: None,
                    stream_mesh: None,
                    presentation: None,
                    iso_presentation: Some((Arc::clone(&scene.presented), rendered)),
                    lighting: Arc::clone(&scene.lighting),
                    light_report: Arc::downgrade(&state.light_report),
                }
            }
            SceneSource::Spectator(scene) | SceneSource::WorldOrthographic(scene, _) => {
                crate::mesh_async::enqueue_spectator(scene);
                let committed = world_candidate(scene);
                let awaiting_source = if let Some(snapshot) = &scene.snapshot {
                    snapshot
                        .ready
                        .lock()
                        .expect("源网格状态锁被毒化")
                        .as_ref()
                        .is_none_or(|ready| {
                            committed.stream_mesh.as_ref().is_none_or(|mesh| {
                                !crate::spectator::StreamMesh::same_terrain(ready, mesh)
                            })
                        })
                } else {
                    scene.cache.lock().expect("源网格状态锁被毒化").is_none()
                };
                let origin = committed.origin();
                let camera = match &self.0 {
                    SceneSource::WorldOrthographic(_, view) => committed
                        .stream_mesh
                        .as_ref()
                        .and_then(|mesh| match view.projection(mesh, size) {
                            Ok(projection) => projection.map(|p| {
                                crate::world_camera::WorldCamera::from(p)
                                    .uniform_at_origin(origin, size)
                            }),
                            Err(error) => {
                                log::error!("[view] 世界等轴构图失败：{error}");
                                None
                            }
                        }),
                    _ => Some(scene.camera.uniform_at_origin(origin, size)),
                };
                let view = match &self.0 {
                    SceneSource::WorldOrthographic(_, view) => {
                        WorldView::Orthographic(view.clone())
                    }
                    _ => WorldView::Free(scene.camera),
                };
                let edit_refresh = scene.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.edit_refresh
                        && committed.stream_mesh.as_ref().is_some_and(|mesh| {
                            snapshot
                                .ready
                                .lock()
                                .expect("编辑源网格锁被毒化")
                                .as_ref()
                                .is_some_and(|ready| {
                                    ready.complete
                                        && crate::spectator::StreamMesh::same_terrain(mesh, ready)
                                })
                        })
                });
                Primitive {
                    frame_tick: state.frame_tick,
                    vertices: committed.vertices,
                    stream_mesh: committed.stream_mesh,
                    atlas: committed.atlas,
                    quality: scene.quality,
                    camera,
                    source_origin: Some(origin),
                    presentation: Some(Presentation {
                        columns: scene.snapshot.as_ref().and_then(|snapshot| {
                            snapshot
                                .display_columns
                                .lock()
                                .expect("显示区块范围锁被毒化")
                                .clone()
                        }),
                        view,
                        mesh: Arc::clone(&scene.presented),
                        frame: Arc::clone(&scene.presented_view),
                        section_order: scene.snapshot.is_some(),
                        edit_refresh,
                        awaiting_source,
                    }),
                    iso_presentation: None,
                    lighting: Arc::clone(&scene.lighting),
                    light_report: Arc::downgrade(&state.light_report),
                }
            }
        }
    }
}
fn world_candidate(scene: &crate::spectator::SpectatorScene) -> crate::spectator::CompletedFrame {
    let ready = scene
        .cache
        .lock()
        .expect("GPU 场景与呈现状态锁被毒化")
        .clone();
    let streamed = scene.snapshot.as_ref().and_then(|snapshot| {
        snapshot
            .ready
            .lock()
            .expect("GPU 场景与呈现状态锁被毒化")
            .clone()
    });
    let mut result = choose_spectator_mesh(
        ready,
        &scene.surface,
        scene.texture_bank.as_ref().and_then(|b| b.atlas()).cloned(),
        streamed,
        scene.fallback.as_ref(),
    );
    if let Some(base) = &result.stream_mesh {
        let is_fallback = scene
            .fallback
            .as_ref()
            .and_then(|f| f.stream_mesh.as_ref())
            .is_some_and(|old| Arc::ptr_eq(old, base));
        if !is_fallback || !scene.entity_enabled {
            result.stream_mesh = Some(crate::entity_geometry::compose(
                base,
                scene.entity_frame.as_ref().filter(|frame| {
                    frame.show_textures == scene.show_textures
                        && frame.show_entities == scene.entity_enabled
                }),
                &scene.entity_cache,
            ));
        }
    }
    if let Some(mesh) = &result.stream_mesh {
        result.atlas = mesh.atlas.clone();
    }
    result
}
static FRAME_PROFILE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static FRAME_PROFILE_CLOCK: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
static CAMERA_SORT_PENDING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

enum LightSource {
    World(std::sync::Weak<crate::spectator::StreamMesh>),
    Projected(std::sync::Weak<Vec<Vertex>>),
}
struct LightValidation {
    key: crate::lighting::Key,
    source: LightSource,
    result: Result<(), String>,
}
pub(crate) struct Pipeline {
    visible_mesh: visible_mesh::Cache,
    command_bindings: Option<Arc<commands::Bindings>>,
    pending: Option<upload::Pending>,
    upload_failure: Option<Primitive>,
    active: Option<Primitive>,
    /// 当前实际 GPU 图集的编码布局，供新增 Section 逐页证明可安全叠加。
    stream_materials: Option<(Arc<[usize]>, Arc<material_hints::Hints>)>,
    upload_limits: upload::Limits,
    frame_upload_budget: Option<(std::time::Instant, upload::Budget)>,
    #[cfg(test)]
    opaque_y32: bool,
    #[cfg(test)]
    material_hints: bool,
    #[cfg(test)]
    upload_bytes: usize,
    #[cfg(test)]
    pub(super) metadata_publications: usize,
    #[cfg(test)]
    pub(super) ordered_preparations: usize,
    #[cfg(test)]
    timestamps: Option<(wgpu::QuerySet, u32)>,
    #[cfg(test)]
    profile_pass_issued: bool,
    #[cfg(test)]
    profile_view: Option<crate::frustum::Frustum>,
    opaque: wgpu::RenderPipeline,
    decal: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    tripwire: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    additive_write: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    world_opaque: wgpu::RenderPipeline,
    world_opaque_culled: wgpu::RenderPipeline,
    world_decal: wgpu::RenderPipeline,
    world_decal_culled: wgpu::RenderPipeline,
    world_region: [wgpu::RenderPipeline; 6],
    world_transparent: wgpu::RenderPipeline,
    world_transparent_culled: wgpu::RenderPipeline,
    world_tripwire: wgpu::RenderPipeline,
    world_tripwire_culled: wgpu::RenderPipeline,
    world_additive: [wgpu::RenderPipeline; 4],
    /// 各个面实例共用的四角拓扑；不为每面复制六个顶点索引。
    quad_indices: wgpu::Buffer,
    world_faces: Arc<Vec<world::TransparentFace>>,
    world_sources: transparent::Sources,
    world_sorted: transparent::Order,
    world_sort_key: world::SortKey,
    section_orders: std::collections::BTreeMap<
        upload::section_order::Key,
        Arc<upload::section_order::Resident>,
    >,
    section_animations: std::collections::BTreeMap<
        upload::section_order::Key,
        Arc<upload::section_order::AnimationSource>,
    >,
    transparent_layout: wgpu::BindGroupLayout,
    transparent_residency: residency::Arena,
    camera_sort: Option<upload::CameraSort>,
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
    last_source_stream: Option<Arc<crate::spectator::StreamMesh>>,
    /// Immutable for a given source-mesh Arc; a camera move should not scan
    /// every opaque face just to rediscover whether sorting is necessary.
    has_transparent_world_faces: bool,
    non_stream_has_tripwire: bool,
    transparent_faces: Vec<TransparentQuad>,
    transparent_order: Vec<usize>,
    atlas: Option<Arc<TextureAtlas>>,
    /// 保持无资源包时使用的白色默认纹理存活。
    _atlas_texture: wgpu::Texture,
    /// 按原页保留句柄索引；同组页共享纹理数组与绑定，填充预算超限则拆组。
    atlas_pages: Vec<(wgpu::Texture, wgpu::BindGroup, GpuMip)>,
    fallback_mip: GpuMip,
    atlas_layout: wgpu::BindGroupLayout,
    nearest: wgpu::Sampler,
    linear: wgpu::Sampler,
    atlas_bind_group: wgpu::BindGroup,
    /// GPU-side resident meshes keyed by source chunk X/Z and atlas page.
    /// Unchanged chunks reuse their existing buffers even when other chunks
    /// are inserted, removed or remeshed.
    chunks: std::collections::BTreeMap<(bool, usize, i32, i32, u32), GpuChunk>,
    /// Streaming source chunks are already independently meshed on the
    /// background thread. Reuse their exact Arc vertex buffers at the GPU
    /// without regrouping every face of every other chunk.
    stream_chunks: std::collections::BTreeMap<upload::ChunkKey, GpuStreamChunk>,
    stream_regions: upload::regional::Residents,
    region_draws: upload::regional::DrawList,
    frame_commands: Vec<commands::Command>,
    region_layout: wgpu::BindGroupLayout,
    section_visibility: crate::section_culling::Cache,
    draw_spans: Vec<DrawSpan>,
    segment_spans: Vec<Vec<DrawSpan>>,
    quality: Quality,
    offscreen_color: wgpu::Texture,
    offscreen_depth: wgpu::Texture,
    offscreen_size: (u32, u32),
    composite_bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    lightmap_buffer: wgpu::Buffer,
    light_table: Arc<crate::lighting::Table>,
    lightmap_key: crate::lighting::Key,
    light_validation: [Option<LightValidation>; 2],
    light_invalid: bool,
    lighting_changed: bool,
    chunk_layout: wgpu::BindGroupLayout,
    zero_offset: ChunkOffset,
    last_camera: Option<crate::spectator::CameraUniform>,
    last_visibility: Option<crate::frustum::ViewKey>,
    animation_frame: Option<Arc<crate::textures::atlas_animation::Frame>>,
    world_animated: atlas::AnimatedMaterials,
    projected_animated: atlas::AnimatedMaterials,
}
fn opaque_y32(pipeline: &Pipeline) -> bool {
    #[cfg(test)]
    {
        pipeline.opaque_y32
    }
    #[cfg(not(test))]
    {
        let _ = pipeline;
        true
    }
}
fn material_hints_enabled(pipeline: &Pipeline) -> bool {
    #[cfg(test)]
    {
        pipeline.material_hints
    }
    #[cfg(not(test))]
    {
        let _ = pipeline;
        true
    }
}
/// Respect the active adapter's maximum vertex buffer allocation. A scene
/// can contain more geometry than a single device buffer can hold; split at
/// complete quads (six vertices) without dropping faces or changing order.
fn gpu_segment_vertices(max_buffer_size: u64) -> usize {
    let stride = std::mem::size_of::<Vertex>() as u64;
    ((max_buffer_size / stride).min(u32::MAX as u64) / 6 * 6) as usize
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DrawSpan {
    page: usize,
    depth_equal: bool,
    additive: bool,
    start: u32,
    end: u32,
}
struct GpuChunk {
    bounds: Option<crate::frustum::MeshBounds>,
    source: Arc<Vec<Vertex>>,
    buffers: Vec<(wgpu::Buffer, u32)>,
    spans: Vec<Vec<DrawSpan>>,
}
#[derive(Clone)]
struct GpuStreamChunk {
    bounds: Option<crate::frustum::MeshBounds>,
    source: Arc<Vec<crate::world_mesh::WorldQuad>>,
    ordered: Arc<world::Buffers>,
    offset: SourceOffset,
    blended: Arc<Vec<usize>>,
    /// 来源几何租约跨场景复用；帧偏移与排序索引分别存储。
    transparent_allocation: Option<Arc<residency::Allocation>>,
    transparent_metadata: Option<Arc<[world::LocalFace]>>,
    /// 本区块在已发布透明面缓存中的连续范围，归属于该完整帧。
    transparent_range: std::ops::Range<usize>,
    page_groups: Arc<[usize]>,
    used_pages: Arc<[u32]>,
    hints: Arc<material_hints::Hints>,
    ordered_commands: commands::Cache,
}
/// 普通不透明面走区域批次，透明面只需要 CPU 偏移；仅有序源层创建独立 GPU uniform。
#[derive(Clone)]
struct SourceOffset {
    value: [f32; 4],
    uniform: Option<ChunkOffset>,
}
impl SourceOffset {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        origin: [i32; 3],
        frame: [i32; 3],
        ordered: bool,
    ) -> Self {
        let delta = origin_delta(origin, frame);
        let uniform = ordered.then(|| {
            let mut uniform = ChunkOffset::new(device, queue, layout);
            uniform.update(queue, origin, frame);
            uniform
        });
        Self {
            value: [delta[0], delta[1], delta[2], 0.0],
            uniform,
        }
    }
    fn local_view(&self, view: &crate::frustum::Frustum) -> crate::frustum::Frustum {
        view.at_offset(self.value)
    }
    fn binding(&self) -> &wgpu::BindGroup {
        &self
            .uniform
            .as_ref()
            .expect("有序源几何必须已创建原点绑定")
            .binding
    }
}
/// 每区块只更新 16 字节坐标偏移，保留顶点与 GPU 缓冲的驻留身份。
#[derive(Clone)]
struct ChunkOffset {
    buffer: wgpu::Buffer,
    binding: wgpu::BindGroup,
    value: [f32; 4],
}
impl ChunkOffset {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MCEdit2 区块坐标基底"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&[0.0f32; 4]));
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MCEdit2 独立区块原点"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        Self {
            buffer,
            binding,
            value: [0.0; 4],
        }
    }
    fn update(&mut self, queue: &wgpu::Queue, origin: [i32; 3], frame: [i32; 3]) -> bool {
        let delta = origin_delta(origin, frame);
        let value = [delta[0], delta[1], delta[2], 0.0];
        if self.value == value {
            return false;
        }
        self.value = value;
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&value));
        true
    }
}
fn origin_delta(origin: [i32; 3], frame: [i32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| (i64::from(origin[i]) - i64::from(frame[i])) as f32)
}
#[derive(Clone)]
struct TransparentQuad {
    vertices: [Vertex; 6],
    center: [f32; 3],
}
fn collect_transparent_quads_at<'a>(
    meshes: impl IntoIterator<Item = (&'a [Vertex], [f32; 3])>,
) -> Vec<TransparentQuad> {
    meshes
        .into_iter()
        .flat_map(|(mesh, delta)| mesh.chunks_exact(6).map(move |quad| (quad, delta)))
        .filter(|(quad, _)| {
            quad.iter()
                .all(|vertex| (vertex.mode & 3) == 2 && vertex.world_mode == 1)
        })
        .map(|(quad, delta)| {
            let vertices: [Vertex; 6] = std::array::from_fn(|index| {
                let mut vertex = quad[index];
                for i in 0..3 {
                    vertex.xyz[i] += delta[i];
                    if vertex.mode & crate::material_mode::FONT_TEXT != 0 {
                        vertex.rgba[i] += delta[i];
                    }
                }
                vertex
            });
            let center = if vertices[0].mode & crate::material_mode::FONT_TEXT != 0 {
                std::array::from_fn(|i| vertices[0].rgba[i])
            } else {
                std::array::from_fn(|i| {
                    (vertices[0].xyz[i]
                        + vertices[1].xyz[i]
                        + vertices[2].xyz[i]
                        + vertices[5].xyz[i])
                        * 0.25
                })
            };
            TransparentQuad { vertices, center }
        })
        .collect()
}
fn collect_transparent_quads<'a>(
    meshes: impl IntoIterator<Item = &'a [Vertex]>,
) -> Vec<TransparentQuad> {
    collect_transparent_quads_at(meshes.into_iter().map(|mesh| (mesh, [0.0; 3])))
}
fn sorted_transparent_quads(
    faces: &[TransparentQuad],
    order: &mut Vec<usize>,
    camera: Option<&crate::spectator::CameraUniform>,
) -> Vec<Vertex> {
    if order.len() != faces.len() {
        *order = (0..faces.len()).collect();
    }
    if let Some(camera) = camera {
        let depths = faces
            .iter()
            .map(|face| world::sort_distance(face.center, camera))
            .collect::<Vec<_>>();
        order.sort_unstable_by(|a, b| depths[*b].total_cmp(&depths[*a]).then_with(|| a.cmp(b)));
    } else {
        order.sort_unstable();
    }
    let mut result = Vec::with_capacity(faces.len().saturating_mul(6));
    for &index in order.iter() {
        result.extend_from_slice(&faces[index].vertices);
    }
    result
}
/// Preserve painter order in a draw stream while switching independently
/// allocated atlas page bindings at complete six-vertex face boundaries.
fn page_spans(vertices: &[Vertex]) -> Vec<DrawSpan> {
    let mut out: Vec<DrawSpan> = Vec::new();
    for (face, quad) in vertices.chunks_exact(6).enumerate() {
        let start = (face * 6) as u32;
        let page = quad[0].atlas_page as usize;
        let depth_equal = quad[0].mode & crate::material_mode::DEPTH_EQUAL != 0;
        let additive = quad[0].mode & crate::material_mode::ADDITIVE != 0;
        if let Some(last) = out.last_mut() {
            if last.page == page
                && last.end == start
                && last.depth_equal == depth_equal
                && last.additive == additive
            {
                last.end += 6;
                continue;
            }
        }
        out.push(DrawSpan {
            page,
            depth_equal,
            additive,
            start,
            end: start + 6,
        });
    }
    out
}
/// 普通投影和限定窗口同样按实际数组绑定合并相邻绘制段，顶点里的原始页号保持不变。
fn bound_spans<'a>(
    spans: &'a [DrawSpan],
    pages: &'a [atlas::Page],
) -> impl Iterator<Item = DrawSpan> + 'a {
    let mut cursor = 0;
    let group = move |page: usize| {
        if pages.is_empty() {
            page
        } else {
            pages[page].2.binding_page
        }
    };
    std::iter::from_fn(move || {
        let first = spans.get(cursor)?;
        let mut current = DrawSpan {
            page: group(first.page),
            depth_equal: first.depth_equal,
            additive: first.additive,
            start: first.start,
            end: first.end,
        };
        cursor += 1;
        while let Some(next) = spans.get(cursor) {
            if group(next.page) != current.page
                || current.end != next.start
                || next.depth_equal != current.depth_equal
                || next.additive != current.additive
            {
                break;
            }
            current.end = next.end;
            cursor += 1;
        }
        Some(current)
    })
}
fn chunk_bucket(quad: &[Vertex], origin: [i32; 3]) -> (i32, i32, u32) {
    // Source is world-relative; derive a stable chunk from the authored face
    // position, not the camera. Negative world coordinates use Euclidean div.
    let x = quad.iter().map(|v| v.xyz[0]).fold(f32::INFINITY, f32::min);
    let z = quad.iter().map(|v| v.xyz[2]).fold(f32::INFINITY, f32::min);
    let to_chunk = |position: f32, base: i32| {
        ((position.floor() as i64 + base as i64).clamp(i32::MIN as i64, i32::MAX as i64) as i32)
            .div_euclid(16)
    };
    (
        to_chunk(x, origin[0]),
        to_chunk(z, origin[2]),
        quad[0].atlas_page,
    )
}
fn same_vertices(a: &[Vertex], b: &[Vertex]) -> bool {
    bytemuck::cast_slice::<Vertex, u8>(a) == bytemuck::cast_slice::<Vertex, u8>(b)
}
fn make_chunk_buffers(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    vertices: &[Vertex],
) -> Vec<(wgpu::Buffer, u32)> {
    let count = gpu_segment_vertices(device.limits().max_buffer_size);
    if count == 0 || vertices.is_empty() {
        return Vec::new();
    }
    vertices
        .chunks(count)
        .map(|part| {
            let bytes = bytemuck::cast_slice::<Vertex, u8>(part);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("MCEdit2 individual resident source chunk"),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, bytes);
            (buffer, part.len() as u32)
        })
        .collect()
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
    if buffer_resized
        || previous.is_none_or(|old| old.len() != current.len() || !current.len().is_multiple_of(6))
    {
        let (changed, first) = mesh_upload_suffix(previous, current, buffer_resized);
        return (
            changed,
            first
                .map(|i| std::iter::once(i..current.len()).collect())
                .unwrap_or_default(),
        );
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
            let unchanged_bytes = (begin - last.end) * std::mem::size_of::<Vertex>();
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
        return (true, std::iter::once(first..current.len()).collect());
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
        let depth = world::sort_distance(center, camera);
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
#[derive(Clone)]
struct GpuMip {
    shared: Option<Arc<atlas::SharedGroup>>,
    pub(super) binding_page: usize,
    layer: u32,
    group_layers: u32,
    material_offset: u64,
    aux: wgpu::Texture,
    owner: wgpu::Texture,
    materials: wgpu::Buffer,
    page_table: wgpu::Buffer,
}
fn make_binding(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    texture: &wgpu::Texture,
    sampler: &wgpu::Sampler,
    camera: &wgpu::Buffer,
    lightmap: &wgpu::Buffer,
    mip: &GpuMip,
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
                binding: 7,
                resource: lightmap.as_entire_binding(),
            },
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
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&mip.aux.create_view(
                    &wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2Array),
                        ..Default::default()
                    },
                )),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&mip.owner.create_view(
                    &wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2Array),
                        ..Default::default()
                    },
                )),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: mip.materials.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: mip.page_table.as_entire_binding(),
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
fn scene_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    fragment: &str,
    stage: Pass,
    geometry: u8,
    cull_back: bool,
) -> wgpu::RenderPipeline {
    let depth_write = stage != Pass::Translucent;
    let world = geometry != 0;
    let indexed = geometry == 2;
    let rebased = geometry == 6;
    let compact = geometry == 5 || rebased;
    let regional = geometry >= 4;
    const INDEX_ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0=>Uint32x2];
    const ATTRS: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
        0=>Float32x2,1=>Float32,2=>Float32x4,3=>Float32x2,4=>Uint32,
        5=>Float32x3,6=>Uint32,7=>Uint32,8=>Uint32
    ];
    const WORLD_ATTRS: [wgpu::VertexAttribute; 15] = wgpu::vertex_attr_array![
        0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x3,
        4=>Float32x2,5=>Float32x2,6=>Float32x2,7=>Float32x2,
        8=>Float32x4,9=>Float32x4,10=>Float32x4,11=>Float32x4,12=>Uint32,13=>Uint32,14=>Uint32
    ];
    const COMPACT_ATTRS: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
        0=>Float32x3,1=>Float32x3,2=>Float32x2,3=>Float32x2,
        4=>Float32x4,5=>Uint32,6=>Uint32,7=>Uint32,8=>Uint32
    ];
    const REGION_ATTRS: [wgpu::VertexAttribute; 16] = wgpu::vertex_attr_array![
        0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x3,
        4=>Float32x2,5=>Float32x2,6=>Float32x2,7=>Float32x2,
        8=>Float32x4,9=>Float32x4,10=>Float32x4,11=>Float32x4,12=>Uint32,13=>Uint32,14=>Uint32,15=>Uint32
    ];
    const REGION_COMPACT_ATTRS: [wgpu::VertexAttribute; 10] = wgpu::vertex_attr_array![
        0=>Float32x3,1=>Float32x3,2=>Float32x2,3=>Float32x2,
        4=>Float32x4,5=>Uint32,6=>Uint32,7=>Uint32,8=>Uint32,9=>Uint32
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("MCEdit2 depth-tested original face stage"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(if rebased {
                "vs_world_compact"
            } else if indexed {
                "vs_world_indexed"
            } else if regional && compact {
                "vs_world_region_compact"
            } else if regional {
                "vs_world_region"
            } else if world {
                "vs_world"
            } else {
                "vs_main"
            }),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: if rebased {
                    std::mem::size_of::<encoding::CompactQuad>() as u64
                } else if indexed {
                    8
                } else if regional && compact {
                    std::mem::size_of::<encoding::RegionalCompact>() as u64
                } else if regional {
                    std::mem::size_of::<encoding::RegionalFull>() as u64
                } else if world {
                    std::mem::size_of::<crate::world_mesh::WorldQuad>() as u64
                } else {
                    std::mem::size_of::<Vertex>() as u64
                },
                step_mode: if world {
                    wgpu::VertexStepMode::Instance
                } else {
                    wgpu::VertexStepMode::Vertex
                },
                attributes: if rebased {
                    &COMPACT_ATTRS
                } else if indexed {
                    &INDEX_ATTRS
                } else if regional && compact {
                    &REGION_COMPACT_ATTRS
                } else if regional {
                    &REGION_ATTRS
                } else if world {
                    &WORLD_ATTRS
                } else {
                    &ATTRS
                },
            }],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: if fragment == "fs_additive" || fragment == "fs_additive_write" {
                    Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    })
                } else if stage == Pass::Opaque {
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
            cull_mode: cull_back.then_some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: depth_write,
            depth_compare: if fragment == "fs_decal" {
                wgpu::CompareFunction::Equal
            } else {
                wgpu::CompareFunction::LessEqual
            },
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}
fn sky_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("MCEdit2 加色场景实际天空背景"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_composite"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_sky"),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    })
}
impl Pipeline {
    #[inline]
    fn atlas_binding(&self, page: usize) -> &wgpu::BindGroup {
        if self.atlas_pages.is_empty() {
            &self.atlas_bind_group
        } else {
            &self.atlas_pages[page].1
        }
    }

    fn projected_pipeline(&self, span: &DrawSpan, stage: Pass) -> &wgpu::RenderPipeline {
        match stage {
            Pass::Opaque => {
                if span.depth_equal {
                    &self.decal
                } else {
                    &self.opaque
                }
            }
            Pass::Tripwire => {
                if span.additive {
                    &self.additive_write
                } else {
                    &self.tripwire
                }
            }
            Pass::Translucent => {
                if span.additive {
                    &self.additive
                } else {
                    &self.transparent
                }
            }
        }
    }
}
impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let source = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("MCEdit2 original-voxel depth and composition"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(residency::shader(WGSL, device))),
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MCEdit2 source atlas and compositing layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<
                            crate::lighting::Uniform,
                        >() as u64),
                    },
                    count: None,
                },
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let chunk_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MCEdit2 区块坐标偏移布局"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            }],
        });
        let zero_offset = ChunkOffset::new(device, queue, &chunk_layout);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("MCEdit2 GPU source and depth compositing layout"),
            bind_group_layouts: &[&atlas_layout, &chunk_layout],
            push_constant_ranges: &[],
        });
        let transparent_layout = residency::layout(device);
        let transparent_residency = residency::Arena::new(device);
        let indexed_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("MCEdit2 透明面索引绘制布局"),
            bind_group_layouts: &[&atlas_layout, &transparent_layout],
            push_constant_ranges: &[],
        });
        let opaque = scene_pipeline(
            device,
            &source,
            &layout,
            "fs_opaque",
            Pass::Opaque,
            0,
            false,
        );
        let decal = scene_pipeline(device, &source, &layout, "fs_decal", Pass::Opaque, 0, false);
        let transparent = scene_pipeline(
            device,
            &source,
            &layout,
            "fs_transparent",
            Pass::Translucent,
            0,
            false,
        );
        let tripwire = scene_pipeline(
            device,
            &source,
            &layout,
            "fs_tripwire",
            Pass::Tripwire,
            0,
            false,
        );
        let additive = scene_pipeline(
            device,
            &source,
            &layout,
            "fs_additive",
            Pass::Translucent,
            0,
            false,
        );
        let additive_write = scene_pipeline(
            device,
            &source,
            &layout,
            "fs_additive_write",
            Pass::Tripwire,
            0,
            false,
        );
        let sky = sky_pipeline(device, &source, &layout);
        let world_opaque = scene_pipeline(
            device,
            &source,
            &layout,
            "fs_opaque",
            Pass::Opaque,
            1,
            false,
        );
        let world_opaque_culled =
            scene_pipeline(device, &source, &layout, "fs_opaque", Pass::Opaque, 1, true);
        let world_decal =
            scene_pipeline(device, &source, &layout, "fs_decal", Pass::Opaque, 1, false);
        let world_decal_culled =
            scene_pipeline(device, &source, &layout, "fs_decal", Pass::Opaque, 1, true);
        let region_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MCEdit2 区域整数原点布局"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(16),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(32),
                    },
                    count: None,
                },
            ],
        });
        let regional_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("MCEdit2 区域网格双绑定组布局"),
                bind_group_layouts: &[&atlas_layout, &region_layout],
                push_constant_ranges: &[],
            });
        let world_region = std::array::from_fn(|index| {
            scene_pipeline(
                device,
                &source,
                &regional_pipeline_layout,
                "fs_opaque",
                Pass::Opaque,
                if index >= 4 {
                    6
                } else if index >= 2 {
                    5
                } else {
                    4
                },
                index % 2 != 0,
            )
        });
        let world_transparent = scene_pipeline(
            device,
            &source,
            &indexed_layout,
            "fs_transparent",
            Pass::Translucent,
            2,
            false,
        );
        let world_transparent_culled = scene_pipeline(
            device,
            &source,
            &indexed_layout,
            "fs_transparent",
            Pass::Translucent,
            2,
            true,
        );
        let world_tripwire = scene_pipeline(
            device,
            &source,
            &indexed_layout,
            "fs_tripwire",
            Pass::Tripwire,
            2,
            false,
        );
        let world_tripwire_culled = scene_pipeline(
            device,
            &source,
            &indexed_layout,
            "fs_tripwire",
            Pass::Tripwire,
            2,
            true,
        );
        let world_additive = std::array::from_fn(|index| {
            let write = index >= 2;
            scene_pipeline(
                device,
                &source,
                &indexed_layout,
                if write {
                    "fs_additive_write"
                } else {
                    "fs_additive"
                },
                if write {
                    Pass::Tripwire
                } else {
                    Pass::Translucent
                },
                2,
                index % 2 != 0,
            )
        });
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
        let shared_indices = world::shared_indices();
        let quad_indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MCEdit2 世界面共享四角索引"),
            size: std::mem::size_of_val(shared_indices.as_slice()) as u64,
            usage: wgpu::BufferUsages::INDEX,
            mapped_at_creation: true,
        });
        quad_indices
            .slice(..)
            .get_mapped_range_mut()
            .copy_from_slice(bytemuck::cast_slice(&shared_indices));
        quad_indices.unmap();
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
        let light_table = crate::lighting::Table::unlit();
        let lightmap_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MCEdit2 已保存光照的共享查色表"),
            size: std::mem::size_of::<crate::lighting::Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &lightmap_buffer,
            0,
            bytemuck::bytes_of(light_table.uniform()),
        );
        let nearest = make_sampler(device, wgpu::FilterMode::Nearest);
        let linear = make_sampler(device, wgpu::FilterMode::Linear);
        let fallback_mip = atlas::fallback(device);
        let atlas_bind_group = make_binding(
            device,
            &atlas_layout,
            &atlas_texture,
            &nearest,
            &camera_buffer,
            &lightmap_buffer,
            &fallback_mip,
        );
        let offscreen_color = make_offscreen_color(device, 1, 1);
        let offscreen_depth = make_offscreen_depth(device, 1, 1);
        let composite_bind_group = make_binding(
            device,
            &atlas_layout,
            &offscreen_color,
            &nearest,
            &camera_buffer,
            &lightmap_buffer,
            &fallback_mip,
        );
        Self {
            command_bindings: None,
            visible_mesh: Default::default(),
            opaque,
            decal,
            transparent,
            tripwire,
            world_opaque,
            world_opaque_culled,
            world_decal,
            world_decal_culled,
            world_region,
            world_transparent,
            world_transparent_culled,
            world_tripwire,
            world_tripwire_culled,
            world_additive,
            additive,
            additive_write,
            sky,
            quad_indices,
            world_faces: Arc::new(Vec::new()),
            world_sources: transparent::Sources::default(),
            world_sorted: transparent::Order::default(),
            world_sort_key: world::SortKey::Source,
            section_orders: Default::default(),
            section_animations: Default::default(),
            transparent_layout,
            transparent_residency,
            camera_sort: None,
            pending: None,
            upload_failure: None,
            active: None,
            stream_materials: None,
            upload_limits: upload::Limits::default(),
            frame_upload_budget: None,
            #[cfg(test)]
            opaque_y32: true,
            #[cfg(test)]
            material_hints: true,
            #[cfg(test)]
            upload_bytes: 0,
            #[cfg(test)]
            metadata_publications: 0,
            #[cfg(test)]
            ordered_preparations: 0,
            #[cfg(test)]
            timestamps: None,
            #[cfg(test)]
            profile_pass_issued: false,
            #[cfg(test)]
            profile_view: None,
            composite,
            buffer,
            capacity,
            count: 0,
            segments: Vec::new(),
            last_uploaded_vertices: None,
            last_source_mesh: None,
            last_source_stream: None,
            has_transparent_world_faces: false,
            non_stream_has_tripwire: false,
            transparent_faces: Vec::new(),
            transparent_order: Vec::new(),
            atlas: None,
            _atlas_texture: atlas_texture,
            atlas_pages: Vec::new(),
            fallback_mip,
            atlas_layout,
            nearest,
            linear,
            atlas_bind_group,
            chunks: std::collections::BTreeMap::new(),
            stream_chunks: std::collections::BTreeMap::new(),
            stream_regions: Default::default(),
            region_draws: Default::default(),
            frame_commands: Vec::new(),
            region_layout,
            section_visibility: crate::section_culling::Cache::default(),
            draw_spans: Vec::new(),
            segment_spans: Vec::new(),
            quality: Quality::Balanced,
            offscreen_color,
            offscreen_depth,
            offscreen_size: (1, 1),
            composite_bind_group,
            camera_buffer,
            lightmap_buffer,
            lightmap_key: light_table.key(),
            light_table,
            light_validation: [None, None],
            light_invalid: false,
            lighting_changed: true,
            chunk_layout,
            zero_offset,
            last_camera: None,
            last_visibility: None,
            animation_frame: None,
            world_animated: Default::default(),
            projected_animated: Default::default(),
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
        viewport: &Viewport,
    ) {
        let viewport = crate::render_viewport::RenderViewport::at_scale(
            bounds.size(),
            viewport.scale_factor(),
            device.limits().max_texture_dimension_2d,
        );
        let _native_span = iced_wgpu::window::frame_statistics::span("world_prepare");
        let mut upload_budget = match (self.frame_tick, pipeline.frame_upload_budget.take()) {
            (Some(tick), Some((previous, budget))) if tick == previous => budget,
            _ => upload::Budget::new(pipeline.upload_limits),
        };
        let previous_bytes = upload_budget.used();
        self.prepare_frame(pipeline, device, queue, viewport, &mut upload_budget);
        iced_wgpu::window::frame_statistics::counter(
            "upload_bytes",
            (upload_budget.used() - previous_bytes) as u64,
        );
        #[cfg(test)]
        {
            pipeline.upload_bytes = upload_budget.used();
        }
        if let Some(tick) = self.frame_tick {
            pipeline.frame_upload_budget = Some((tick, upload_budget));
        }
    }
    fn draw(&self, pipeline: &Pipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        self.draw_ready(pipeline, pass)
    }
}
impl Primitive {
    fn validate_lighting(&self, pipeline: &mut Pipeline, slot: usize) -> Result<(), String> {
        if !self.lighting.settings.enabled {
            return Ok(());
        }
        let key = self.lighting.key();
        if let Some(previous) = &pipeline.light_validation[slot] {
            let same_source = match (&previous.source, &self.stream_mesh) {
                (LightSource::World(old), Some(mesh)) => old.ptr_eq(&Arc::downgrade(mesh)),
                (LightSource::Projected(old), None) => old.ptr_eq(&Arc::downgrade(&self.vertices)),
                _ => false,
            };
            if previous.key == key && same_source {
                return previous.result.clone();
            }
        }
        let (source, result) = if let Some(mesh) = &self.stream_mesh {
            (
                LightSource::World(Arc::downgrade(mesh)),
                mesh.chunks.iter().try_for_each(|chunk| {
                    chunk
                        .geometry()
                        .lighting_usage()
                        .check(&self.lighting)
                        .map_err(|error| format!("源区块 {},{}：{error}", chunk.key.0, chunk.key.1))
                }),
            )
        } else {
            (
                LightSource::Projected(Arc::downgrade(&self.vertices)),
                self.vertices
                    .iter()
                    .try_for_each(|v| self.lighting.validate(v.light)),
            )
        };
        pipeline.light_validation[slot] = Some(LightValidation {
            key,
            source,
            result: result.clone(),
        });
        result
    }
    fn finish_lighting(&self, pipeline: &mut Pipeline, result: Result<(), String>) -> bool {
        let valid = result.is_ok();
        if valid && pipeline.light_invalid {
            pipeline.lighting_changed = true;
        }
        pipeline.light_invalid = !valid;
        if let Some(owner) = self.light_report.upgrade() {
            self.lighting.report(&owner, result);
        } else if let Err(error) = result {
            log::error!("[lighting] {error}");
        }
        if !valid {
            if let Some(presentation) = &self.presentation {
                *presentation.mesh.lock().expect("源显示锁被毒化") = None;
                *presentation.frame.lock().expect("源相机显示锁被毒化") = None;
            }
            if let Some((presented, _)) = &self.iso_presentation {
                *presented.lock().expect("投影显示锁被毒化") = None;
            }
        }
        valid
    }
    fn prepare_frame(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: crate::render_viewport::RenderViewport,
        upload_budget: &mut upload::Budget,
    ) {
        if let Some(owner) = self.light_report.upgrade() {
            self.lighting.enter(&owner);
        }
        if !Arc::ptr_eq(&pipeline.light_table, &self.lighting) {
            pipeline.light_table = Arc::clone(&self.lighting);
        }
        let lighting = self.validate_lighting(pipeline, 0);
        if lighting.is_err() {
            self.finish_lighting(pipeline, lighting);
            return;
        }
        if pipeline.lightmap_key != self.lighting.key() {
            queue.write_buffer(
                &pipeline.lightmap_buffer,
                0,
                bytemuck::bytes_of(self.lighting.uniform()),
            );
            pipeline.lightmap_key = self.lighting.key();
            pipeline.lighting_changed = true;
        }
        let world = self.stream_mesh.is_some();
        let previous = pipeline
            .pending
            .as_ref()
            .map(|p| &p.target)
            .or(pipeline.active.as_ref());
        let same_context =
            previous.is_none_or(|old| match (&old.presentation, &self.presentation) {
                (Some(a), Some(b)) => Arc::ptr_eq(&a.mesh, &b.mesh),
                (None, None) => true,
                _ => false,
            });
        if !world || !same_context {
            pipeline.visible_mesh.clear();
            pipeline.camera_sort = None;
            if let Some(old) = pipeline.active.take() {
                if let Some(presentation) = old.presentation {
                    *presentation
                        .mesh
                        .lock()
                        .expect("GPU 场景与呈现状态锁被毒化") = None;
                    *presentation
                        .frame
                        .lock()
                        .expect("GPU 场景与呈现状态锁被毒化") = None;
                }
            }
            pipeline.pending = None;
            pipeline.upload_failure = None;
            if !same_context {
                pipeline.last_source_stream = None;
                pipeline.stream_chunks.clear();
                pipeline.stream_regions.clear();
                pipeline.world_sorted.segments.clear();
                pipeline.world_sources = transparent::Sources::default();
                pipeline.world_faces = Arc::new(Vec::new());
                pipeline.section_orders.clear();
                pipeline.transparent_residency.clear();
            }
        }
        if !world {
            if !self.vertices.is_empty()
                && gpu_segment_vertices(device.limits().max_buffer_size) == 0
            {
                log::error!("[gpu] 设备缓冲上限不足以绘制一个完整投影面");
                if let Some((presented, _)) = &self.iso_presentation {
                    *presented.lock().expect("GPU 场景与呈现状态锁被毒化") = None;
                }
                return;
            }
            self.finish_lighting(pipeline, Ok(()));
            self.prepare_ready(pipeline, device, queue, viewport, false, false);
            if let Some((presented, rendered)) = &self.iso_presentation {
                let mut shown = presented.lock().expect("GPU 场景与呈现状态锁被毒化");
                let next = rendered.as_ref().map(|source| {
                    let source = crate::projected_entities::Rendered::with_animation_frame(
                        source,
                        pipeline.animation_frame.as_ref(),
                    );
                    crate::projected_entities::Rendered::presented(
                        &source,
                        shown.as_ref(),
                        viewport,
                    )
                });
                *shown = next;
            }
            return;
        }
        let awaiting_source = self
            .presentation
            .as_ref()
            .is_some_and(|p| p.awaiting_source);
        if !awaiting_source {
            if let Some(pending) = &mut pipeline.pending {
                pending.offer(self, pipeline.active.as_ref());
            }
        }
        // 返回当前已显示的绘制来源时直接取消过期上传，不让旧候选随后覆盖撤销结果。
        if !awaiting_source
            && pipeline
                .active
                .as_ref()
                .is_some_and(|active| self.same_gpu_source(active))
        {
            pipeline.pending = None;
            if pipeline.active.as_ref().is_some_and(|active| {
                !same_arc(active.stream_mesh.as_ref(), self.stream_mesh.as_ref())
            }) {
                pipeline.active = Some(self.clone());
                pipeline.last_source_stream = self.stream_mesh.clone();
                #[cfg(test)]
                {
                    pipeline.metadata_publications += 1;
                }
            }
        }
        let needs_upload = pipeline.active.as_ref().is_none_or(|active| {
            !same_arc(active.stream_mesh.as_ref(), self.stream_mesh.as_ref())
                || !same_arc(active.atlas.as_ref(), self.atlas.as_ref())
                || active.section_mode() != self.section_mode()
        });
        let failed = pipeline.upload_failure.as_ref().is_some_and(|old| {
            same_arc(old.stream_mesh.as_ref(), self.stream_mesh.as_ref())
                && same_arc(old.atlas.as_ref(), self.atlas.as_ref())
        });
        if needs_upload
            && !failed
            && pipeline.pending.is_none()
            && (!awaiting_source || pipeline.active.is_none())
        {
            pipeline.upload_failure = None;
            pipeline.pending = Some(upload::Pending::new(
                self.clone(),
                &mut pipeline.world_sorted,
                &pipeline.stream_chunks,
            ));
        }
        let mut committed = false;
        #[cfg(test)]
        {
            pipeline.upload_bytes = 0;
        }
        if let Some(mut pending) = pipeline.pending.take() {
            let started = FRAME_PROFILE
                .get_or_init(|| std::env::var_os("MCEDIT2_FRAME_PROFILE").is_some())
                .then(std::time::Instant::now);
            let camera = self.camera_for(&pending.target, viewport.logical);
            let interaction_pending = pipeline.camera_sort.is_some()
                || pipeline.active.as_ref().is_some_and(|active| {
                    !pipeline.world_faces.is_empty()
                        && world::SortKey::camera(
                            self.camera_for(active, viewport.logical).as_ref(),
                        ) != pipeline.world_sort_key
                });
            let minimum_transfer = (device.limits().max_texture_dimension_2d as usize * 4)
                .max(std::mem::size_of::<crate::world_mesh::WorldQuad>());
            let share = interaction_pending
                .then(|| upload_budget.share(minimum_transfer))
                .flatten();
            let result = if let Some(mut share) = share {
                let result = pending.advance(pipeline, device, queue, camera.as_ref(), &mut share);
                upload_budget.absorb(share);
                result
            } else {
                pending.advance(pipeline, device, queue, camera.as_ref(), upload_budget)
            };
            if let Some(start) = started {
                log::debug!(
                    "[frame-profile] stream elapsed_us={} complete={}",
                    start.elapsed().as_micros(),
                    matches!(result, Ok(true))
                );
            }
            #[cfg(test)]
            {
                pipeline.upload_bytes = pending.frame_bytes;
            }
            match result {
                Ok(true) => {
                    let target = pending.install(pipeline);
                    pipeline.active = Some(target);
                    committed = true;
                }
                Ok(false) => {
                    pending.update_preview(pipeline, self);
                    pipeline.pending = Some(pending);
                }
                Err(error) => {
                    log::error!("[gpu-upload] 场景交接失败：{error}");
                    // 同一个无效快照只报告一次；新快照或显式重新加载才重新尝试。
                    pipeline.upload_failure = Some(pending.target.clone());
                }
            }
        }
        let mut displayed = pipeline.active.clone().unwrap_or_else(|| Primitive {
            frame_tick: None,
            vertices: Arc::new(Vec::new()),
            atlas: None,
            quality: self.quality,
            camera: None,
            source_origin: Some([0; 3]),
            stream_mesh: None,
            presentation: self.presentation.clone(),
            iso_presentation: None,
            lighting: crate::lighting::Table::unlit(),
            light_report: std::sync::Weak::new(),
        });
        // 相机与实际显示记录立即跟随输入；后台排序只发布索引，不回写旧相机。
        let source_columns = displayed
            .presentation
            .as_ref()
            .and_then(|p| p.columns.clone());
        if let (Some(presentation), Some(latest)) =
            (&mut displayed.presentation, &self.presentation)
        {
            presentation.view = latest.view.clone();
            presentation.columns = latest.columns.clone();
        }
        if let Some(mesh) = pipeline
            .pending
            .as_ref()
            .and_then(|pending| pending.preview.mesh.as_ref())
        {
            displayed.stream_mesh = Some(Arc::clone(mesh));
        }
        if let Some(mesh) = &displayed.stream_mesh {
            let columns = displayed
                .presentation
                .as_ref()
                .and_then(|p| p.columns.as_ref());
            displayed.stream_mesh = Some(pipeline.visible_mesh.prepare(
                mesh,
                columns,
                source_columns.as_ref() == columns,
            ));
        }
        displayed.camera = self.camera_for(&displayed, viewport.logical);
        displayed.quality = self.quality;
        displayed.lighting = Arc::clone(&self.lighting);
        displayed.light_report = self.light_report.clone();
        if committed {
            pipeline.camera_sort = None;
        }
        let sort_camera = displayed.camera.map(world::SortKey::reference);
        let wanted_key = world::SortKey::camera(sort_camera.as_ref());
        let mut sorted = false;
        if !committed
            && displayed.stream_mesh.is_some()
            && !pipeline.world_faces.is_empty()
            && (wanted_key != pipeline.world_sort_key || pipeline.camera_sort.is_some())
        {
            if pipeline
                .camera_sort
                .as_ref()
                .is_some_and(|job| !Arc::ptr_eq(&job.source, &pipeline.world_faces))
            {
                pipeline.camera_sort = None;
            }
            if pipeline.camera_sort.is_none() && wanted_key != pipeline.world_sort_key {
                let page_groups = pipeline
                    .atlas_pages
                    .iter()
                    .map(|page| page.2.binding_page)
                    .collect::<Vec<_>>()
                    .into();
                pipeline.camera_sort = Some(upload::CameraSort::new(
                    Arc::clone(&pipeline.world_faces),
                    displayed.camera,
                    device,
                    &mut pipeline.world_sorted,
                    page_groups,
                    pipeline.world_sources.clone(),
                ));
            }
            if let Some(mut job) = pipeline.camera_sort.take() {
                match job.advance(&pipeline.world_sources, device, queue, upload_budget) {
                    Ok(true) => {
                        let key = job.key;
                        let previous = std::mem::replace(&mut pipeline.world_sorted, job.finish());
                        pipeline.world_sorted.recycle(previous);
                        pipeline.world_sort_key = key;
                        sorted = true;
                        if wanted_key != key {
                            let page_groups = pipeline
                                .atlas_pages
                                .iter()
                                .map(|page| page.2.binding_page)
                                .collect::<Vec<_>>()
                                .into();
                            pipeline.camera_sort = Some(upload::CameraSort::new(
                                Arc::clone(&pipeline.world_faces),
                                displayed.camera,
                                device,
                                &mut pipeline.world_sorted,
                                page_groups,
                                pipeline.world_sources.clone(),
                            ));
                        }
                    }
                    Ok(false) => pipeline.camera_sort = Some(job),
                    Err(error) => log::error!("[gpu] 透明排序交接失败：{error}"),
                }
            }
        }
        if !pipeline.section_orders.is_empty()
            || pipeline
                .pending
                .as_ref()
                .is_some_and(|p| !p.preview.transparent.is_empty())
        {
            let groups = pipeline
                .atlas_pages
                .iter()
                .map(|page| page.2.binding_page)
                .collect::<Vec<_>>();
            sorted |= upload::section_order::resort(
                &pipeline.section_orders,
                pipeline.pending.as_ref().map(|pending| &pending.preview),
                device,
                queue,
                displayed.camera,
                groups.into(),
                upload_budget,
                displayed
                    .presentation
                    .as_ref()
                    .and_then(|p| p.columns.as_deref()),
            );
        }
        // 移动期间继续使用同一来源的上一份完整顺序；相机和点选矩阵保持当前帧一致。
        displayed.frame_tick = self.frame_tick;
        let display_changed = !same_arc(
            pipeline.last_source_stream.as_ref(),
            displayed.stream_mesh.as_ref(),
        );
        let lighting = displayed.validate_lighting(pipeline, 1);
        if !displayed.finish_lighting(pipeline, lighting) {
            return;
        }
        displayed.prepare_ready(
            pipeline,
            device,
            queue,
            viewport,
            committed || display_changed,
            sorted,
        );
        if committed && !pipeline.world_faces.is_empty() && wanted_key != pipeline.world_sort_key {
            let page_groups = pipeline
                .atlas_pages
                .iter()
                .map(|page| page.2.binding_page)
                .collect::<Vec<_>>()
                .into();
            pipeline.camera_sort = Some(upload::CameraSort::new(
                Arc::clone(&pipeline.world_faces),
                displayed.camera,
                device,
                &mut pipeline.world_sorted,
                page_groups,
                pipeline.world_sources.clone(),
            ));
        }
        if let Some(presentation) = &displayed.presentation {
            *presentation
                .mesh
                .lock()
                .expect("GPU 场景与呈现状态锁被毒化") = displayed.stream_mesh.clone();
            *presentation
                .frame
                .lock()
                .expect("GPU 场景与呈现状态锁被毒化") =
                displayed.stream_mesh.as_ref().and_then(|mesh| {
                    let camera = match &presentation.view {
                        WorldView::Free(camera) => {
                            Some(crate::world_camera::WorldCamera::Free(*camera))
                        }
                        WorldView::Orthographic(view) => {
                            match view.projection(mesh, viewport.logical) {
                                Ok(p) => p.map(crate::world_camera::WorldCamera::from),
                                Err(e) => {
                                    log::error!("[view] 已显示画面的构图失败：{e}");
                                    None
                                }
                            }
                        }
                    }?;
                    let atlas = displayed.atlas.as_ref().map(|atlas| {
                        let Some(frame) = &pipeline.animation_frame else {
                            return Arc::clone(atlas);
                        };
                        let mut snapshot = atlas.as_ref().clone();
                        snapshot.animation_frame = Some(Arc::clone(frame));
                        Arc::new(snapshot)
                    });
                    Some(crate::spectator::PresentedView {
                        camera,
                        viewport,
                        quality: displayed.quality,
                        mesh: Arc::clone(mesh),
                        atlas,
                    })
                });
        }
    }
}
fn same_arc<T>(a: Option<&Arc<T>>, b: Option<&Arc<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}
impl Primitive {
    /// 原点、有序源几何及图集身份相同，才允许直接沿用驻留 GPU 数据。
    fn same_gpu_source(&self, other: &Self) -> bool {
        self.source_origin == other.source_origin
            && self.section_mode() == other.section_mode()
            && same_arc(self.atlas.as_ref(), other.atlas.as_ref())
            && match (&self.stream_mesh, &other.stream_mesh) {
                (Some(a), Some(b)) => a.origin == b.origin && a.chunks.same_geometry(&b.chunks),
                _ => false,
            }
    }
    fn section_mode(&self) -> bool {
        self.presentation.as_ref().is_some_and(|p| p.section_order)
    }
    fn camera_for(
        &self,
        target: &Primitive,
        size: Size,
    ) -> Option<crate::spectator::CameraUniform> {
        let origin = target.source_origin?;
        match self.presentation.as_ref().map(|p| &p.view) {
            Some(WorldView::Free(camera)) => Some(camera.uniform_at_origin(origin, size)),
            Some(WorldView::Orthographic(view)) => {
                target
                    .stream_mesh
                    .as_ref()
                    .and_then(|mesh| match view.projection(mesh, size) {
                        Ok(projection) => projection.map(|p| {
                            crate::world_camera::WorldCamera::from(p)
                                .uniform_at_origin(origin, size)
                        }),
                        Err(error) => {
                            log::error!("[view] 世界等轴构图失败：{error}");
                            None
                        }
                    })
            }
            None => self.camera.map(|mut camera| {
                let delta = origin_delta(self.source_origin.unwrap_or(origin), origin);
                for (p, d) in camera.position[..3].iter_mut().zip(delta) {
                    *p += d;
                }
                camera
            }),
        }
    }
    fn prepare_ready(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: crate::render_viewport::RenderViewport,
        staged: bool,
        async_sorted: bool,
    ) {
        let profile = *std::sync::OnceLock::get_or_init(&FRAME_PROFILE, || {
            std::env::var_os("MCEDIT2_FRAME_PROFILE").is_some()
        });
        let collecting = profile || iced_wgpu::window::frame_statistics::is_recording();
        let started = profile.then(std::time::Instant::now);
        let mesh_identity_changed = if let Some(stream) = &self.stream_mesh {
            pipeline
                .last_source_stream
                .as_ref()
                .is_none_or(|old| !Arc::ptr_eq(old, stream))
        } else {
            pipeline.last_source_stream.is_some()
                || pipeline
                    .last_source_mesh
                    .as_ref()
                    .is_none_or(|old| !Arc::ptr_eq(old, &self.vertices))
        };
        let changed_camera = pipeline.last_camera != self.camera;
        if mesh_identity_changed {
            pipeline.non_stream_has_tripwire = self.stream_mesh.is_none()
                && self
                    .vertices
                    .iter()
                    .any(|v| crate::material_mode::pass(v.mode) == Pass::Tripwire);
            pipeline.has_transparent_world_faces = if self.stream_mesh.is_some() {
                !pipeline.world_faces.is_empty()
            } else {
                self.vertices
                    .iter()
                    .any(|v| (v.mode & 3) == 2 && v.world_mode == 1)
            };
        }
        // 透视按眼点距离排序，转头不触发重排；正交按观察方向排序。
        let changed_direction = match (&pipeline.last_camera, &self.camera) {
            (Some(previous), Some(current)) => previous.forward != current.forward,
            _ => changed_camera,
        };
        let resort = !async_sorted
            && self.camera.is_some()
            && pipeline.has_transparent_world_faces
            && (world::SortKey::camera(pipeline.last_camera.as_ref())
                != world::SortKey::camera(self.camera.as_ref())
                || mesh_identity_changed);
        let mut changed_mesh = staged || async_sorted;
        if let Some(origin) = self.source_origin {
            if self.stream_mesh.is_some() {
                if !pipeline.chunks.is_empty() {
                    pipeline.chunks.clear();
                    changed_mesh = true;
                }
            } else {
                if !pipeline.stream_chunks.is_empty() {
                    pipeline.stream_chunks.clear();
                    pipeline.stream_regions.clear();
                    changed_mesh = true;
                }
                // Each independent resident GPU buffer belongs to a real source
                // chunk and atlas page. A new chunk cannot shift any other
                // chunk's GPU offsets. Only dirty chunks are uploaded.
                if mesh_identity_changed || pipeline.chunks.is_empty() {
                    let mut groups: std::collections::BTreeMap<
                        (bool, usize, i32, i32, u32),
                        Vec<Vertex>,
                    > = std::collections::BTreeMap::new();
                    let mut ordered_run = 0usize;
                    let mut previous_ordered = None;
                    for quad in self.vertices.chunks_exact(6) {
                        if quad.iter().all(|v| (v.mode & 3) == 2 && v.world_mode == 1) {
                            continue;
                        }
                        let bucket = chunk_bucket(quad, origin);
                        let ordered = quad[0].mode
                            & (crate::material_mode::ORDERED_OPAQUE
                                | crate::material_mode::DEPTH_EQUAL)
                            != 0;
                        if ordered && previous_ordered != Some(bucket) {
                            ordered_run += 1;
                            previous_ordered = Some(bucket);
                        }
                        groups
                            .entry((
                                ordered,
                                if ordered { ordered_run } else { 0 },
                                bucket.0,
                                bucket.1,
                                bucket.2,
                            ))
                            .or_default()
                            .extend_from_slice(quad);
                    }
                    let mut previous = std::mem::take(&mut pipeline.chunks);
                    let mut next = std::collections::BTreeMap::new();
                    for (key, vertices) in groups {
                        let entry = match previous.remove(&key) {
                            Some(resident) if same_vertices(&resident.source, &vertices) => {
                                resident
                            }
                            _ => {
                                changed_mesh = true;
                                let buffers = make_chunk_buffers(device, queue, &vertices);
                                let spans = vertices
                                    .chunks(gpu_segment_vertices(device.limits().max_buffer_size))
                                    .map(page_spans)
                                    .collect();
                                GpuChunk {
                                    spans,
                                    bounds: crate::frustum::MeshBounds::from_vertices(&vertices),
                                    source: Arc::new(vertices),
                                    buffers,
                                }
                            }
                        };
                        next.insert(key, entry);
                    }
                    if !previous.is_empty() {
                        changed_mesh = true;
                    }
                    pipeline.chunks = next;
                }
            }
            // 流式透明顺序只由后台任务交接；转向不再物化、上传完整面，也不在此同步排序。
            if self.stream_mesh.is_some() {
                pipeline.transparent_faces.clear();
                pipeline.transparent_order.clear();
                pipeline.segments.clear();
                pipeline.segment_spans.clear();
                pipeline.count = 0;
                pipeline.draw_spans.clear();
            } else if mesh_identity_changed || resort {
                if mesh_identity_changed {
                    pipeline.transparent_faces =
                        collect_transparent_quads(std::iter::once(self.vertices.as_slice()));
                    pipeline.transparent_order.clear();
                }
                let transparent = sorted_transparent_quads(
                    &pipeline.transparent_faces,
                    &mut pipeline.transparent_order,
                    self.camera.as_ref(),
                );
                let needed = transparent.len() as u64 * std::mem::size_of::<Vertex>() as u64;
                let max_buffer = device.limits().max_buffer_size;
                if needed > max_buffer || transparent.len() > u32::MAX as usize {
                    pipeline.segments = make_chunk_buffers(device, queue, &transparent);
                    let each = gpu_segment_vertices(max_buffer);
                    pipeline.segment_spans = transparent.chunks(each).map(page_spans).collect();
                    pipeline.count = 0;
                } else {
                    pipeline.segments.clear();
                    pipeline.segment_spans.clear();
                    if needed > pipeline.capacity {
                        pipeline.capacity = needed.next_power_of_two().min(max_buffer);
                        pipeline.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("MCEdit2 dynamic sorted transparent faces"),
                            size: pipeline.capacity,
                            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        });
                    }
                    if !transparent.is_empty() {
                        queue.write_buffer(
                            &pipeline.buffer,
                            0,
                            bytemuck::cast_slice::<Vertex, u8>(&transparent),
                        );
                    }
                    pipeline.count = transparent.len() as u32;
                    pipeline.draw_spans = page_spans(&transparent);
                }
                changed_mesh = true;
            }
            pipeline.last_uploaded_vertices = None;
        } else {
            let was_chunks = !pipeline.chunks.is_empty() || !pipeline.stream_chunks.is_empty();
            if was_chunks {
                pipeline.chunks.clear();
                pipeline.stream_chunks.clear();
                pipeline.stream_regions.clear();
            }
            // Existing isometric projection uses screen-space vertices.
            // Keep its suffix/range writes while the 3D spectator mode uses
            // independent per-chunk buffers.
            let unchanged = !mesh_identity_changed && !resort && !was_chunks;
            let ordered = if resort {
                self.camera.as_ref().map(|camera| {
                    Arc::new(ordered_transparent_vertices(
                        self.vertices.as_slice(),
                        camera,
                    ))
                })
            } else if !unchanged {
                Some(Arc::clone(&self.vertices))
            } else {
                None
            };
            let stride = std::mem::size_of::<Vertex>() as u64;
            let needed = self.vertices.len() as u64 * stride;
            let max_buffer = device.limits().max_buffer_size;
            if needed > max_buffer || self.vertices.len() > u32::MAX as usize {
                let max_vertices = gpu_segment_vertices(max_buffer);
                if max_vertices == 0 {
                    return;
                }
                if !unchanged || pipeline.segments.is_empty() {
                    let current = ordered.as_deref().unwrap_or(&self.vertices);
                    pipeline.segments = make_chunk_buffers(device, queue, current);
                    pipeline.segment_spans = current.chunks(max_vertices).map(page_spans).collect();
                    pipeline.last_uploaded_vertices =
                        Some(ordered.unwrap_or_else(|| Arc::clone(&self.vertices)));
                    changed_mesh = true;
                }
                pipeline.count = 0;
            } else {
                let was_segmented = !pipeline.segments.is_empty();
                if was_segmented {
                    pipeline.segments.clear();
                    pipeline.segment_spans.clear();
                    pipeline.last_uploaded_vertices = None;
                }
                let resized = needed > pipeline.capacity;
                if resized {
                    pipeline.capacity = needed.next_power_of_two().min(max_buffer);
                    pipeline.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("MCEdit2 isometric source GPU mesh"),
                        size: pipeline.capacity,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                }
                let (modified, ranges) = if unchanged && !resized && !was_segmented {
                    (false, Vec::new())
                } else {
                    let current = ordered.as_deref().unwrap_or(&self.vertices);
                    mesh_upload_ranges(
                        pipeline
                            .last_uploaded_vertices
                            .as_deref()
                            .map(Vec::as_slice),
                        current,
                        resized || was_segmented,
                    )
                };
                if !ranges.is_empty() {
                    let current = ordered.as_deref().unwrap_or(&self.vertices);
                    for range in ranges {
                        let bytes = bytemuck::cast_slice::<Vertex, u8>(&current[range.clone()]);
                        if !bytes.is_empty() {
                            queue.write_buffer(
                                &pipeline.buffer,
                                range.start as u64 * stride,
                                bytes,
                            );
                        }
                    }
                }
                if let Some(v) = ordered {
                    pipeline.draw_spans = page_spans(&v);
                    pipeline.last_uploaded_vertices = Some(v);
                }
                pipeline.count = self.vertices.len() as u32;
                changed_mesh = modified || was_segmented || was_chunks;
            }
        }
        pipeline.last_source_mesh = if self.stream_mesh.is_some() {
            None
        } else {
            Some(Arc::clone(&self.vertices))
        };
        pipeline.last_source_stream = self.stream_mesh.clone();
        if self.stream_mesh.is_none() {
            pipeline.world_faces = Arc::new(Vec::new());
            pipeline.section_orders.clear();
            pipeline.world_sources = transparent::Sources::default();
            pipeline.world_sorted.segments.clear();
            pipeline.transparent_residency.clear();
        }
        let changed_atlas = match (&pipeline.atlas, &self.atlas) {
            (Some(old), Some(new)) => !Arc::ptr_eq(old, new),
            (None, None) => false,
            _ => true,
        };
        let changed_quality = pipeline.quality != self.quality;
        if changed_atlas || changed_quality {
            let sampler = match self.quality {
                Quality::Detailed => &pipeline.linear,
                _ => &pipeline.nearest,
            };
            let next_gpu = if let Some(source) = self.atlas.as_ref() {
                match atlas::upload_full(
                    source,
                    pipeline.atlas.as_deref(),
                    &pipeline.atlas_pages,
                    pipeline.quality == self.quality,
                    device,
                    queue,
                    &pipeline.atlas_layout,
                    sampler,
                    &pipeline.camera_buffer,
                    &pipeline.lightmap_buffer,
                ) {
                    Ok(pages) => pages,
                    Err(error) => {
                        log::error!("[gpu] 图集数组上传失败，保留已完成画面：{error}");
                        return;
                    }
                }
            } else {
                Vec::new()
            };
            // 所有原页上传完成后才交接数组；已显示资源始终保持只读。
            pipeline.atlas_pages = next_gpu;
            pipeline.atlas = self.atlas.clone();
            pipeline.stream_materials = None;
            pipeline.quality = self.quality;
        }
        if changed_atlas {
            pipeline.animation_frame = None;
        }
        if self.stream_mesh.is_none() && (mesh_identity_changed || changed_atlas) {
            let faces = self
                .vertices
                .chunks_exact(3)
                .map(|tri| (tri[0].atlas_page, [tri[0].uv, tri[1].uv, tri[2].uv]));
            pipeline.projected_animated =
                match atlas::animated_materials(self.atlas.as_deref(), faces, &|| false) {
                    Ok(used) => used,
                    Err(error) => {
                        log::error!("[gpu] 投影动画来源无效：{error}");
                        return;
                    }
                };
        }
        let used = if self.stream_mesh.is_some() {
            pipeline
                .pending
                .as_ref()
                .filter(|pending| pending.preview.mesh.is_some())
                .map_or(&pipeline.world_animated, |pending| {
                    &pending.preview.animated
                })
        } else {
            &pipeline.projected_animated
        };
        let animation_frame = match self
            .atlas
            .as_ref()
            .map(|source| {
                atlas::animate(
                    source,
                    &pipeline.atlas_pages,
                    queue,
                    self.frame_tick,
                    used,
                    pipeline.animation_frame.as_ref(),
                )
            })
            .transpose()
        {
            Ok(tick) => tick.flatten(),
            Err(error) => {
                log::error!("[gpu] 动画材质更新失败：{error}");
                return;
            }
        };
        let changed_animation = match (&pipeline.animation_frame, &animation_frame) {
            (Some(old), Some(new)) => {
                old.samples.len() != new.samples.len()
                    || old.samples.iter().zip(&new.samples).any(
                        |((old_key, old), (new_key, new))| {
                            old_key != new_key || old.stamp != new.stamp
                        },
                    )
            }
            (None, None) => false,
            _ => true,
        };
        pipeline.animation_frame = animation_frame;
        if changed_camera {
            queue.write_buffer(
                &pipeline.camera_buffer,
                0,
                bytemuck::bytes_of(&self.camera.unwrap_or_default()),
            );
            pipeline.last_camera = self.camera;
        }
        let target = (viewport.raster.width, viewport.raster.height);
        iced_wgpu::window::frame_statistics::counter("world_raster_width", u64::from(target.0));
        iced_wgpu::window::frame_statistics::counter("world_raster_height", u64::from(target.1));
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
                &pipeline.lightmap_buffer,
                &pipeline.fallback_mip,
            );
        }
        if self.camera.is_none()
            || self
                .stream_mesh
                .as_ref()
                .is_none_or(|mesh| mesh.visibility.is_none())
        {
            pipeline.section_visibility.release_source();
        }
        let view = self.camera.as_ref().map(|camera| {
            let view = crate::frustum::Frustum::new(camera);
            let sections = self.stream_mesh.as_ref().and_then(|mesh| {
                mesh.visibility.as_ref().and_then(|topology| {
                    pipeline
                        .section_visibility
                        .prepare(topology, mesh.origin, &view)
                })
            });
            view.with_sections(sections)
        });
        #[cfg(test)]
        {
            pipeline.profile_view = view.clone();
        }
        let visibility_key = view.as_ref().map(crate::frustum::Frustum::key);
        let changed_visibility = pipeline.last_visibility != visibility_key;
        // Re-render the intermediate color and Z targets only if the source
        // faces, source PNG atlas, quality sampler or viewport have changed.
        if !changed_mesh
            && !changed_animation
            && !changed_atlas
            && !changed_quality
            && !changed_camera
            && !resized_view
            && !changed_visibility
            && !pipeline.lighting_changed
        {
            if let Some(start) = started {
                log::debug!(
                    "[frame-profile] idle at_us={} prepare_us={}",
                    FRAME_PROFILE_CLOCK
                        .get_or_init(std::time::Instant::now)
                        .elapsed()
                        .as_micros(),
                    start.elapsed().as_micros()
                );
            }
            return;
        }
        pipeline.last_visibility = visibility_key;
        pipeline.lighting_changed = false;
        let encode_started = profile.then(std::time::Instant::now);
        #[cfg(test)]
        {
            pipeline.profile_pass_issued = true;
        }
        let mut opaque_chunks = 0usize;
        let mut opaque_faces = 0usize;
        let mut opaque_spans = 0usize;
        let mut transparent_spans = 0usize;
        let mut transparent_faces = 0usize;
        let mut tripwire_spans = 0usize;
        let mut tripwire_faces = 0usize;
        let mut bundles_issued = 0usize;
        let mut bundles_rebuilt = 0usize;
        let command_bindings = commands::bindings(pipeline);
        let mut bundles = std::mem::take(&mut pipeline.frame_commands);
        if profile {
            let stats = pipeline.section_visibility.stats;
            log::debug!(
                "[frame-profile] section_visibility visible={} total={} gap_ranges={} prepare_us={} reused={}",
                stats.visible, stats.total, stats.gap_ranges, stats.micros, stats.reused
            );
        }
        let color = pipeline
            .offscreen_color
            .create_view(&wgpu::TextureViewDescriptor::default());
        let depth = pipeline
            .offscreen_depth
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("MCEdit2 opaque / tripwire depth and translucent passes"),
        });
        // Opaque and binary-alpha-cutout fragments populate Z. The exact
        // source model triangles can intersect without painter-order errors.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Original opaque and tripwire depth / color"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        // 普通源保留透明背景；加色场景在下方先绘制实际天空，避免直通图片丢失零 Alpha 的 RGB。
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
                timestamp_writes: {
                    #[cfg(test)]
                    {
                        pipeline.timestamps.as_ref().map(|(query_set, base)| {
                            wgpu::RenderPassTimestampWrites {
                                query_set,
                                beginning_of_pass_write_index: Some(*base),
                                end_of_pass_write_index: Some(*base + 1),
                            }
                        })
                    }
                    #[cfg(not(test))]
                    {
                        None
                    }
                },
                occlusion_query_set: None,
            });
            pass.set_bind_group(1, &pipeline.zero_offset.binding, &[]);
            let additive = self.stream_mesh.as_ref().map_or_else(
                || {
                    self.vertices
                        .iter()
                        .any(|vertex| vertex.mode & crate::material_mode::ADDITIVE != 0)
                },
                |mesh| {
                    mesh.chunks
                        .iter()
                        .any(|chunk| chunk.geometry().has_additive())
                },
            );
            if additive {
                pass.set_pipeline(&pipeline.sky);
                pass.set_bind_group(0, &pipeline.atlas_bind_group, &[]);
                pass.draw(0..6, 0..1);
            }
            pass.set_pipeline(&pipeline.opaque);
            if self.stream_mesh.is_some() {
                let origin = self
                    .stream_mesh
                    .as_ref()
                    .expect("区域绘制持有流式源")
                    .origin;
                let preview = pipeline
                    .pending
                    .as_mut()
                    .map(|pending| &mut pending.preview)
                    .filter(|preview| preview.mesh.is_some());
                let preview_view = preview
                    .as_ref()
                    .and_then(|_| self.camera.as_ref())
                    .map(crate::frustum::Frustum::new);
                let (excluded, new) = match preview {
                    Some(preview) => (
                        Some(&preview.replaced),
                        Some((
                            &mut preview.regions,
                            &mut preview.draws,
                            preview_view.as_ref(),
                            None,
                        )),
                    ),
                    None => (None, None),
                };
                let base = (
                    &mut pipeline.stream_regions,
                    &mut pipeline.region_draws,
                    view.as_ref(),
                    excluded,
                );
                for (regions, draws, camera, excluded) in std::iter::once(base).chain(new) {
                    draws.prepare(regions, camera, origin, |section| {
                        excluded.is_some_and(|set| set.contains(&(section, true)))
                            || self
                                .presentation
                                .as_ref()
                                .and_then(|p| p.columns.as_ref())
                                .is_some_and(|columns| {
                                    !columns.contains(&(
                                        section[0].div_euclid(16),
                                        section[2].div_euclid(16),
                                    ))
                                })
                    });
                    for draw in &draws.entries {
                        let local_view = &draws.views[draw.view];
                        let resident = regions.at_mut(draw.resident);
                        if collecting {
                            opaque_chunks += 1;
                        }
                        if let Some(prepared) = resident.commands.opaque(
                            device,
                            queue,
                            &command_bindings,
                            &resident.offsets.binding,
                            &resident.buffers,
                            local_view.as_ref(),
                        ) {
                            if collecting {
                                opaque_spans += prepared.draws;
                                opaque_faces += prepared.faces;
                                bundles_issued += 1;
                                bundles_rebuilt += usize::from(!prepared.reused);
                            }
                            bundles.push(prepared.bundle);
                        }
                    }
                }
                // 实体共面层在普通不透明几何之后按源序绘制；页切换仅分段，不重排。
                let preview = pipeline
                    .pending
                    .as_mut()
                    .map(|pending| &mut pending.preview);
                let (replaced, ordered) = preview.map_or((None, None), |preview| {
                    (Some(&preview.replaced), Some(&mut preview.ordered))
                });
                for (_, resident) in pipeline
                    .stream_chunks
                    .iter_mut()
                    .filter(|(key, _)| {
                        !replaced
                            .is_some_and(|set| set.contains(&upload::section_order::key(**key)))
                    })
                    .chain(ordered.into_iter().flat_map(|ordered| ordered.iter_mut()))
                    .filter(|(key, _)| {
                        key.0 .2 != 1
                            || self
                                .presentation
                                .as_ref()
                                .and_then(|p| p.columns.as_ref())
                                .is_none_or(|columns| columns.contains(&(key.0 .0, key.0 .1)))
                    })
                    .filter(|(_, resident)| !resident.ordered.segments.is_empty())
                {
                    if view.as_ref().is_some_and(|view| {
                        resident.bounds.as_ref().is_some_and(|bounds| {
                            !bounds.visible_in(&resident.offset.local_view(view))
                        })
                    }) {
                        continue;
                    }
                    if let Some(prepared) = resident.ordered_commands.opaque(
                        device,
                        queue,
                        &command_bindings,
                        resident.offset.binding(),
                        &resident.ordered,
                        None,
                    ) {
                        if collecting {
                            bundles_issued += 1;
                            bundles_rebuilt += usize::from(!prepared.reused);
                        }
                        bundles.push(prepared.bundle);
                    }
                }
                // 绊线共享已排序的常驻源，但在此通道混合并写入 Z；普通透明随后只读 Z。
                if pipeline.world_sorted.has_tripwire() {
                    for segment in &mut pipeline.world_sorted.segments {
                        if let Some(prepared) = segment.prepare_commands(
                            device,
                            &command_bindings,
                            &pipeline.zero_offset.binding,
                            &pipeline.world_sources,
                            view.as_ref(),
                            true,
                        ) {
                            if collecting {
                                tripwire_spans += prepared.draws;
                                tripwire_faces += prepared.faces;
                                bundles_issued += 1;
                                bundles_rebuilt += usize::from(!prepared.reused);
                            }
                            bundles.push(prepared.bundle);
                        }
                    }
                }
                upload::section_order::commands(
                    &pipeline.section_orders,
                    pipeline.pending.as_ref().map(|pending| &pending.preview),
                    self.camera.as_ref(),
                    device,
                    &command_bindings,
                    &pipeline.zero_offset.binding,
                    view.as_ref(),
                    true,
                    self.presentation
                        .as_ref()
                        .and_then(|p| p.columns.as_deref()),
                    |prepared| {
                        if collecting {
                            tripwire_spans += prepared.draws;
                            tripwire_faces += prepared.faces;
                            bundles_issued += 1;
                            bundles_rebuilt += usize::from(!prepared.reused);
                        }
                        bundles.push(prepared.bundle);
                    },
                );
                // 每个 bundle 自带完整绑定；execute_bundles 会清空 pass 状态，后面不混用直接绘制。
                commands::execute(&mut pass, &bundles);
                bundles.clear();
            } else if self.source_origin.is_some() {
                for ((_, _, _, _, page), chunk) in &pipeline.chunks {
                    if view.as_ref().is_some_and(|view| {
                        chunk
                            .bounds
                            .as_ref()
                            .is_some_and(|bounds| !bounds.visible_in(view))
                    }) {
                        continue;
                    }
                    let binding = pipeline.atlas_binding(*page as usize);
                    pass.set_bind_group(0, binding, &[]);
                    for ((buffer, _), spans) in chunk.buffers.iter().zip(&chunk.spans) {
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        for span in spans {
                            pass.set_pipeline(if span.depth_equal {
                                &pipeline.decal
                            } else {
                                &pipeline.opaque
                            });
                            pass.draw(span.start..span.end, 0..1);
                        }
                    }
                }
            } else {
                if pipeline.segments.is_empty() {
                    pass.set_vertex_buffer(0, pipeline.buffer.slice(..));
                    for span in bound_spans(&pipeline.draw_spans, &pipeline.atlas_pages) {
                        let binding = pipeline.atlas_binding(span.page);
                        pass.set_pipeline(if span.depth_equal {
                            &pipeline.decal
                        } else {
                            &pipeline.opaque
                        });
                        pass.set_bind_group(0, binding, &[]);
                        pass.draw(span.start..span.end, 0..1);
                    }
                } else {
                    for ((buffer, _), spans) in
                        pipeline.segments.iter().zip(&pipeline.segment_spans)
                    {
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        for span in bound_spans(spans, &pipeline.atlas_pages) {
                            let binding = pipeline.atlas_binding(span.page);
                            pass.set_pipeline(if span.depth_equal {
                                &pipeline.decal
                            } else {
                                &pipeline.opaque
                            });
                            pass.set_bind_group(0, binding, &[]);
                            pass.draw(span.start..span.end, 0..1);
                        }
                    }
                }
            }
            if self.stream_mesh.is_none() && pipeline.non_stream_has_tripwire {
                pass.set_pipeline(&pipeline.tripwire);
                pass.set_bind_group(1, &pipeline.zero_offset.binding, &[]);
                if pipeline.segments.is_empty() {
                    pass.set_vertex_buffer(0, pipeline.buffer.slice(..));
                    for span in bound_spans(&pipeline.draw_spans, &pipeline.atlas_pages) {
                        let binding = pipeline.atlas_binding(span.page);
                        pass.set_bind_group(0, binding, &[]);
                        pass.set_pipeline(pipeline.projected_pipeline(&span, Pass::Tripwire));
                        pass.draw(span.start..span.end, 0..1);
                    }
                } else {
                    for ((buffer, _), spans) in
                        pipeline.segments.iter().zip(&pipeline.segment_spans)
                    {
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        for span in bound_spans(spans, &pipeline.atlas_pages) {
                            let binding = pipeline.atlas_binding(span.page);
                            pass.set_bind_group(0, binding, &[]);
                            pass.set_pipeline(pipeline.projected_pipeline(&span, Pass::Tripwire));
                            pass.draw(span.start..span.end, 0..1);
                        }
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
                timestamp_writes: {
                    #[cfg(test)]
                    {
                        pipeline.timestamps.as_ref().map(|(query_set, base)| {
                            wgpu::RenderPassTimestampWrites {
                                query_set,
                                beginning_of_pass_write_index: Some(*base + 2),
                                end_of_pass_write_index: Some(*base + 3),
                            }
                        })
                    }
                    #[cfg(not(test))]
                    {
                        None
                    }
                },
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline.transparent);
            pass.set_bind_group(1, &pipeline.zero_offset.binding, &[]);
            if self.stream_mesh.is_some() {
                for segment in &mut pipeline.world_sorted.segments {
                    if let Some(prepared) = segment.prepare_commands(
                        device,
                        &command_bindings,
                        &pipeline.zero_offset.binding,
                        &pipeline.world_sources,
                        view.as_ref(),
                        false,
                    ) {
                        if collecting {
                            transparent_spans += prepared.draws;
                            transparent_faces += prepared.faces;
                            bundles_issued += 1;
                            bundles_rebuilt += usize::from(!prepared.reused);
                        }
                        bundles.push(prepared.bundle);
                    }
                }
                upload::section_order::commands(
                    &pipeline.section_orders,
                    pipeline.pending.as_ref().map(|pending| &pending.preview),
                    self.camera.as_ref(),
                    device,
                    &command_bindings,
                    &pipeline.zero_offset.binding,
                    view.as_ref(),
                    false,
                    self.presentation
                        .as_ref()
                        .and_then(|p| p.columns.as_deref()),
                    |prepared| {
                        if collecting {
                            transparent_spans += prepared.draws;
                            transparent_faces += prepared.faces;
                            bundles_issued += 1;
                            bundles_rebuilt += usize::from(!prepared.reused);
                        }
                        bundles.push(prepared.bundle);
                    },
                );
                commands::execute(&mut pass, &bundles);
                bundles.clear();
            } else if pipeline.segments.is_empty() {
                pass.set_vertex_buffer(0, pipeline.buffer.slice(..));
                for span in bound_spans(&pipeline.draw_spans, &pipeline.atlas_pages) {
                    let binding = pipeline.atlas_binding(span.page);
                    pass.set_bind_group(0, binding, &[]);
                    pass.set_pipeline(pipeline.projected_pipeline(&span, Pass::Translucent));
                    pass.draw(span.start..span.end, 0..1);
                }
            } else {
                for ((buffer, _), spans) in pipeline.segments.iter().zip(&pipeline.segment_spans) {
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    for span in bound_spans(spans, &pipeline.atlas_pages) {
                        let binding = pipeline.atlas_binding(span.page);
                        pass.set_bind_group(0, binding, &[]);
                        pass.set_pipeline(pipeline.projected_pipeline(&span, Pass::Translucent));
                        pass.draw(span.start..span.end, 0..1);
                    }
                }
            }
        }
        pipeline.region_draws.clear();
        pipeline.frame_commands = bundles;
        iced_wgpu::window::frame_statistics::counter("world_passes", 1);
        iced_wgpu::window::frame_statistics::counter("visible_regions", opaque_chunks as u64);
        iced_wgpu::window::frame_statistics::counter("opaque_draws", opaque_spans as u64);
        iced_wgpu::window::frame_statistics::counter("opaque_faces", opaque_faces as u64);
        iced_wgpu::window::frame_statistics::counter("transparent_faces", transparent_faces as u64);
        iced_wgpu::window::frame_statistics::counter("tripwire_faces", tripwire_faces as u64);
        iced_wgpu::window::frame_statistics::counter("commands", bundles_issued as u64);
        iced_wgpu::window::frame_statistics::counter("commands_rebuilt", bundles_rebuilt as u64);
        let command = encoder.finish();
        let encoded = encode_started.map(|start| start.elapsed());
        let submit_started = profile.then(std::time::Instant::now);
        queue.submit(Some(command));
        if let (Some(start), Some(encode)) = (started, encode_started) {
            log::debug!("[frame-profile] render at_us={} total_us={} encode_submit_us={} encode_us={} submit_us={} faces={} spans={} chunks={} visible_chunks={} opaque_spans={} transparent_visible_faces={} transparent_ranges={} tripwire_spans={} tripwire_visible_faces={} bundles={} bundles_rebuilt={} camera_changed={} direction_changed={} mesh_changed={} staged={}", FRAME_PROFILE_CLOCK.get_or_init(std::time::Instant::now).elapsed().as_micros(), start.elapsed().as_micros(), encode.elapsed().as_micros(), encoded.unwrap_or_default().as_micros(), submit_started.map_or(0, |s| s.elapsed().as_micros()), pipeline.world_faces.len(), transparent_spans, pipeline.stream_chunks.len(), opaque_chunks, opaque_spans, transparent_faces, pipeline.world_sorted.segments.iter().map(|s| s.spans().len()).sum::<usize>(), tripwire_spans, tripwire_faces, bundles_issued, bundles_rebuilt, changed_camera, changed_direction, mesh_identity_changed, staged);
        }
    }
    fn draw_ready(&self, pipeline: &Pipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        if pipeline.light_invalid {
            return false;
        }
        // iced owns the actual swapchain pass. Only sample the completed
        // depth-tested offscreen result into the shader widget's viewport.
        pass.set_pipeline(&pipeline.composite);
        pass.set_bind_group(1, &pipeline.zero_offset.binding, &[]);
        pass.set_bind_group(0, &pipeline.composite_bind_group, &[]);
        pass.draw(0..6, 0..1);
        true
    }
}
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
    world::run_test_category(category);
    redraw_tests::run_test_category(category);
    hardware_tests::run_test_category(category);
}

#[cfg(test)]
#[path = "gpu_redraw_tests.rs"]
mod redraw_tests;
#[cfg(test)]
mod tests {
    use super::*;

    fn partial_scene_handoff_preserves_previous_coverage_atlas_and_source_origin() {
        let (_, scene, old, _) = crate::picking_integration_tests::fixture();
        let previous = crate::spectator::CompletedFrame {
            vertices: Arc::new(Vec::new()),
            stream_mesh: Some(Arc::clone(&old)),
            surface: Arc::clone(&scene.surface),
            atlas: old.atlas.clone(),
        };
        let mut composite = (*old).clone();
        composite.terrain = Some(Arc::clone(&old));
        let composite = Arc::new(composite);
        let mut fallback_scene = scene.clone();
        fallback_scene.cache = Default::default();
        fallback_scene.snapshot = None;
        fallback_scene.entity_enabled = true;
        fallback_scene.fallback = Some(crate::spectator::CompletedFrame {
            stream_mesh: Some(Arc::clone(&composite)),
            ..previous.clone()
        });
        assert!(Arc::ptr_eq(
            world_candidate(&fallback_scene)
                .stream_mesh
                .as_ref()
                .unwrap(),
            &composite
        ));
        fallback_scene.entity_enabled = false;
        assert!(Arc::ptr_eq(
            world_candidate(&fallback_scene)
                .stream_mesh
                .as_ref()
                .unwrap(),
            &old
        ));
        let mut high = (*scene.surface).clone();
        high.y_min += 10;
        high.y_max += 10;
        high.voxels[0].y += 10;
        let high_scene = crate::spectator::SpectatorScene {
            surface: Arc::new(high),
            ..scene.clone()
        };
        let mut high = crate::spectator::build_scene_cancellable(&high_scene, &|| false).unwrap();
        high.complete = false;
        high.atlas = Some(Arc::new(TextureAtlas::empty_test_atlas()));
        let high = Arc::new(high);
        let held = choose_spectator_mesh(
            None,
            &high_scene.surface,
            high.atlas.clone(),
            Some(Arc::clone(&high)),
            Some(&previous),
        );
        assert!(Arc::ptr_eq(held.stream_mesh.as_ref().unwrap(), &old));
        assert_eq!(held.origin(), old.origin);
        assert!(held.atlas.is_none());
        let first = choose_spectator_mesh(
            None,
            &high_scene.surface,
            high.atlas.clone(),
            Some(Arc::clone(&high)),
            None,
        );
        assert!(
            Arc::ptr_eq(first.stream_mesh.as_ref().unwrap(), &high),
            "首次加载仍允许渐进显示"
        );
        let mut rebased = (*scene.surface).clone();
        rebased.x -= 16;
        rebased.z -= 16;
        rebased.y_min -= 8;
        let rebased_scene = crate::spectator::SpectatorScene {
            surface: Arc::new(rebased),
            ..scene
        };
        let mut rebased =
            crate::spectator::build_scene_cancellable(&rebased_scene, &|| false).unwrap();
        rebased.complete = false;
        let rebased = Arc::new(rebased);
        let promoted = choose_spectator_mesh(
            None,
            &rebased_scene.surface,
            None,
            Some(Arc::clone(&rebased)),
            Some(&previous),
        );
        assert!(
            Arc::ptr_eq(promoted.stream_mesh.as_ref().unwrap(), &rebased),
            "已覆盖原场景时不阻塞新增区块的渐进发布"
        );
        let mut complete = (*high).clone();
        complete.complete = true;
        let complete = Arc::new(complete);
        let promoted = choose_spectator_mesh(
            None,
            &high_scene.surface,
            complete.atlas.clone(),
            Some(Arc::clone(&complete)),
            Some(&previous),
        );
        assert!(
            Arc::ptr_eq(promoted.stream_mesh.as_ref().unwrap(), &complete),
            "已完成的新快照可以收缩或改变源范围"
        );
    }

    fn atlas_page_spans_preserve_face_order_across_independent_page_bindings() {
        let mut quads = Vec::new();
        for page in [0, 1, 1, 260, 0] {
            quads.extend(gpu_test_quad(page as f32).into_iter().map(|mut v| {
                v.atlas_page = page;
                v
            }));
        }
        let spans = page_spans(&quads);
        assert_eq!(
            spans,
            vec![
                DrawSpan {
                    depth_equal: false,
                    additive: false,
                    page: 0,
                    start: 0,
                    end: 6
                },
                DrawSpan {
                    depth_equal: false,
                    additive: false,
                    page: 1,
                    start: 6,
                    end: 18
                },
                DrawSpan {
                    depth_equal: false,
                    additive: false,
                    page: 260,
                    start: 18,
                    end: 24
                },
                DrawSpan {
                    depth_equal: false,
                    additive: false,
                    page: 0,
                    start: 24,
                    end: 30
                },
            ]
        );
    }

    fn spectator_chunk_identity_keeps_negative_world_coordinates() {
        let quad = gpu_test_quad(2.0);
        assert_eq!(chunk_bucket(&quad, [-32, 0, -32]), (-2, -2, 0));
        assert!(same_vertices(&quad, &quad));
        let mut changed = quad;
        changed[0].atlas_page += 1;
        assert!(!same_vertices(&quad, &changed));
    }

    fn spectator_mesh_handoff_retains_previous_complete_vertices_and_origin() {
        use mcedit2_world::surface::SurfaceView;
        let source = |x: i32| {
            Arc::new(SurfaceView {
                biomes: Default::default(),
                x,
                z: 0,
                y_min: 64,
                y_max: 111,
                width: 16,
                depth: 16,
                voxels: vec![],
                source_tiles: vec![],
                source_chunks: 0,
                missing_columns: 0,
            })
        };
        let old_origin = source(0);
        let new_origin = source(64);
        let old_mesh = Arc::new(vec![Vertex {
            xy: [0.0; 2],
            depth: 0.0,
            rgba: [1.0; 4],
            uv: [0.0; 2],
            mode: 0,
            xyz: [0.5, 0.0, 0.0],
            world_mode: 1,
            atlas_page: 0,
            light: crate::lighting::FULLBRIGHT,
        }]);
        let fallback = crate::spectator::CompletedFrame {
            vertices: Arc::clone(&old_mesh),
            stream_mesh: None,
            surface: Arc::clone(&old_origin),
            atlas: None,
        };
        let during = choose_spectator_mesh(None, &new_origin, None, None, Some(&fallback));
        assert!(Arc::ptr_eq(&during.vertices, &old_mesh));
        assert!(Arc::ptr_eq(&during.surface, &old_origin));
        let replacement = Arc::new(vec![crate::world_mesh::WorldQuad::from_vertices(
            &gpu_test_quad(1.0),
        )
        .unwrap()]);
        let replacement_mesh = Arc::new(crate::spectator::StreamMesh {
            terrain: None,
            visibility: None,
            entities: None,
            complete: true,
            origin: [new_origin.x, new_origin.y_min, new_origin.z],
            atlas: None,
            chunks: vec![crate::spectator::MeshChunk::new(
                (0, 0, 1, 0, 0),
                [0; 3],
                Arc::clone(&replacement),
                Default::default(),
            )]
            .into(),
        });
        let ready = choose_spectator_mesh(
            Some(Arc::clone(&replacement_mesh)),
            &new_origin,
            None,
            None,
            Some(&fallback),
        );
        assert!(Arc::ptr_eq(
            ready.stream_mesh.as_ref().unwrap(),
            &replacement_mesh
        ));
        assert!(Arc::ptr_eq(
            &ready.stream_mesh.as_ref().unwrap().chunks[0].quads(),
            &replacement
        ));
        assert!(Arc::ptr_eq(&ready.surface, &new_origin));
    }

    fn completed_stream_generation_keeps_separate_chunks_and_original_atlas() {
        let original_atlas = Arc::new(TextureAtlas::empty_test_atlas());
        let new_atlas = Arc::new(TextureAtlas::empty_test_atlas());
        let original = Arc::new(crate::spectator::StreamMesh {
            terrain: None,
            visibility: None,
            entities: None,
            complete: true,
            atlas: Some(Arc::clone(&original_atlas)),
            origin: [-16, 64, 0],
            chunks: vec![
                crate::spectator::MeshChunk::new(
                    (-1, 0, 1, 0, 0),
                    [0; 3],
                    Arc::new(vec![crate::world_mesh::WorldQuad::from_vertices(
                        &gpu_test_quad(1.0),
                    )
                    .unwrap()]),
                    Default::default(),
                ),
                crate::spectator::MeshChunk::new(
                    (0, 0, 4, 1, 0),
                    [0; 3],
                    Arc::new(vec![crate::world_mesh::WorldQuad::from_vertices(
                        &gpu_test_quad(2.0),
                    )
                    .unwrap()]),
                    Default::default(),
                ),
            ]
            .into(),
        });
        let source = Arc::new(mcedit2_world::surface::SurfaceView {
            biomes: Default::default(),
            x: -16,
            z: 0,
            y_min: 64,
            y_max: 128,
            width: 16,
            depth: 16,
            voxels: vec![],
            source_tiles: vec![],
            source_chunks: 0,
            missing_columns: 0,
        });
        let committed = choose_spectator_mesh(
            None,
            &source,
            Some(new_atlas),
            Some(Arc::clone(&original)),
            None,
        );
        assert!(Arc::ptr_eq(
            committed.atlas.as_ref().unwrap(),
            &original_atlas
        ));
        assert!(Arc::ptr_eq(
            committed.stream_mesh.as_ref().unwrap(),
            &original
        ));
        assert!(committed.vertices.is_empty());
        assert_eq!(committed.stream_mesh.unwrap().chunks.len(), 2);
    }

    fn streamed_camera_keeps_mesh_origin_when_source_view_has_moved() {
        let source = Arc::new(mcedit2_world::surface::SurfaceView {
            biomes: Default::default(),
            x: 32,
            z: -48,
            y_min: -64,
            y_max: 128,
            width: 16,
            depth: 16,
            voxels: vec![],
            source_tiles: vec![],
            source_chunks: 0,
            missing_columns: 0,
        });
        let mesh = Arc::new(crate::spectator::StreamMesh {
            terrain: None,
            visibility: None,
            entities: None,
            complete: true,
            atlas: None,
            origin: [-16, 64, 0],
            chunks: vec![].into(),
        });
        let committed = choose_spectator_mesh(None, &source, None, Some(mesh), None);
        assert_eq!(committed.origin(), [-16, 64, 0]);
        let camera = crate::spectator::Camera {
            position: [-10.5, 70.0, 2.25],
            ..Default::default()
        };
        let uniform = camera.uniform_at_origin(committed.origin(), Size::new(800.0, 600.0));
        assert_eq!(uniform.position, [5.5, 6.0, 2.25, 0.0]);
        let next = Arc::new(crate::spectator::StreamMesh {
            terrain: None,
            visibility: None,
            entities: None,
            complete: true,
            atlas: None,
            origin: [32, -64, -48],
            chunks: vec![].into(),
        });
        let replacement = choose_spectator_mesh(None, &source, None, Some(next), Some(&committed));
        assert_eq!(replacement.origin(), [32, -64, -48]);
        let uniform = camera.uniform_at_origin(replacement.origin(), Size::new(800.0, 600.0));
        assert_eq!(uniform.position, [-42.5, 134.0, 50.25, 0.0]);
        let waiting = choose_spectator_mesh(None, &source, None, None, Some(&committed));
        assert_eq!(waiting.origin(), [-16, 64, 0]);
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
            light: crate::lighting::FULLBRIGHT,
        })
    }

    fn perspective_sort_follows_eye_position_and_retains_order_when_turning() {
        let forward = test_camera([0.0, 0.0, 1.0]);
        let mut moved = forward;
        moved.position = [0.0, 0.0, 20.0, 0.0];
        assert_eq!(forward.forward, moved.forward);
        assert_ne!(forward, moved);
        let rotated = test_camera([1.0, 0.0, 0.0]);
        assert_ne!(forward.forward, rotated.forward);
        let near = test_quad(1.0, 2.0, 2, 1);
        let far = test_quad(2.0, 8.0, 2, 1);
        let faces = collect_transparent_quads([near.as_slice(), far.as_slice()]);
        let mut order = Vec::new();
        let initial = sorted_transparent_quads(&faces, &mut order, Some(&forward));
        let translated = sorted_transparent_quads(&faces, &mut order, Some(&moved));
        assert_eq!(translated[..6], initial[6..]);
        assert_eq!(translated[6..], initial[..6]);
        assert_eq!(
            sorted_transparent_quads(&faces, &mut order, Some(&rotated)),
            initial
        );
    }

    fn extracted_translucent_faces_match_original_global_quad_sort() {
        let mut first = Vec::new();
        first.extend(test_quad(1.0, 1.0, 0, 1));
        first.extend(test_quad(2.0, 10.0, 2, 1));
        let mut second = Vec::new();
        second.extend(test_quad(3.0, 3.0, 2, 1));
        second.extend(test_quad(4.0, 0.0, 1, 1));
        let cached = collect_transparent_quads([first.as_slice(), second.as_slice()]);
        assert_eq!(cached.len(), 2);
        let mut order = Vec::new();
        let camera = test_camera([0.0, 0.0, 1.0]);
        let sorted = sorted_transparent_quads(&cached, &mut order, Some(&camera));
        assert_eq!(sorted.len(), 12);
        assert_eq!(sorted[0].atlas_page, 2);
        assert_eq!(sorted[6].atlas_page, 3);
        let reversed = test_camera([0.0, 0.0, -1.0]);
        let resorted = sorted_transparent_quads(&cached, &mut order, Some(&reversed));
        assert_eq!(resorted, sorted);
        assert_eq!(
            sorted_transparent_quads(&cached, &mut order, Some(&reversed)),
            resorted
        );
    }

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
        // 透视原地转头不改变距离顺序。
        let reversed = ordered_transparent_vertices(&vertices, &test_camera([0.0, 0.0, -1.0]));
        assert_eq!(reversed, sorted);
    }

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
            light: crate::lighting::FULLBRIGHT,
        }; 6]
    }

    fn hardware_mesh_segmentation_preserves_complete_quads() {
        let stride = std::mem::size_of::<Vertex>() as u64;
        assert_eq!(gpu_segment_vertices(stride * 6), 6);
        assert_eq!(gpu_segment_vertices(stride * 16), 12);
        assert_eq!(gpu_segment_vertices(stride * 5), 0);
        let full = (0..30).collect::<Vec<_>>();
        let rebuilt = full
            .chunks(gpu_segment_vertices(stride * 12))
            .flat_map(|chunk| chunk.iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(rebuilt, full);
    }

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

    fn equal_length_chunk_edits_only_upload_local_dirty_faces() {
        let original: Vec<Vertex> = (0..256).flat_map(|i| gpu_test_quad(i as f32)).collect();
        let mut updated = original.clone();
        updated[6].atlas_page = 12;
        updated[255 * 6].atlas_page = 34;
        let (changed, ranges) = mesh_upload_ranges(Some(&original), &updated, false);
        assert!(changed);
        assert_eq!(ranges, vec![6..12, 255 * 6..256 * 6]);
        assert!(ranges.iter().map(|r| r.len()).sum::<usize>() < updated.len());
        assert_eq!(
            mesh_upload_ranges(Some(&updated), &updated, false),
            (false, vec![])
        );
        assert_eq!(
            mesh_upload_ranges(None, &updated, false),
            (true, std::iter::once(0..updated.len()).collect())
        );
    }

    fn shifted_geometry_uploads_suffix_and_shrinking_only_updates_draw_count() {
        let a = gpu_test_quad(1.0);
        let b = gpu_test_quad(2.0);
        let c = gpu_test_quad(3.0);
        let old = [a.as_slice(), b.as_slice(), c.as_slice()].concat();
        let shifted = [a.as_slice(), c.as_slice()].concat();
        assert_eq!(
            mesh_upload_ranges(Some(&old), &shifted, false),
            (true, std::iter::once(6..12).collect())
        );
        let removed_end = [a.as_slice(), b.as_slice()].concat();
        assert_eq!(
            mesh_upload_ranges(Some(&old), &removed_end, false),
            (true, vec![])
        );
        assert_eq!(
            mesh_upload_ranges(Some(&old), &old, true),
            (true, std::iter::once(0..old.len()).collect())
        );
    }

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
            light: crate::lighting::FULLBRIGHT,
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

    fn render_target_dimensions_are_capped_and_nonzero() {
        let rect = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 750.2,
            height: 360.5,
        };
        assert_eq!(
            crate::render_viewport::RenderViewport::at_scale(rect.size(), 1., 8192).raster,
            Size::new(751, 361)
        );
        let empty = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
        assert_eq!(
            crate::render_viewport::RenderViewport::at_scale(empty.size(), 1., 8192).raster,
            Size::new(1, 1)
        );
        let massive = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 9000.0,
            height: 9000.0,
        };
        assert_eq!(
            crate::render_viewport::RenderViewport::at_scale(massive.size(), 1., 4096).raster,
            Size::new(4096, 4096)
        );
        assert_eq!(
            crate::render_viewport::RenderViewport::at_scale(massive.size(), 1., 8192).raster,
            Size::new(8192, 8192)
        );
        for (scale, expected) in [
            (1.25, Size::new(938, 451)),
            (1.5, Size::new(1126, 541)),
            (2., Size::new(1501, 721)),
        ] {
            assert_eq!(
                crate::render_viewport::RenderViewport::at_scale(rect.size(), scale, 8192).raster,
                expected
            );
        }
        let capped =
            crate::render_viewport::RenderViewport::at_scale(Size::new(9000., 4500.), 2., 4096);
        assert_eq!(capped.raster, Size::new(4096, 2048));
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Rendering {
            partial_scene_handoff_preserves_previous_coverage_atlas_and_source_origin();
            atlas_page_spans_preserve_face_order_across_independent_page_bindings();
            spectator_chunk_identity_keeps_negative_world_coordinates();
            spectator_mesh_handoff_retains_previous_complete_vertices_and_origin();
            completed_stream_generation_keeps_separate_chunks_and_original_atlas();
            streamed_camera_keeps_mesh_origin_when_source_view_has_moved();
            perspective_sort_follows_eye_position_and_retains_order_when_turning();
            extracted_translucent_faces_match_original_global_quad_sort();
            transparent_quads_sort_as_whole_original_model_faces();
            gltf_single_sided_blend_mode_keeps_back_to_front_transparency_sort();
            transparent_equal_depth_is_deterministic_and_keeps_triangle_pairs();
            hardware_mesh_segmentation_preserves_complete_quads();
            gpu_upload_avoids_rewriting_unchanged_prefix_and_handles_shorter_mesh();
            equal_length_chunk_edits_only_upload_local_dirty_faces();
            shifted_geometry_uploads_suffix_and_shrinking_only_updates_draw_count();
            replacing_middle_chunk_uploads_entire_shifted_suffix();
            bitwise_vertex_comparison_is_stable_across_identical_allocations();
            vertex_world_depth_maps_nearest_to_zero_and_farthest_to_one();
            render_target_dimensions_are_capped_and_nonzero();
        }
    }
}

#[path = "gpu_visible_mesh.rs"]
mod visible_mesh;

#[cfg(test)]
#[path = "gpu_validation.rs"]
pub(crate) mod hardware_tests;
