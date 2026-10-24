//! 2D overhead overview of the exact same original source surface used by
//! the 3D renderer. Never read another chunk, invent distant terrain or
//! return an approximate LOD sample to the editor.
use std::{collections::{BTreeMap,HashMap},sync::Arc};
use iced::{Color,Point,Size,Rectangle,Renderer,Theme,mouse};
use iced::widget::canvas::{self,Frame,Path,Stroke};
use mcedit2_world::surface::{SurfaceView,SurfaceVoxel};
use crate::{Message,palette};

#[derive(Clone)]
pub(crate) struct TopDown{
    pub source:Arc<SurfaceView>,
    pub zoom:f32,pub pan_x:f32,pub pan_y:f32,
    pub selected:Option<(i32,i32,i32)>,
}
struct Projection{scale:f32,origin:Point,x:i32,z:i32}
impl Projection{
    fn new(v:&SurfaceView,size:Size,zoom:f32,pan_x:f32,pan_y:f32)->Option<Self>{
        if v.width==0||v.depth==0||size.width<=0.0||size.height<=0.0{return None;}
        let scale=(size.width/v.width as f32)
            .min(size.height/v.depth as f32)*zoom.clamp(0.5,3.0);
        if !scale.is_finite()||scale<=0.0{return None;}
        Some(Self{
            scale,x:v.x,z:v.z,
            origin:Point::new((size.width-v.width as f32*scale)*0.5+pan_x,
                (size.height-v.depth as f32*scale)*0.5+pan_y),
        })
    }
    fn point(&self,x:f64,z:f64)->Point{
        // Only convert the delta to f32, not the +/-30m source coordinate.
        Point::new(self.origin.x+(x-self.x as f64) as f32*self.scale,
            self.origin.y+(z-self.z as f64) as f32*self.scale)
    }
    fn cell(&self,p:Point)->Option<(i32,i32)>{
        let dx=((p.x-self.origin.x)/self.scale).floor() as i64;
        let dz=((p.y-self.origin.y)/self.scale).floor() as i64;
        Some((i32::try_from(self.x as i64+dx).ok()?,
            i32::try_from(self.z as i64+dz).ok()?))
    }
}
fn tint(v:&SurfaceVoxel)->Color{
    palette::tint(&v.state,v.kind,true)
        .unwrap_or_else(||palette::unknown_tint(&v.state,v.kind))
}
fn exact_visible(v:&SurfaceView)->BTreeMap<(i32,i32),usize>{
    let mut tops:BTreeMap<(i32,i32),usize>=BTreeMap::new();
    for (index,voxel) in v.voxels.iter().enumerate(){
        if voxel.span!=1{continue;}
        let key=(voxel.x,voxel.z);
        if tops.get(&key).is_none_or(|&old|v.voxels[old].y<voxel.y){
            tops.insert(key,index);
        }
    }
    tops
}
impl TopDown{
    fn pick(&self,p:Point,size:Size)->Option<(i32,i32,i32)>{
        if self.source.width>64||self.source.depth>64{return None;}
        let proj=Projection::new(&self.source,size,self.zoom,self.pan_x,self.pan_y)?;
        let (x,z)=proj.cell(p)?;
        if x<self.source.x||z<self.source.z
            ||x as i64>=self.source.x as i64+self.source.width as i64
            ||z as i64>=self.source.z as i64+self.source.depth as i64{
            return None;
        }
        exact_visible(&self.source).get(&(x,z)).map(|&index|
            (x,self.source.voxels[index].y,z))
    }
}
impl canvas::Program<Message> for TopDown{
    type State=();
    fn draw(&self,_:&(),renderer:&Renderer,_:&Theme,bounds:Rectangle,
        _cursor:mouse::Cursor)->Vec<canvas::Geometry>{
        let mut frame=Frame::new(renderer,bounds.size());
        let Some(p)=Projection::new(&self.source,bounds.size(),
            self.zoom,self.pan_x,self.pan_y)else{return vec![frame.into_geometry()]};
        if self.source.width<=64&&self.source.depth<=64{
            for (_,index) in exact_visible(&self.source){
                let voxel=&self.source.voxels[index];
                let origin=p.point(voxel.x as f64,voxel.z as f64);
                if origin.x+p.scale<0.0||origin.y+p.scale<0.0
                    ||origin.x>bounds.width||origin.y>bounds.height{continue;}
                let rect=Path::rectangle(origin,Size::new(p.scale,p.scale));
                frame.fill(&rect,tint(voxel));
                if self.selected==Some((voxel.x,voxel.y,voxel.z)){
                    frame.stroke(&rect,Stroke::default().with_width(2.0)
                        .with_color(Color::from_rgb(0.1,1.0,0.9)));
                }
            }
        }else{
            // True far chunks often occupy less than a pixel. Coalesce
            // only verified sampled columns into bounded screen cells.
            let mut pixels:HashMap<(i32,i32),usize>=HashMap::new();
            for (index,voxel) in self.source.voxels.iter().enumerate(){
                let half=(voxel.span as f64-1.0)*0.5;
                let q=p.point(voxel.x as f64+half,voxel.z as f64+half);
                if !q.x.is_finite()||!q.y.is_finite()
                    ||q.x<0.0||q.y<0.0||q.x>=bounds.width||q.y>=bounds.height{
                    continue;
                }
                let key=((q.x/2.0) as i32,(q.y/2.0) as i32);
                if pixels.get(&key).is_none_or(|&old|
                    self.source.voxels[old].y<voxel.y){
                    pixels.insert(key,index);
                }
            }
            for ((x,y),index) in pixels{
                frame.fill(&Path::rectangle(Point::new(
                    x as f32*2.0,y as f32*2.0),Size::new(2.0,2.0)),
                    tint(&self.source.voxels[index]));
            }
        }
        vec![frame.into_geometry()]
    }
    fn update(&self,_:&mut (),event:&canvas::Event,bounds:Rectangle,
        cursor:mouse::Cursor)->Option<canvas::Action<Message>>{
        match event{
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))=>{
                let xyz=self.pick(cursor.position_in(bounds)?,bounds.size())?;
                Some(canvas::Action::publish(Message::PickSurface(
                    xyz.0,xyz.1,xyz.2)).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled{delta})=>{
                cursor.position_in(bounds)?;
                let dy=match *delta{
                    mouse::ScrollDelta::Lines{y,..}=>y,
                    mouse::ScrollDelta::Pixels{y,..}=>y/80.0,
                };
                if dy.abs()<0.05{return None;}
                Some(canvas::Action::publish(Message::ZoomSurface(
                    if dy>0.0{1}else{-1})).and_capture())
            }
            _=>None,
        }
    }
    fn mouse_interaction(&self,_:&(),bounds:Rectangle,cursor:mouse::Cursor)
        ->mouse::Interaction{
        if cursor.position_in(bounds)
            .and_then(|point|self.pick(point,bounds.size())).is_some(){
            mouse::Interaction::Pointer
        }else{mouse::Interaction::default()}
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    use mcedit2_world::sections::State;
    use mcedit2_world::surface::{VoxelMaterial,ALL_FACES};
    fn scene()->SurfaceView{
        SurfaceView{x:-16,z:29_999_980,width:16,depth:16,
            y_min:-64,y_max:319,missing_columns:0,source_chunks:1,
            source_tiles:vec![],voxels:vec![SurfaceVoxel{
                x:-16,y:100,z:29_999_980,span:1,
                state:State{name:"minecraft:stone".into(),properties:vec![]},
                original_block_entity:false,staged:false,
                faces:ALL_FACES,kind:VoxelMaterial::Solid,
            }]}
    }
    #[test]fn signed_world_border_projected_coordinates_roundtrip(){
        let s=scene();let size=Size::new(800.0,440.0);
        let p=Projection::new(&s,size,1.0,0.0,0.0).unwrap();
        let q=p.point(-15.5,29_999_980.5);
        assert_eq!(p.cell(q),Some((-16,29_999_980)));
    }
    #[test]fn exact_picks_never_promote_sparse_far_samples(){
        let mut s=scene();
        let mut lower=s.voxels[0].clone();lower.y=80;s.voxels.push(lower);
        let view=TopDown{source:Arc::new(s),zoom:1.0,
            pan_x:0.0,pan_y:0.0,selected:None};
        let size=Size::new(800.0,440.0);
        let p=Projection::new(&view.source,size,1.0,0.0,0.0).unwrap()
            .point(-15.5,29_999_980.5);
        assert_eq!(view.pick(p,size),Some((-16,100,29_999_980)));
        let mut far=(*view.source).clone();
        far.width=8192;far.depth=8192;far.voxels[0].span=16;
        let far_view=TopDown{source:Arc::new(far),..view};
        assert!(far_view.pick(Point::new(400.0,220.0),size).is_none());
    }
}
