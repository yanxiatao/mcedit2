//! Conservative visual geometry for common vanilla non-cube blockstates.
//! Not a replacement for arbitrary resource-pack JSON block models.
//! Unsupported/modded blocks remain full cubes for selection safety.
use mcedit2_world::sections::State;
#[derive(Debug,Clone,Copy,PartialEq)]
pub(crate) struct Box3{
    pub min:[f32;3],
    pub max:[f32;3],
}
impl Box3{
    pub const FULL:Self=Self{min:[-0.5,-0.5,-0.5],max:[0.5,0.5,0.5]};
    /// Vanilla model JSON uses coordinates in the [0,16] cube. Refuse
    /// malformed/inverted/nonfinite shapes, not approximate them.
    pub fn from_model_units(from:[f32;3],to:[f32;3])->Option<Self>{
        if (0..3).any(|i|!from[i].is_finite()||!to[i].is_finite()
            ||from[i]<0.0||to[i]>16.0||from[i]>=to[i]){return None;}
        Some(Self{min:from.map(|v|v/16.0-0.5),max:to.map(|v|v/16.0-0.5)})
    }
    /// Java allows a model element to be a zero-thickness quad (saplings,
    /// crossed flowers, tall grass). Exactly one axis may collapse; the
    /// resource JSON still must explicitly declare its visible faces.
    pub fn from_model_units_plane(from:[f32;3],to:[f32;3])->Option<Self>{
        let mut collapsed=0;
        for axis in 0..3{
            if !from[axis].is_finite()||!to[axis].is_finite()
                ||from[axis]<0.0||to[axis]>16.0||from[axis]>to[axis]{
                return None;
            }
            if from[axis]==to[axis]{collapsed+=1;}
        }
        if collapsed!=1{return None;}
        Some(Self{min:from.map(|v|v/16.0-0.5),
            max:to.map(|v|v/16.0-0.5)})
    }
    /// Java blockstate variant x/y quarter rotations about block center.
    /// Only exact axis-aligned angles are accepted. No enclosing AABB is
    /// substituted for 22.5/45 degree element rotations.
    pub fn rotate(&self,x:i32,y:i32)->Option<Self>{
        if x%90!=0||y%90!=0||x.abs()>360||y.abs()>360{return None;}
        let mut min=[f32::INFINITY;3];
        let mut max=[f32::NEG_INFINITY;3];
        for i in 0..8{
            let mut p=[
                if i&1==0{self.min[0]}else{self.max[0]},
                if i&2==0{self.min[1]}else{self.max[1]},
                if i&4==0{self.min[2]}else{self.max[2]},
            ];
            for _ in 0..x.rem_euclid(360)/90{
                p=[p[0],-p[2],p[1]];
            }
            for _ in 0..y.rem_euclid(360)/90{
                p=[-p[2],p[1],p[0]];
            }
            for axis in 0..3{
                min[axis]=min[axis].min(p[axis]);
                max[axis]=max[axis].max(p[axis]);
            }
        }
        Some(Self{min,max})
    }
    fn from_six(x0:f32,y0:f32,z0:f32,x1:f32,y1:f32,z1:f32)->Self{
        Self{min:[x0,y0,z0],max:[x1,y1,z1]}
    }
    pub fn face_on_boundary(&self,axis:usize,positive:bool)->bool{
        let v=if positive{self.max[axis]}else{self.min[axis]};
        (v-if positive{0.5}else{-0.5}).abs()<0.0001
    }
    pub fn face_center(&self,axis:usize,positive:bool)->[f32;3]{
        let mut p=[
            (self.min[0]+self.max[0])*0.5,
            (self.min[1]+self.max[1])*0.5,
            (self.min[2]+self.max[2])*0.5,
        ];
        p[axis]=if positive{self.max[axis]}else{self.min[axis]};
        p
    }
    pub fn occluded_by(&self,other:&Self,axis:usize,positive:bool)->bool{
        let pos=if positive{self.max[axis]}else{self.min[axis]};
        let adjacent=if positive{other.min[axis]}else{other.max[axis]};
        if (pos-adjacent).abs()>0.0001{return false;}
        for d in 0..3{
            if d==axis{continue;}
            if other.min[d]>self.min[d]+0.0001||other.max[d]<self.max[d]-0.0001{
                return false;
            }
        }
        true
    }
}
/// Real element rotation (22.5 or 45 degrees) applied to each source-face
/// vertex rather than expanding the shape to a misleading bounding box.
#[derive(Clone,Debug)]
pub(crate) struct ElementRotation{
    pub axis:usize,
    pub origin:[f32;3], // normalized source voxel local coordinates
    pub angle_degrees:f32,
    pub rescale:bool,
}
#[derive(Clone,Debug)]
pub(crate) struct Part{
    pub bounds:Box3,
    pub rotation:Option<ElementRotation>,
    pub variant_x:i32,
    pub variant_y:i32,
}
impl Part{
    pub fn cube(bounds:Box3)->Self{
        Self{bounds,rotation:None,variant_x:0,variant_y:0}
    }
    fn rotate_local(v:[f32;3],axis:usize,angle:f32)->[f32;3]{
        let (s,c)=angle.to_radians().sin_cos();
        match axis{
            0=>[v[0],v[1]*c-v[2]*s,v[1]*s+v[2]*c],
            1=>[v[0]*c+v[2]*s,v[1],-v[0]*s+v[2]*c],
            _=>[v[0]*c-v[1]*s,v[0]*s+v[1]*c,v[2]],
        }
    }
    fn transform(&self,mut p:[f32;3],direction:bool)->[f32;3]{
        if let Some(rot)=&self.rotation{
            if !direction{
                for i in 0..3{p[i]-=rot.origin[i];}
            }
            if rot.rescale{
                // Minecraft element rotations rescale the two perpendicular
                // dimensions to retain their projected nominal extent.
                let factor=1.0/rot.angle_degrees.to_radians().cos().abs().max(0.001);
                for (i,v) in p.iter_mut().enumerate(){
                    if i!=rot.axis{*v*=factor;}
                }
            }
            p=Self::rotate_local(p,rot.axis,rot.angle_degrees);
            if !direction{
                for i in 0..3{p[i]+=rot.origin[i];}
            }
        }
        for _ in 0..self.variant_x.rem_euclid(360)/90{
            p=[p[0],-p[2],p[1]];
        }
        for _ in 0..self.variant_y.rem_euclid(360)/90{
            p=[-p[2],p[1],p[0]];
        }
        p
    }
    pub fn point(&self,local:[f32;3])->[f32;3]{self.transform(local,false)}
    pub fn face_vertices(&self,corners:[[f32;3];4])->[[f32;3];4]{
        corners.map(|p|{
            self.point([
                if p[0]>0.0{self.bounds.max[0]}else{self.bounds.min[0]},
                if p[1]>0.0{self.bounds.max[1]}else{self.bounds.min[1]},
                if p[2]>0.0{self.bounds.max[2]}else{self.bounds.min[2]},
            ])
        })
    }
    pub fn normal(&self,face_normal:(i32,i32,i32))->[f32;3]{
        self.transform([face_normal.0 as f32,face_normal.1 as f32,
            face_normal.2 as f32],true)
    }
    pub fn center(&self)->[f32;3]{
        self.point([
            (self.bounds.min[0]+self.bounds.max[0])*0.5,
            (self.bounds.min[1]+self.bounds.max[1])*0.5,
            (self.bounds.min[2]+self.bounds.max[2])*0.5,
        ])
    }
    pub fn boundary(&self,axis:usize,positive:bool)->bool{
        self.rotation.is_none()&&self.bounds.face_on_boundary(axis,positive)
    }
    pub fn occluded_by(&self,other:&Self,axis:usize,positive:bool)->bool{
        self.rotation.is_none()&&other.rotation.is_none()
            &&self.variant_x==other.variant_x&&self.variant_y==other.variant_y
            &&self.bounds.occluded_by(&other.bounds,axis,positive)
    }
}
fn property<'a>(state:&'a State,key:&str)->Option<&'a str>{
    state.properties.iter().find(|(k,_)|k==key).map(|(_,v)|v.as_str())
}
/// Approximate collision/display cuboids for recognized *minecraft:* states.
/// Never guesses geometry for foreign namespaces or numeric legacy IDs.
pub(crate) fn boxes(state:&State)->Vec<Box3>{
    let Some(name)=state.name.strip_prefix("minecraft:")else{return vec![Box3::FULL];};
    if name.ends_with("_slab"){
        return match property(state,"type"){
            Some("top")=>vec![Box3::from_six(-0.5,0.0,-0.5,0.5,0.5,0.5)],
            Some("bottom")=>vec![Box3::from_six(-0.5,-0.5,-0.5,0.5,0.0,0.5)],
            _=>vec![Box3::FULL]
        };
    }
    if name.ends_with("_stairs"){
        let top=property(state,"half")==Some("top");
        let (y0,y1,a,b)=if top{(0.0,0.5,-0.5,0.0)}else{(-0.5,0.0,0.0,0.5)};
        let mut elements=vec![Box3::from_six(-0.5,y0,-0.5,0.5,y1,0.5)];
        let second=match property(state,"facing").unwrap_or("north"){
            "north"=>Box3::from_six(-0.5,a,-0.5,0.5,b,0.0),
            "south"=>Box3::from_six(-0.5,a,0.0,0.5,b,0.5),
            "west"=>Box3::from_six(-0.5,a,-0.5,0.0,b,0.5),
            "east"=>Box3::from_six(0.0,a,-0.5,0.5,b,0.5),
            _=>return vec![Box3::FULL],
        };
        // Correct the top-half flight: the tread occupies the other half.
        if top{
            elements.push(Box3::from_six(second.min[0],-0.5,second.min[2],
                second.max[0],0.0,second.max[2]));
        }else{elements.push(second);}
        return elements;
    }
    if name=="snow"{
        let layers=property(state,"layers").and_then(|s|s.parse::<u8>().ok())
            .unwrap_or(1).clamp(1,8) as f32;
        return vec![Box3::from_six(-0.5,-0.5,-0.5,0.5,-0.5+layers/8.0,0.5)];
    }
    if name.ends_with("_carpet")||name=="moss_carpet"{
        return vec![Box3::from_six(-0.5,-0.5,-0.5,0.5,-0.4375,0.5)];
    }
    if matches!(name,"farmland"|"dirt_path"){
        return vec![Box3::from_six(-0.5,-0.5,-0.5,0.5,0.4375,0.5)];
    }
    if name.ends_with("_trapdoor"){
        let top=property(state,"half")==Some("top");
        let open=property(state,"open")==Some("true");
        if !open{
            return vec![Box3::from_six(-0.5,if top{0.3125}else{-0.5},-0.5,
                0.5,if top{0.5}else{-0.3125},0.5)];
        }
        return vec![match property(state,"facing").unwrap_or("north"){
            "north"=>Box3::from_six(-0.5,-0.5,-0.5,0.5,0.5,-0.3125),
            "south"=>Box3::from_six(-0.5,-0.5,0.3125,0.5,0.5,0.5),
            "west"=>Box3::from_six(-0.5,-0.5,-0.5,-0.3125,0.5,0.5),
            "east"=>Box3::from_six(0.3125,-0.5,-0.5,0.5,0.5,0.5),
            _=>Box3::FULL
        }];
    }
    if name.ends_with("_fence") && !name.ends_with("_fence_gate"){
        let mut out=vec![Box3::from_six(-0.125,-0.5,-0.125,0.125,0.5,0.125)];
        for (key,rail) in [
            ("north",Box3::from_six(-0.125,-0.375,-0.5,0.125,0.375,-0.125)),
            ("south",Box3::from_six(-0.125,-0.375,0.125,0.125,0.375,0.5)),
            ("west",Box3::from_six(-0.5,-0.375,-0.125,-0.125,0.375,0.125)),
            ("east",Box3::from_six(0.125,-0.375,-0.125,0.5,0.375,0.125))
        ]{
            if property(state,key)==Some("true"){out.push(rail);}
        }
        return out;
    }
    vec![Box3::FULL]
}
#[cfg(test)]
mod tests{
    use super::*;
    fn state(s:&str,p:&[(&str,&str)])->State{
        State{name:s.into(),properties:p.iter().map(|(a,b)|(a.to_string(),b.to_string())).collect()}
    }
    #[test]fn slab_snow_and_carpet_heights_are_signed_world_offsets(){
        let bottom=boxes(&state("minecraft:stone_slab",&[("type","bottom")]))[0];
        let top=boxes(&state("minecraft:stone_slab",&[("type","top")]))[0];
        assert_eq!((bottom.max[1],top.min[1]),(0.0,0.0));
        assert_eq!(boxes(&state("minecraft:snow",&[("layers","3")]))[0].max[1],-0.125);
        assert_eq!(boxes(&state("minecraft:white_carpet",&[]))[0].max[1],-0.4375);
    }
    #[test]fn stair_shapes_and_face_culling_require_full_cover(){
        let stairs=boxes(&state("minecraft:oak_stairs",&[("half","bottom"),("facing","north")]));
        assert_eq!(stairs.len(),2);
        assert_eq!(stairs[0].max[1],0.0);
        assert_eq!(stairs[1].min[1],0.0);
        assert!(!stairs[0].face_on_boundary(1,true));
        assert!(stairs[1].face_on_boundary(1,true));
        assert!(!stairs[0].occluded_by(&stairs[1],1,true));
    }
    #[test]fn zero_thickness_model_planes_are_supported_only_as_explicit_surfaces(){
        let plane=Box3::from_model_units_plane([0.8,0.0,8.0],[15.2,16.0,8.0])
            .unwrap();
        assert_eq!(plane.min[2],0.0);
        assert_eq!(plane.max[2],0.0);
        assert!(Box3::from_model_units([0.8,0.0,8.0],[15.2,16.0,8.0]).is_none());
        assert!(Box3::from_model_units_plane([8.0,8.0,0.0],[8.0,8.0,16.0]).is_none());
        assert!(Box3::from_model_units_plane([0.0,0.0,0.0],[16.0,16.0,16.0]).is_none());
    }
    #[test]fn model_unit_boxes_check_signed_scale_and_invalid_dimensions(){
        let a=Box3::from_model_units([2.0,0.0,4.0],[14.0,8.0,12.0]).unwrap();
        assert_eq!(a.min,[-0.375,-0.5,-0.25]);
        assert_eq!(a.max,[0.375,0.0,0.25]);
        assert!(Box3::from_model_units([0.0,0.0,0.0],[0.0,16.0,16.0]).is_none());
        assert!(Box3::from_model_units([0.0,0.0,0.0],[17.0,16.0,16.0]).is_none());
    }
    #[test]fn java_y_and_x_quarter_turns_preserve_exact_box_dimensions(){
        let box_=Box3::from_model_units([0.0,0.0,0.0],[4.0,12.0,16.0]).unwrap();
        let y=box_.rotate(0,90).unwrap();
        assert_eq!(y.min,[-0.5,-0.5,-0.5]);
        assert_eq!(y.max,[0.5,0.25,-0.25]);
        assert_eq!(box_.rotate(0,360).unwrap(),box_);
        assert_eq!(box_.rotate(360,0).unwrap(),box_);
        assert!(box_.rotate(45,0).is_none());
    }
    #[test]fn angled_model_face_is_quad_not_axis_aligned_aabb(){
        let part=Part{
            bounds:Box3::from_model_units([6.0,0.0,6.0],[10.0,16.0,10.0]).unwrap(),
            rotation:Some(ElementRotation{axis:1,origin:[0.0,0.0,0.0],
                angle_degrees:45.0,rescale:false}),
            variant_x:0,variant_y:0,
        };
        let p=part.point([0.25,0.0,0.0]);
        assert!((p[0]-0.1767767).abs()<0.0001);
        assert!((p[2]+0.1767767).abs()<0.0001);
        let vertices=part.face_vertices([[0.5,0.5,-0.5],
            [0.5,0.5,0.5],[-0.5,0.5,0.5],[-0.5,0.5,-0.5]]);
        assert!(vertices.iter().any(|p|p[0].abs()>0.125));
        assert!(!part.boundary(0,true));
    }
    #[test]fn no_modded_block_model_is_invented(){
        assert_eq!(boxes(&state("other:custom_stairs",&[])),vec![Box3::FULL]);
        assert_eq!(boxes(&state("numeric:108",&[])),vec![Box3::FULL]);
        assert_eq!(boxes(&state("minecraft:stone_slab",&[("type","double")])),vec![Box3::FULL]);
        assert_eq!(boxes(&state("minecraft:spruce_trapdoor",&[("open","true"),("facing","east")]))[0].min[0],0.3125);
    }
}