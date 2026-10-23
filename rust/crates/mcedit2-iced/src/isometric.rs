//! Interactive isometric projection of ORIGINAL Java block surface cells.
//! Untextured polygon rendering is an intermediate visual editor milestone;
//! every pick uses exact original XYZ and the existing editing safety checks.
use iced::{Color,Point,Rectangle,Renderer,Theme,mouse};
use iced::widget::canvas::{self,Path,Frame};
use mcedit2_world::surface::{SurfaceView,SurfaceVoxel};
use crate::Message;

#[derive(Debug,Clone)]
pub struct IsometricSurface {pub surface:SurfaceView,pub camera_quarters:u8}
fn color(block:&SurfaceVoxel)->Color{
    if block.original_block_entity{return Color::from_rgb(0.98,0.60,0.19);}
    let mut hash=0x811c9dc5u32;
    for b in block.state.name.as_bytes(){
        hash=(hash ^ (*b as u32)).wrapping_mul(16_777_619);
    }
    let r=0.28+((hash>>16&255) as f32/255.0)*0.52;
    let g=0.28+((hash>>8&255) as f32/255.0)*0.52;
    let b=0.28+((hash&255) as f32/255.0)*0.52;
    Color::from_rgb(r,g,b)
}
fn tint(base:Color,m:f32)->Color{
    Color::from_rgba(base.r*m,base.g*m,base.b*m,1.0)
}
fn polygon(frame:&mut Frame,points:&[Point],shade:Color){
    let path=Path::new(|builder|{
        builder.move_to(points[0]);
        for point in &points[1..]{builder.line_to(*point);}
        builder.close();
    });
    frame.fill(&path,shade);
}
fn rotated(voxel:&SurfaceVoxel,view:&SurfaceView,camera:u8)->(f32,f32){
    let x=(voxel.x-view.x) as f32;
    let z=(voxel.z-view.z) as f32;
    let width=(view.width-1) as f32;
    let depth=(view.depth-1) as f32;
    match camera%4{
        0=>(x,z),
        1=>(depth-z,x),
        2=>(width-x,depth-z),
        _=>(z,width-x),
    }
}
fn center(voxel:&SurfaceVoxel,view:&SurfaceView,camera:u8,size:iced::Size)->Point{
    // Orthographic isometric XYZ projection with natural vertical heights.
    let (px,pz)=rotated(voxel,view,camera);
    let py=(voxel.y-view.y_min) as f32;
    Point::new(
        size.width/2.0+(px-pz)*12.0,
        54.0+(px+pz)*6.0-py*7.0
    )
}
fn top_diamond(c:Point)->[Point;4]{
    [
        Point::new(c.x,c.y-12.0),
        Point::new(c.x+12.0,c.y-6.0),
        Point::new(c.x,c.y),
        Point::new(c.x-12.0,c.y-6.0),
    ]
}
fn contained(point:Point,vertices:&[Point])->bool {
    let mut count=0;
    for index in 0..vertices.len(){
        let a=vertices[index];let b=vertices[(index+1)%vertices.len()];
        if (a.y>point.y)!=(b.y>point.y)
            &&point.x<(b.x-a.x)*(point.y-a.y)/(b.y-a.y)+a.x{
            count+=1;
        }
    }
    count%2==1
}
impl IsometricSurface {
    fn ordered(&self)->Vec<&SurfaceVoxel>{
        let mut entries=self.surface.voxels.iter().collect::<Vec<_>>();
        entries.sort_by(|a,b|{
            let (ax,az)=rotated(a,&self.surface,self.camera_quarters);
            let (bx,bz)=rotated(b,&self.surface,self.camera_quarters);
            (ax+az).total_cmp(&(bx+bz))
        });
        entries
    }
    fn pick(&self,point:Point,size:iced::Size)->Option<(i32,i32,i32)> {
        // Reverse painter's order so the nearest visible top face wins.
        self.ordered().into_iter().rev().find_map(|voxel|{
            let diamond=top_diamond(center(
                voxel,&self.surface,self.camera_quarters,size
            ));
            if contained(point,&diamond){Some((voxel.x,voxel.y,voxel.z))}
            else{None}
        })
    }
}
impl canvas::Program<Message> for IsometricSurface {
    type State=();
    fn draw(&self,_state:&(),renderer:&Renderer,_theme:&Theme,
        bounds:Rectangle,_cursor:mouse::Cursor)->Vec<canvas::Geometry>{
        let mut frame=Frame::new(renderer,bounds.size());
        for voxel in self.ordered(){
            let center=center(voxel,&self.surface,self.camera_quarters,bounds.size());
            let top=top_diamond(center);
            let shade=color(voxel);
            let down=Point::new(0.0,12.0);
            polygon(&mut frame,&[
                top[3],top[2],
                Point::new(top[2].x+down.x,top[2].y+down.y),
                Point::new(top[3].x+down.x,top[3].y+down.y),
            ],tint(shade,0.62));
            polygon(&mut frame,&[
                top[1],top[2],
                Point::new(top[2].x+down.x,top[2].y+down.y),
                Point::new(top[1].x+down.x,top[1].y+down.y),
            ],tint(shade,0.78));
            polygon(&mut frame,&top,shade);
        }
        vec![frame.into_geometry()]
    }
    fn update(&self,_state:&mut (),event:&canvas::Event,
        bounds:Rectangle,cursor:mouse::Cursor)->Option<canvas::Action<Message>>{
        if let canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))=event{
            let point=cursor.position_in(bounds)?;
            return self.pick(point,bounds.size())
                .map(|(x,y,z)|canvas::Action::publish(Message::PickSurface(x,y,z)).and_capture());
        }
        None
    }
    fn mouse_interaction(&self,_state:&(),bounds:Rectangle,cursor:mouse::Cursor)
        ->mouse::Interaction{
        if cursor.position_in(bounds)
            .is_some_and(|point|self.pick(point,bounds.size()).is_some()){
            mouse::Interaction::Pointer
        }else{mouse::Interaction::default()}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use mcedit2_world::sections::State;
    #[test]
    fn iso_picking_uses_original_xyz_including_negative_heights(){
        let voxel=SurfaceVoxel{
            x:2,y:-63,z:3,
            state:State{name:"minecraft:stone".into(),properties:vec![]},
            original_block_entity:false
        };
        let view=SurfaceView{x:2,z:3,y_min:-64,y_max:-49,width:1,depth:1,
            voxels:vec![voxel.clone()],missing_columns:0,source_chunks:1};
        let s=iced::Size::new(700.0,420.0);
        for quarter in 0..4{
            let c=center(&voxel,&view,quarter,s);
            assert_eq!(
                IsometricSurface{surface:view.clone(),camera_quarters:quarter}
                    .pick(Point::new(c.x,c.y-6.0),s),
                Some((2,-63,3))
            );
        }
    }
}
