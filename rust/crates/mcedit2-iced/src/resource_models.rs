//! Bounded lookup of resource-pack blockstate variants and model texture maps.
//! A small conservative subset of Java JSON model inheritance is supported;
//! unsupported multipart, rotations, random weighted variants and custom
//! geometry use the ordinary named-texture fallback rather than guessing.
use std::collections::{BTreeMap,BTreeSet};
use std::hash::{Hash,Hasher};
use std::sync::OnceLock;
use mcedit2_world::sections::State;
use serde_json::Value;
use crate::isometric::Face;
use crate::models::{Box3,Part,ElementRotation};

#[derive(Debug,Clone,PartialEq)]
pub(crate) struct FaceSpec {
    pub texture:String,
    /// Minecraft model-element UV rectangle, in 0..16 texture units.
    pub uv:[f32;4],
    pub rotation:u16,
    /// Source face vertex i samples base texture corner uvlock[i].
    /// Handles quarter-turns and mirrors without corrupting geometry.
    pub uvlock:[u8;4],
}
#[derive(Clone,Debug,Default)]
pub(crate) struct ModelBank {
    files:BTreeMap<String,Value>,
}
fn asset_id(path:&str)->Option<String>{
    let p:Vec<&str>=path.split('/').collect();
    if p.len()<4||p[0]!="assets"{return None;}
    let ns=p[1];
    if ns.is_empty()||ns.len()>80||!ns.bytes().all(|b|
        b.is_ascii_lowercase()||b.is_ascii_digit()||b==b'_'||b==b'-'){return None;}
    let (kind,start)=if p.get(2)==Some(&"blockstates"){("state",3)}
        else if p.get(2)==Some(&"models")&&p.get(3)==Some(&"block"){("model",4)}
        else{return None;};
    let rel=p[start..].join("/");
    let stem=rel.strip_suffix(".json")?;
    if stem.is_empty()||stem.len()>160||!stem.bytes().all(|b|
        b.is_ascii_lowercase()||b.is_ascii_digit()||b==b'_'||b==b'-'||b==b'/'){return None;}
    Some(format!("{kind}:{ns}:{stem}"))
}
fn normalize_model(value:&str,default_ns:&str)->Option<String>{
    let (ns,path)=value.split_once(':').unwrap_or((default_ns,value));
    let path=path.strip_prefix("block/").unwrap_or(path);
    if ns.is_empty()||path.is_empty()||path.contains("..")||path.starts_with('/'){return None;}
    Some(format!("model:{ns}:{path}"))
}
/// Vanilla's common block parent templates are not bundled in most
/// resource packs. Resolve only known minecraft: parent IDs to trusted,
/// bounded model elements, preserving all child texture aliases. Never
/// synthesize geometry for an absent third-party namespace/parent.
fn vanilla_parent_elements(id:&str)->Option<&'static Vec<Value>>{
    static TEMPLATES:OnceLock<BTreeMap<&'static str,Vec<Value>>>=OnceLock::new();
    let templates=TEMPLATES.get_or_init(||{
        const DIRS:[&str;6]=["down","up","north","south","west","east"];
        let mut out=BTreeMap::new();
        for (name,aliases) in [
            ("cube_all",["all","all","all","all","all","all"]),
            ("cube",["down","up","north","south","west","east"]),
            ("cube_column",["end","end","side","side","side","side"]),
            ("cube_column_horizontal",["side","side","end","end","side","side"]),
            ("cube_bottom_top",["bottom","top","side","side","side","side"]),
            ("orientable",["bottom","top","front","side","side","side"]),
            ("orientable_with_bottom",["bottom","top","front","side","side","side"]),
        ]{
            let mut faces=serde_json::Map::new();
            for (dir,alias) in DIRS.into_iter().zip(aliases){
                faces.insert(dir.to_string(),serde_json::json!({
                    "texture":format!("#{alias}"),"cullface":dir
                }));
            }
            out.insert(name,vec![serde_json::json!({
                "from":[0,0,0],"to":[16,16,16],"faces":faces
            })]);
        }
        // Crossed plants are two physically flat, double-sided planes.
        // Unlike a fallback cube they admit transparent holes and accurate
        // silhouette/picking under a JSON model with no copied parent.
        let cross=||[45,-45].into_iter().map(|angle|{
            serde_json::json!({
                "from":[0,0,8],"to":[16,16,8],
                "rotation":{"origin":[8,8,8],"axis":"y","angle":angle},
                "shade":false,
                "faces":{"north":{"texture":"#cross"},
                         "south":{"texture":"#cross"}}
            })
        }).collect::<Vec<_>>();
        out.insert("cross",cross());
        out.insert("tinted_cross",cross());
        out
    });
    let id=id.strip_prefix("model:minecraft:")?;
    templates.get(id)
}
fn normalize_texture(value:&str,default_ns:&str)->Option<String>{
    let (ns,path)=value.split_once(':').unwrap_or((default_ns,value));
    let path=path.strip_prefix("block/").unwrap_or(path);
    if ns.is_empty()||path.is_empty()||path.contains("..")||path.starts_with('/'){return None;}
    Some(format!("{ns}:{path}"))
}
fn variant_matches(name:&str,state:&State)->bool{
    name.split(',').all(|entry|{
        if entry.is_empty(){return true;}
        let Some((key,value))=entry.split_once('=')else{return false;};
        state.properties.iter().any(|(k,v)|k==key&&v==value)
    })
}
fn best_variant<'a>(obj:&'a serde_json::Map<String,Value>,state:&State)->Option<&'a Value>{
    obj.iter().filter(|(k,_)|variant_matches(k,state))
        .max_by_key(|(k,_)|k.split(',').filter(|s|!s.is_empty()).count())
        .map(|(_,v)|v)
}
fn variant_angles(value:&Value)->Option<(i32,i32)>{
    if value.is_array()||value.get("uvlock").is_some_and(|v|v.as_bool().is_none()){return None;}
    let angle=|key:&str|->Option<i32>{
        let value=value.get(key);
        let n=if let Some(v)=value{v.as_i64()?}else{0};
        if n.abs()>360||n%90!=0{return None;}
        i32::try_from(n).ok()
    };
    Some((angle("x")?,angle("y")?))
}
#[derive(Debug,Clone,Hash)]
struct ChosenModel{key:String,x:i32,y:i32,uvlock:bool}
fn deterministic_model_seed(state:&State,pos:(i32,i32,i32),salt:u64)->u64{
    let mut hasher=std::collections::hash_map::DefaultHasher::new();
    state.name.hash(&mut hasher);
    state.properties.hash(&mut hasher);
    pos.hash(&mut hasher);
    salt.hash(&mut hasher);
    hasher.finish()
}
/// Choose a weighted Java model entry without a global RNG: reloading a
/// chunk/camera yields the same source XYZ appearance and matched UVs.
fn choose_weighted<'a>(value:&'a Value,seed:u64)->Option<&'a Value>{
    let Some(array)=value.as_array()else{return Some(value);};
    if array.is_empty()||array.len()>64{return None;}
    let mut weights=Vec::with_capacity(array.len());
    let mut total=0u64;
    for item in array{
        let weight=item.get("weight").map(Value::as_u64).unwrap_or(Some(1))?;
        if weight==0||weight>10_000{return None;}
        total=total.checked_add(weight)?;
        if total>640_000{return None;}
        weights.push(weight);
    }
    let mut target=seed%total;
    for (item,weight) in array.iter().zip(weights){
        if target<weight{return Some(item);}
        target-=weight;
    }
    None
}
fn uvlock_permutation(local:Face,x:i32,y:i32)->Option<[u8;4]>{
    const FACES:[Face;6]=[Face::Top,Face::Bottom,
        Face::PosX,Face::NegX,Face::PosZ,Face::NegZ];
    let part=Part{bounds:Box3::FULL,rotation:None,variant_x:x,variant_y:y};
    let normal=part.normal(local.normal());
    let target=FACES.iter().copied().find(|face|{
        let n=face.normal();
        let expected=[n.0 as f32,n.1 as f32,n.2 as f32];
        normal.iter().zip(expected).all(|(a,b)|(*a-b).abs()<1e-5)
    })?;
    let rotated=part.face_vertices(local.corners());
    let canonical=target.corners();
    let mut permutation=[0u8;4];
    for (i,point) in rotated.iter().enumerate(){
        let index=canonical.iter().position(|target|point.iter().zip(target)
            .all(|(a,b)|(*a-b).abs()<1e-5))?;
        permutation[i]=index as u8;
    }
    let mut sorted=permutation;
    sorted.sort_unstable();
    (sorted==[0,1,2,3]).then_some(permutation)
}

/// Minecraft multipart: property AND, pipe-separated alternatives,
/// nested OR/AND, and omitted when (always). Reject unknown syntax.
fn condition_matches(condition:&Value,state:&State,depth:usize)->Option<bool>{
    if depth>8{return None;}
    let terms=condition.as_object()?;
    if terms.len()>64{return None;}
    let mut all=true;
    for (key,value) in terms{
        let matches=match key.as_str(){
            "OR"|"AND"=>{
                let expressions=value.as_array()?;
                if expressions.is_empty()||expressions.len()>64{return None;}
                let mut any=false;let mut every=true;
                for expr in expressions{
                    let yes=condition_matches(expr,state,depth+1)?;
                    any|=yes; every&=yes;
                }
                if key=="OR"{any}else{every}
            }
            _=>{
                if key.is_empty()||key.len()>80||
                    !key.bytes().all(|c|c.is_ascii_alphanumeric()||c==b'_')
                {return None;}
                let allowed=value.as_str()?;
                let actual=state.properties.iter().find(|(k,_)|k==key)
                    .map(|(_,v)|v.as_str());
                let mut any=false;
                for alt in allowed.split('|'){
                    if alt.is_empty(){return None;}
                    any|=actual==Some(alt);
                }
                any
            }
        };
        all &= matches;
    }
    Some(all)
}
fn read_rotation_origin(doc:&Value)->Option<[f32;3]>{
    let v=doc.get("origin")?.as_array()?;
    if v.len()!=3{return None;}
    let mut o=[0.0;3];
    for (i,x) in v.iter().enumerate(){
        let value=x.as_f64()? as f32;
        if !value.is_finite()||!(-16.0..=32.0).contains(&value){
            return None;
        }
        o[i]=value/16.0-0.5;
    }
    Some(o)
}
/// Implicit Java model element UVs depend on which face of a partial
/// cuboid is rendered. A full [0,0,16,16] texture stretches partial models.
fn implicit_face_uv(element:&Value,face:Face)->Option<[f32;4]>{
    let parse=|name:&str|->Option<[f32;3]>{
        let coords=element.get(name)?.as_array()?;
        if coords.len()!=3{return None;}
        let mut out=[0f32;3];
        for (i,v) in coords.iter().enumerate(){
            out[i]=v.as_f64()? as f32;
            if !out[i].is_finite()||!(0.0..=16.0).contains(&out[i]){
                return None;
            }
        }
        Some(out)
    };
    let [x0,y0,z0]=parse("from")?;
    let [x1,y1,z1]=parse("to")?;
    if x0>x1||y0>y1||z0>z1{return None;}
    let flat=[x0==x1,y0==y1,z0==z1];
    if flat.iter().filter(|&&v|v).count()>1{return None;}
    if flat[0]&&!matches!(face,Face::NegX|Face::PosX)
        ||flat[1]&&!matches!(face,Face::Bottom|Face::Top)
        ||flat[2]&&!matches!(face,Face::NegZ|Face::PosZ){
        return None;
    }
    Some(match face{
        Face::Bottom=>[x0,16.0-z1,x1,16.0-z0],
        Face::Top=>[x0,z0,x1,z1],
        Face::NegZ=>[16.0-x1,16.0-y1,16.0-x0,16.0-y0],
        Face::PosZ=>[x0,16.0-y1,x1,16.0-y0],
        Face::NegX=>[z0,16.0-y1,z1,16.0-y0],
        Face::PosX=>[16.0-z1,16.0-y1,16.0-z0,16.0-y0],
    })
}
fn variant_model(value:&Value)->Option<&str>{
    // Deterministic first entry for weighted variants; no RNG or invented geometry.
    let value=if let Some(list)=value.as_array(){list.first()?}else{value};
    value.get("model")?.as_str()
}
fn face_keys(face:Face,parent:&str)->&'static [&'static str]{
    const TOP:&[&str]=&["up","top","end","all"];
    const BOTTOM:&[&str]=&["down","bottom","end","all"];
    const NORTH:&[&str]=&["north","side","all"];
    const SOUTH:&[&str]=&["south","side","all"];
    const EAST:&[&str]=&["east","side","all"];
    const WEST:&[&str]=&["west","side","all"];
    // "parent" retained for future precise parent geometry expansion.
    let _=parent;
    match face{
        Face::Top=>TOP,Face::Bottom=>BOTTOM,Face::PosX=>EAST,
        Face::NegX=>WEST,Face::PosZ=>SOUTH,Face::NegZ=>NORTH
    }
}
impl ModelBank {
    fn selections(&self,state:&State)->Option<Vec<ChosenModel>>{
        self.selections_at(state,(0,0,0))
    }
    fn selections_at(&self,state:&State,pos:(i32,i32,i32))
        ->Option<Vec<ChosenModel>>{
        let (ns,name)=state.name.split_once(':')?;
        let doc=self.files.get(&format!("state:{ns}:{name}"))?;
        if let Some(parts)=doc.get("multipart"){
            let parts=parts.as_array()?;
            if parts.is_empty()||parts.len()>64{return None;}
            let mut chosen=Vec::new();
            for (index,part) in parts.iter().enumerate(){
                let active=part.get("when").map(|v|condition_matches(v,state,0))
                    .unwrap_or(Some(true))?;
                if !active{continue;}
                let apply=choose_weighted(part.get("apply")?,
                    deterministic_model_seed(state,pos,index as u64+1))?;
                let (x,y)=variant_angles(apply)?;
                chosen.push(ChosenModel{
                    key:normalize_model(apply.get("model")?.as_str()?,ns)?,x,y,
                    uvlock:apply.get("uvlock").and_then(Value::as_bool).unwrap_or(false)
                });
            }
            if chosen.is_empty(){return None;}
            return Some(chosen);
        }
        let value=choose_weighted(
            best_variant(doc.get("variants")?.as_object()?,state)?,
            deterministic_model_seed(state,pos,0))?;
        let (x,y)=variant_angles(value)?;
        Some(vec![ChosenModel{
            key:normalize_model(value.get("model")?.as_str()?,ns)?,x,y,
            uvlock:value.get("uvlock").and_then(Value::as_bool).unwrap_or(false)
        }])
    }
    fn chosen_element(&self,state:&State,index:usize)->Option<(ChosenModel,&Value)>{
        self.chosen_element_at(state,index,(0,0,0))
    }
    fn chosen_element_at(&self,state:&State,index:usize,pos:(i32,i32,i32))
        ->Option<(ChosenModel,&Value)>{
        if index>=128{return None;}
        let mut remaining=index;
        for model in self.selections_at(state,pos)?{
            let elements=self.model_elements(&model.key)?;
            if remaining<elements.len(){
                return Some((model,elements.get(remaining)?));
            }
            remaining=remaining.checked_sub(elements.len())?;
        }
        None
    }
    /// Signed block position changes weighted variants; the cache key
    /// hashes only the chosen model set, not each individual coordinate.
    pub fn weighted_selection_key(&self,state:&State,pos:(i32,i32,i32))->u64{
        let Some(chosen)=self.selections_at(state,pos)else{return 0;};
        let mut hash=std::collections::hash_map::DefaultHasher::new();
        chosen.hash(&mut hash);
        hash.finish()
    }
    /// Elements may deliberately disable directional shading, notably the
    /// zero-thickness crossed planes used by flowers and saplings.
    pub fn element_unshaded(&self,state:&State,index:usize)->Option<bool>{
        self.element_unshaded_at(state,index,(0,0,0))
    }
    pub fn element_unshaded_at(&self,state:&State,index:usize,pos:(i32,i32,i32))
        ->Option<bool>{
        let (_,element)=self.chosen_element_at(state,index,pos)?;
        let shade=element.get("shade").map(Value::as_bool).unwrap_or(Some(true))?;
        Some(!shade)
    }
    /// Some(false) means an explicit Java model face is absent and must NOT
    /// be rendered as a synthetic palette-textured side. None means that
    /// the model/element is not resolved and the fallback remains active.
    pub fn face_defined(&self,state:&State,face:Face,index:usize)->Option<bool>{
        self.face_defined_at(state,face,index,(0,0,0))
    }
    pub fn face_defined_at(&self,state:&State,face:Face,index:usize,
        pos:(i32,i32,i32))->Option<bool>{
        let (_model,element)=self.chosen_element_at(state,index,pos)?;
        let faces=element.get("faces")?.as_object()?;
        let name=match face{
            Face::Top=>"up",Face::Bottom=>"down",
            Face::PosX=>"east",Face::NegX=>"west",
            Face::PosZ=>"south",Face::NegZ=>"north"
        };
        Some(faces.contains_key(name))
    }
    /// Return the rotated, *world-facing* vanilla cullface declaration.
    /// Some(None) is an explicitly rendered face without cullface: it must
    /// not be removed just because an adjacent block fills the cell.
    /// None means no reliable JSON face metadata was found.
    pub fn face_cull_direction(&self,state:&State,face:Face,index:usize)
        ->Option<Option<Face>>{
        self.face_cull_direction_at(state,face,index,(0,0,0))
    }
    pub fn face_cull_direction_at(&self,state:&State,face:Face,index:usize,
        pos:(i32,i32,i32))->Option<Option<Face>>{
        let (chosen,element)=self.chosen_element_at(state,index,pos)?;
        let name=match face{
            Face::Top=>"up",Face::Bottom=>"down",
            Face::PosX=>"east",Face::NegX=>"west",
            Face::PosZ=>"south",Face::NegZ=>"north"
        };
        let face_doc=element.get("faces")?.get(name)?;
        let Some(raw)=face_doc.get("cullface") else{return Some(None)};
        let local=match raw.as_str()?{
            "up"=>Face::Top,"down"=>Face::Bottom,
            "east"=>Face::PosX,"west"=>Face::NegX,
            "south"=>Face::PosZ,"north"=>Face::NegZ,
            _=>return None,
        };
        let mut part=Part::cube(Box3::FULL);
        part.variant_x=chosen.x;
        part.variant_y=chosen.y;
        let n=part.normal(local.normal());
        let rotated=match n{
            [1.0,0.0,0.0]=>Face::PosX,
            [-1.0,0.0,0.0]=>Face::NegX,
            [0.0,1.0,0.0]=>Face::Top,
            [0.0,-1.0,0.0]=>Face::Bottom,
            [0.0,0.0,1.0]=>Face::PosZ,
            [0.0,0.0,-1.0]=>Face::NegZ,
            _=>return None,
        };
        Some(Some(rotated))
    }
    fn model_elements(&self,key:&str)->Option<&Vec<Value>>{
        let mut current=Some(key.to_string());
        let mut seen=BTreeSet::new();
        while let Some(id)=current{
            if seen.len()>=16||!seen.insert(id.clone()){return None;}
            let Some(model)=self.files.get(&id)else{
                return vanilla_parent_elements(&id);
            };
            if let Some(elements)=model.get("elements"){
                return elements.as_array();
            }
            let source_ns=id.strip_prefix("model:")?.split_once(':')?.0;
            current=model.get("parent").and_then(Value::as_str)
                .and_then(|v|normalize_model(v,source_ns));
        }
        None
    }
    pub fn insert(&mut self,path:&str,bytes:&[u8])->Result<bool,String>{
        let Some(key)=asset_id(path)else{return Ok(false);};
        if bytes.len()>256*1024{return Err("Model JSON larger than 256KiB".into());}
        if self.files.len()>=4096{return Err("Model JSON entry limit reached".into());}
        let json:Value=serde_json::from_slice(bytes).map_err(|e|e.to_string())?;
        if !json.is_object(){return Err("Model JSON root must be an object".into());}
        self.files.insert(key,json);Ok(true)
    }
    pub fn count(&self)->usize{self.files.len()}
    /// Flatten all matched variant/multipart model elements while retaining
    /// original voxel identity for picking.
    /// Native visual parts support vanilla 22.5/45 degree element axes
    /// without collapsing the source geometry into a larger AABB.
    pub fn parts(&self,state:&State)->Option<Vec<Part>>{
        self.parts_at(state,(0,0,0))
    }
    pub fn parts_at(&self,state:&State,pos:(i32,i32,i32))->Option<Vec<Part>>{
        let chosen=self.selections_at(state,pos)?;
        let mut out=Vec::new();
        for model in chosen{
            let elements=self.model_elements(&model.key)?;
            if elements.is_empty()||elements.len()>64
                ||out.len()+elements.len()>128{return None;}
            for entry in elements{
                let read=|name:&str|->Option<[f32;3]>{
                    let a=entry.get(name)?.as_array()?;
                    if a.len()!=3{return None;}
                    Some([a[0].as_f64()? as f32,
                        a[1].as_f64()? as f32,a[2].as_f64()? as f32])
                };
                let from=read("from")?;
                let to=read("to")?;
                let bounds=Box3::from_model_units(from,to).or_else(||{
                    let flat=Box3::from_model_units_plane(from,to)?;
                    let axis=(0..3).find(|&i|from[i]==to[i])?;
                    let sides=match axis{
                        0=>["east","west"],
                        1=>["up","down"],
                        _=>["north","south"],
                    };
                    let faces=entry.get("faces")?.as_object()?;
                    // The zero-thickness plane is renderable only when the
                    // JSON explicitly declares a face normal to its axis.
                    sides.iter().any(|name|faces.contains_key(*name)).then_some(flat)
                })?;
                let rotation=if let Some(doc)=entry.get("rotation"){
                    let axis=match doc.get("axis")?.as_str()?{
                        "x"=>0,"y"=>1,"z"=>2,_=>return None,
                    };
                    let angle=doc.get("angle")?.as_f64()? as f32;
                    if ![-45.0,-22.5,0.0,22.5,45.0].contains(&angle){
                        return None;
                    }
                    let o=read_rotation_origin(doc)?;
                    Some(ElementRotation{
                        axis,origin:o,angle_degrees:angle,
                        rescale:doc.get("rescale").and_then(Value::as_bool)
                            .unwrap_or(false),
                    })
                }else{None};
                out.push(Part{bounds,rotation,variant_x:model.x,
                    variant_y:model.y});
            }
        }
        if out.is_empty(){None}else{Some(out)}
    }
    pub fn elements(&self,state:&State)->Option<Vec<Box3>>{
        let chosen=self.selections(state)?;
        let mut out=Vec::new();
        for model in chosen{
            let elements=self.model_elements(&model.key)?;
            if elements.is_empty()||elements.len()>64
                ||out.len()+elements.len()>128{return None;}
            for entry in elements{
                if entry.get("rotation").is_some(){return None;}
                let read=|name:&str|->Option<[f32;3]>{
                    let a=entry.get(name)?.as_array()?;
                    if a.len()!=3{return None;}
                    Some([a[0].as_f64()? as f32,
                        a[1].as_f64()? as f32,a[2].as_f64()? as f32])
                };
                out.push(Box3::from_model_units(read("from")?,read("to")?)?
                    .rotate(model.x,model.y)?);
            }
        }
        if out.is_empty(){None}else{Some(out)}
    }
    /// A specific Java model element face's texture and declared UV.
    /// This is more precise than the older block-global texture map.
    pub fn element_face(&self,state:&State,face:Face,index:usize)->Option<FaceSpec>{
        self.element_face_at(state,face,index,(0,0,0))
    }
    pub fn element_face_at(&self,state:&State,face:Face,index:usize,
        pos:(i32,i32,i32))->Option<FaceSpec>{
        let (model,element)=self.chosen_element_at(state,index,pos)?;
        // A RenderFace stores the *local model face*; its vertices and
        // normals were rotated later by Part::point(). Inverse rotation
        // here would apply the same transform twice.
        let model_face=face;
        let uvlock=if model.uvlock{
            uvlock_permutation(face,model.x,model.y)?
        }else{[0,1,2,3]};
        let model_key=model.key;
        let mut current=Some(model_key.clone());
        let mut visited=BTreeSet::new();
        let mut textures:Vec<&serde_json::Map<String,Value>>=Vec::new();
        let mut model_elements=None;
        while let Some(key)=current{
            if visited.len()>=16||!visited.insert(key.clone()){return None;}
            let Some(doc)=self.files.get(&key)else{break};
            if let Some(map)=doc.get("textures").and_then(Value::as_object){
                textures.push(map);
            }
            if model_elements.is_none(){
                model_elements=doc.get("elements").and_then(Value::as_array);
            }
            let source_ns=key.strip_prefix("model:")?.split_once(':')?.0;
            current=doc.get("parent").and_then(Value::as_str)
                .and_then(|s|normalize_model(s,source_ns));
        }
        let _=model_elements; // validated inheritance, chosen element is exact
        let name=match model_face{
            Face::Top=>"up",Face::Bottom=>"down",Face::PosX=>"east",
            Face::NegX=>"west",Face::PosZ=>"south",Face::NegZ=>"north"
        };
        let face_doc=element.get("faces")?.get(name)?;
        let mut raw=face_doc.get("texture")?.as_str()?.to_string();
        let mut visited_aliases=BTreeSet::new();
        while let Some(alias)=raw.strip_prefix('#'){
            if visited_aliases.len()>=16||!visited_aliases.insert(alias.to_string()){
                return None;
            }
            raw=textures.iter().find_map(|m|m.get(alias).and_then(Value::as_str))?.into();
        }
        let default_ns=model_key.strip_prefix("model:")?.split_once(':')?.0;
        let texture=normalize_texture(&raw,default_ns)?;
        let rotation=match face_doc.get("rotation"){
            None=>0,
            Some(v)=>u16::try_from(v.as_u64()?).ok()?,
        };
        if rotation%90!=0||rotation>=360{return None;}
        let uv=if let Some(v)=face_doc.get("uv"){
            let a=v.as_array()?;
            if a.len()!=4{return None;}
            let mut out=[0f32;4];
            for (i,item) in a.iter().enumerate(){
                let value=item.as_f64()? as f32;
                if !value.is_finite()||!(0.0..=16.0).contains(&value){return None;}
                out[i]=value;
            }
            out
        }else{implicit_face_uv(element,model_face)?};
        Some(FaceSpec{texture,uv,rotation,uvlock})
    }
    pub fn texture(&self,state:&State,face:Face)->Option<String>{
        self.texture_at(state,face,(0,0,0))
    }
    pub fn texture_at(&self,state:&State,face:Face,pos:(i32,i32,i32))
        ->Option<String>{
        let chosen=self.selections_at(state,pos)?.into_iter().next()?;
        let lookup_face=face.undo_variant_rotation(chosen.x,chosen.y);
        let model_key=chosen.key;
        let fallback_ns=model_key.strip_prefix("model:")?.split_once(':')?.0.to_string();
        let mut maps=Vec::new();
        let mut visited=BTreeSet::new();
        let mut current=Some(model_key.clone());
        while let Some(key)=current{
            if visited.len()>=16||!visited.insert(key.clone()){return None;}
            let Some(model)=self.files.get(&key)else{break};
            maps.push(model.get("textures").and_then(Value::as_object));
            let source_ns=key.strip_prefix("model:")?.split_once(':')?.0;
            current=model.get("parent").and_then(Value::as_str)
                .and_then(|v|normalize_model(v,source_ns));
        }
        // Newer models override inherited parent texture keys.
        let get=|key:&str|->Option<String>{
            for map in &maps{
                if let Some(value)=map.and_then(|m|m.get(key)).and_then(Value::as_str){
                    return Some(value.to_string());
                }
            }
            None
        };
        for key in face_keys(lookup_face,&model_key){
            let Some(mut texture)=get(key)else{continue;};
            let mut seen=BTreeSet::new();
            for _ in 0..16{
                if !texture.starts_with('#'){break;}
                if !seen.insert(texture.clone()){return None;}
                texture=get(texture.trim_start_matches('#'))?;
            }
            if texture.starts_with('#'){return None;}
            if let Some(id)=normalize_texture(&texture,&fallback_ns){return Some(id);}
        }
        None
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    fn s(name:&str,props:&[(&str,&str)])->State{
        State{name:name.into(),properties:props.iter().map(|(a,b)|(a.to_string(),b.to_string())).collect()}
    }
    #[test]fn weighted_variants_are_stable_per_source_xyz_and_match_texture_geometry(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/ore.json",
            br#"{"variants":{"":[
               {"model":"test:block/short","weight":1},
               {"model":"test:block/tall","weight":3}]}}"#).unwrap();
        bank.insert("assets/test/models/block/short.json",
            br#"{"elements":[{"from":[0,0,0],"to":[16,8,16],
                 "faces":{"up":{"texture":"test:block/short","uv":[0,0,16,16]}}}]}"#).unwrap();
        bank.insert("assets/test/models/block/tall.json",
            br#"{"elements":[{"from":[0,0,0],"to":[16,16,16],
                 "faces":{"up":{"texture":"test:block/tall","uv":[0,0,16,16]}}}]}"#).unwrap();
        let state=s("test:ore",&[]);
        let mut short=0;
        let mut tall=0;
        for x in -64..64{
            let pos=(x,64,-14);
            let a=bank.parts_at(&state,pos).unwrap();
            let b=bank.parts_at(&state,pos).unwrap();
            assert_eq!(a.iter().map(|p|p.bounds).collect::<Vec<_>>(),
                b.iter().map(|p|p.bounds).collect::<Vec<_>>());
            let texture=bank.element_face_at(&state,Face::Top,0,pos).unwrap().texture;
            let key=bank.weighted_selection_key(&state,pos);
            assert_eq!(key,bank.weighted_selection_key(&state,pos));
            if a[0].bounds.max[1]<0.5{
                short+=1;assert_eq!(texture,"test:short");
            }else{
                tall+=1;assert_eq!(texture,"test:tall");
            }
        }
        assert!(short>0&&tall>short);
    }
    #[test]fn multipart_weighted_apply_uses_independent_stable_choices(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/garden.json",
            br#"{"multipart":[
                {"apply":[{"model":"test:block/short","weight":1},
                          {"model":"test:block/tall","weight":1}]},
                {"when":{"side":"north"},
                 "apply":[{"model":"test:block/tall","weight":1},
                          {"model":"test:block/short","weight":1}]}]}"#).unwrap();
        bank.insert("assets/test/models/block/short.json",
            br#"{"elements":[{"from":[0,0,0],"to":[16,8,16],
                 "faces":{"up":{"texture":"test:block/short"}}}]}"#).unwrap();
        bank.insert("assets/test/models/block/tall.json",
            br#"{"elements":[{"from":[0,0,0],"to":[16,16,16],
                 "faces":{"up":{"texture":"test:block/tall"}}}]}"#).unwrap();
        let state=s("test:garden",&[("side","north")]);
        let source=(8,70,-3);
        let chosen=bank.selections_at(&state,source).unwrap();
        assert_eq!(chosen.len(),2);
        assert_eq!(chosen.iter().map(|m|m.key.clone()).collect::<Vec<_>>(),
            bank.selections_at(&state,source).unwrap()
                .iter().map(|m|m.key.clone()).collect::<Vec<_>>());
        assert_eq!(bank.parts_at(&state,source).unwrap().len(),2);
        assert!(choose_weighted(&serde_json::json!([
            {"model":"x","weight":0}
        ]),0).is_none());
    }
    #[test]fn state_variants_inherit_named_top_side_and_preserve_namespace(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/widget.json",
            br#"{"variants":{"facing=north":{"model":"test:block/widget_n"},"":{"model":"test:block/widget"}}}"#).unwrap();
        bank.insert("assets/test/models/block/widget_n.json",
            br##"{"parent":"test:block/cube_column","textures":{"end":"test:block/widget_end","side":"#body","body":"test:block/widget_wall"}}"##).unwrap();
        let state=s("test:widget",&[("facing","north")]);
        assert_eq!(bank.texture(&state,Face::Top),Some("test:widget_end".into()));
        assert_eq!(bank.texture(&state,Face::PosZ),Some("test:widget_wall".into()));
        assert!(bank.texture(&s("other:widget",&[]),Face::Top).is_none());
    }
    #[test]fn cyclic_texture_aliases_and_untrusted_paths_fail_closed(){
        let mut bank=ModelBank::default();
        bank.insert("assets/minecraft/blockstates/loop.json",
            br#"{"variants":{"":{"model":"minecraft:block/loop"}}}"#).unwrap();
        bank.insert("assets/minecraft/models/block/loop.json",
            br##"{"textures":{"all":"#one","one":"#all"}}"##).unwrap();
        assert_eq!(bank.texture(&s("minecraft:loop",&[]),Face::Top),None);
        assert!(!bank.insert("assets/minecraft/models/item/bad.json",b"{}").unwrap());
        assert!(!bank.insert("assets/../models/block/thing.json",b"{}").unwrap());
    }
    #[test]fn vanilla_model_elements_produce_exact_unrotated_cuboids(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/window.json",
            br#"{"variants":{"":{"model":"test:block/window"}}}"#).unwrap();
        bank.insert("assets/test/models/block/window.json",
            br#"{"elements":[{"from":[3,1,4],"to":[13,15,12]}]}"#).unwrap();
        let elements=bank.elements(&s("test:window",&[])).unwrap();
        assert_eq!(elements.len(),1);
        assert_eq!(elements[0].min,[3.0/16.0-0.5,1.0/16.0-0.5,-0.25]);
        assert_eq!(elements[0].max,[13.0/16.0-0.5,15.0/16.0-0.5,0.25]);
    }
    #[test]fn rotated_or_multipart_models_do_not_invent_geometry(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/rot.json",
            br#"{"variants":{"":{"model":"test:block/rot","x":90}}}"#).unwrap();
        bank.insert("assets/test/models/block/rot.json",
            br#"{"elements":[{"from":[0,0,0],"to":[16,16,16]}]}"#).unwrap();
        assert_eq!(bank.elements(&s("test:rot",&[])).unwrap(),vec![Box3::FULL]);
    }
    #[test]fn quarter_turn_variant_rotates_exact_cuboid_and_face_texture(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/lever.json",
            br#"{"variants":{"":{"model":"test:block/lever","y":90}}}"#).unwrap();
        bank.insert("assets/test/models/block/lever.json",
            br#"{"textures":{"north":"test:block/north","east":"test:block/east"},
                "elements":[{"from":[0,0,0],"to":[4,12,16]}]}"#).unwrap();
        let s=s("test:lever",&[]);
        let a=bank.elements(&s).unwrap();
        assert_eq!(a[0].max[2],-0.25);
        assert_eq!(bank.texture(&s,Face::PosX),Some("test:north".into()));
        assert_eq!(bank.texture(&s,Face::PosZ),Some("test:east".into()));
    }
    #[test]fn unsupported_variant_rotation_fail_closed(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/bad.json",
            br#"{"variants":{"":{"model":"test:block/bad","y":45}}}"#).unwrap();
        bank.insert("assets/test/models/block/bad.json",
            br#"{"elements":[{"from":[0,0,0],"to":[16,16,16]}]}"#).unwrap();
        assert!(bank.elements(&s("test:bad",&[])).is_none());
        assert_eq!(bank.texture(&s("test:bad",&[]),Face::Top),None);
    }
    #[test]fn uvlock_corners_preserve_world_texture_basis_with_any_variant_turn(){
        for x in [0,90,180,270]{
            for y in [0,90,180,270]{
                for face in [Face::Top,Face::Bottom,Face::PosX,
                    Face::NegX,Face::PosZ,Face::NegZ]{
                    let permutation=uvlock_permutation(face,x,y).unwrap();
                    let mut sorted=permutation;
                    sorted.sort_unstable();
                    assert_eq!(sorted,[0,1,2,3]);
                }
            }
        }
        assert_eq!(uvlock_permutation(Face::Top,0,0),Some([0,1,2,3]));
        assert_ne!(uvlock_permutation(Face::Top,0,90),Some([0,1,2,3]));
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/turn.json",
            br#"{"variants":{"":{"model":"test:block/turn","y":90,"uvlock":true}}}"#).unwrap();
        bank.insert("assets/test/models/block/turn.json",
            br##"{"elements":[{"from":[0,0,0],"to":[16,16,16],
                "faces":{"up":{"texture":"test:block/checker"}}}]}"##).unwrap();
        let st=s("test:turn",&[]);
        assert_eq!(bank.parts(&st).unwrap().len(),1);
        assert_ne!(bank.element_face(&st,Face::Top,0).unwrap().uvlock,[0,1,2,3]);
    }
    #[test]fn json_cullface_is_optional_and_y_variant_rotates_world_direction(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/rotor.json",
            br#"{"variants":{"":{"model":"test:block/rotor","y":90}}}"#).unwrap();
        bank.insert("assets/test/models/block/rotor.json",
            br##"{"elements":[{"from":[0,0,0],"to":[16,16,16],
                "faces":{"east":{"texture":"test:block/a","cullface":"east"},
                         "up":{"texture":"test:block/b"}}}]}"##).unwrap();
        let state=s("test:rotor",&[]);
        assert_eq!(bank.face_cull_direction(&state,Face::PosX,0),
            Some(Some(Face::PosZ)));
        assert_eq!(bank.face_cull_direction(&state,Face::Top,0),Some(None));
        assert_eq!(bank.face_cull_direction(&state,Face::NegX,0),None);
        assert_eq!(bank.face_cull_direction(&state,Face::Top,1),None);
    }
    #[test]fn explicit_json_model_only_renders_declared_faces_with_original_local_uvs(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/rotator.json",
            br#"{"variants":{"":{"model":"test:block/rotator","y":90}}}"#).unwrap();
        bank.insert("assets/test/models/block/rotator.json",
            br##"{"textures":{"end":"test:block/red"},"elements":[
                {"from":[0,0,0],"to":[8,16,16],
                 "faces":{"east":{"texture":"#end","uv":[1,2,8,14]}}}]}"##).unwrap();
        let state=s("test:rotator",&[]);
        assert_eq!(bank.face_defined(&state,Face::PosX,0),Some(true));
        assert_eq!(bank.face_defined(&state,Face::NegZ,0),Some(false));
        assert_eq!(bank.face_defined(&state,Face::Top,0),Some(false));
        let selected=bank.element_face(&state,Face::PosX,0).unwrap();
        assert_eq!(selected.uv,[1.0,2.0,8.0,14.0]);
        assert_eq!(selected.texture,"test:red");
    }
    #[test]fn absent_vanilla_cube_parent_recovers_geometry_and_face_culling(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/bricks.json",
            br#"{"variants":{"":{"model":"test:block/bricks"}}}"#).unwrap();
        bank.insert("assets/test/models/block/bricks.json",
            br##"{"parent":"minecraft:block/cube_all",
                "textures":{"all":"test:block/terracotta"}}"##).unwrap();
        let st=s("test:bricks",&[]);
        assert_eq!(bank.parts(&st).unwrap().len(),1);
        assert_eq!(bank.parts(&st).unwrap()[0].bounds,Box3::FULL);
        for face in [Face::Top,Face::Bottom,Face::NegX,
            Face::PosX,Face::NegZ,Face::PosZ]{
            assert_eq!(bank.face_defined(&st,face,0),Some(true));
            assert_eq!(bank.face_cull_direction(&st,face,0),Some(Some(face)));
            assert_eq!(bank.element_face(&st,face,0).unwrap().texture,
                "test:terracotta");
        }
        assert!(vanilla_parent_elements("model:test:cube_all").is_none());
    }
    #[test]fn vanilla_cube_column_parent_preserves_end_side_texture_aliases(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/log.json",
            br#"{"variants":{"":{"model":"test:block/log"}}}"#).unwrap();
        bank.insert("assets/test/models/block/log.json",
            br##"{"parent":"minecraft:block/cube_column",
                "textures":{"end":"test:block/cap","side":"test:block/bark"}}"##).unwrap();
        let st=s("test:log",&[]);
        assert_eq!(bank.element_face(&st,Face::Top,0).unwrap().texture,"test:cap");
        assert_eq!(bank.element_face(&st,Face::NegZ,0).unwrap().texture,"test:bark");
    }
    #[test]fn vanilla_cross_parent_keeps_two_zero_thickness_quads(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/flower.json",
            br#"{"variants":{"":{"model":"test:block/flower"}}}"#).unwrap();
        bank.insert("assets/test/models/block/flower.json",
            br##"{"parent":"minecraft:block/cross",
                "textures":{"cross":"test:block/flower_png"}}"##).unwrap();
        let st=s("test:flower",&[]);
        let parts=bank.parts(&st).unwrap();
        assert_eq!(parts.len(),2);
        assert_eq!(parts[0].bounds.min[2],parts[0].bounds.max[2]);
        assert!(parts.iter().all(|p|p.rotation.is_some()));
        assert_eq!(bank.face_defined(&st,Face::Top,0),Some(false));
        assert_eq!(bank.face_defined(&st,Face::NegZ,0),Some(true));
        assert_eq!(bank.element_face(&st,Face::NegZ,0).unwrap().uv,
            [0.0,0.0,16.0,16.0]);
        assert_eq!(bank.element_face(&st,Face::NegZ,1).unwrap().texture,
            "test:flower_png");
    }
    #[test]fn vanilla_implicit_uv_preserves_partial_element_face_dimensions(){
        let slab:Value=serde_json::from_str(
            r#"{"from":[2,0,3],"to":[14,8,13]}"#
        ).unwrap();
        assert_eq!(implicit_face_uv(&slab,Face::Top),
            Some([2.0,3.0,14.0,13.0]));
        assert_eq!(implicit_face_uv(&slab,Face::Bottom),
            Some([2.0,3.0,14.0,13.0]));
        assert_eq!(implicit_face_uv(&slab,Face::NegZ),
            Some([2.0,8.0,14.0,16.0]));
        assert_eq!(implicit_face_uv(&slab,Face::PosZ),
            Some([2.0,8.0,14.0,16.0]));
        assert_eq!(implicit_face_uv(&slab,Face::NegX),
            Some([3.0,8.0,13.0,16.0]));
        assert_eq!(implicit_face_uv(&slab,Face::PosX),
            Some([3.0,8.0,13.0,16.0]));
    }
    #[test]fn inherited_model_missing_uv_uses_element_bounds_not_full_texture(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/platform.json",
            br#"{"variants":{"":{"model":"test:block/platform"}}}"#).unwrap();
        bank.insert("assets/test/models/block/platform.json",
            br##"{"textures":{"top":"test:block/wood"},"elements":[
                {"from":[0,0,0],"to":[16,8,16],
                 "faces":{"north":{"texture":"#top"}}}]}"##).unwrap();
        let f=bank.element_face(&s("test:platform",&[]),Face::NegZ,0).unwrap();
        assert_eq!(f.uv,[0.0,8.0,16.0,16.0]);
        assert_eq!(f.texture,"test:wood");
    }
    #[test]fn per_element_face_texture_alias_crop_and_quarter_turn(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/machine.json",
            br#"{"variants":{"":{"model":"test:block/machine"}}}"#).unwrap();
        bank.insert("assets/test/models/block/machine.json",
            br##"{"textures":{"surface":"test:block/casing"},
              "elements":[{"from":[0,0,0],"to":[16,16,16],
                "faces":{"north":{"texture":"#surface","uv":[4,2,12,14],"rotation":90},
                         "up":{"texture":"test:block/top"}}}]}"##).unwrap();
        let state=s("test:machine",&[]);
        assert_eq!(bank.element_face(&state,Face::NegZ,0),
            Some(FaceSpec{texture:"test:casing".into(),uv:[4.0,2.0,12.0,14.0],
                rotation:90,uvlock:[0,1,2,3]}));
        assert_eq!(bank.element_face(&state,Face::Top,0).unwrap().texture,"test:top");
        assert!(bank.element_face(&state,Face::PosX,0).is_none());
        assert!(bank.element_face(&state,Face::NegZ,1).is_none());
    }
    #[test]fn multipart_conditions_flatten_matching_models_and_keep_element_faces(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/gate.json",
            br#"{"multipart":[
                {"apply":{"model":"test:block/post"}},
                {"when":{"north":"true|auto"},"apply":{"model":"test:block/north"}},
                {"when":{"OR":[{"east":"true"},{"west":"true"}]},
                 "apply":{"model":"test:block/east"}}]}"#).unwrap();
        bank.insert("assets/test/models/block/post.json",
            br##"{"textures":{"all":"test:block/post"},"elements":[
                {"from":[7,0,7],"to":[9,16,9],
                 "faces":{"up":{"texture":"#all"}}}]}"##).unwrap();
        bank.insert("assets/test/models/block/north.json",
            br##"{"textures":{"all":"test:block/north"},"elements":[
                {"from":[7,6,0],"to":[9,9,7],
                 "faces":{"north":{"texture":"#all"}}}]}"##).unwrap();
        bank.insert("assets/test/models/block/east.json",
            br##"{"textures":{"all":"test:block/east"},"elements":[
                {"from":[9,6,7],"to":[16,9,9],
                 "faces":{"east":{"texture":"#all"}}}]}"##).unwrap();
        let state=s("test:gate",&[("north","true"),("east","false")]);
        assert_eq!(bank.elements(&state).unwrap().len(),2);
        assert_eq!(bank.element_face(&state,Face::Top,0).unwrap().texture,
            "test:post");
        assert_eq!(bank.element_face(&state,Face::NegZ,1).unwrap().texture,
            "test:north");
        let all=s("test:gate",&[("north","true"),("east","true")]);
        assert_eq!(bank.elements(&all).unwrap().len(),3);
        let none=s("test:gate",&[]);
        assert_eq!(bank.elements(&none).unwrap().len(),1);
    }
    #[test]fn nested_multipart_condition_semantics_fail_closed_for_invalid_data(){
        let state=s("test:machine",&[("on","true"),("axis","z")]);
        let good:Value=serde_json::from_str(
            r#"{"AND":[{"on":"false|true"},{"OR":[{"axis":"x"},{"axis":"z"}]}]}"#
        ).unwrap();
        assert_eq!(condition_matches(&good,&state,0),Some(true));
        let bad:Value=serde_json::from_str(r#"{"OR":[]}"#).unwrap();
        assert_eq!(condition_matches(&bad,&state,0),None);
        let missing:Value=serde_json::from_str(r#"{"axis":"x"}"#).unwrap();
        assert_eq!(condition_matches(&missing,&state,0),Some(false));
    }
    #[test]fn vanilla_crossed_plant_planes_keep_opposing_faces_and_no_shade(){
        let mut bank=ModelBank::default();
        bank.insert("assets/minecraft/blockstates/oak_sapling.json",
            br#"{"variants":{"":{"model":"minecraft:block/oak_sapling"}}}"#).unwrap();
        bank.insert("assets/minecraft/models/block/oak_sapling.json",
            br##"{"parent":"minecraft:block/cross",
                "textures":{"cross":"minecraft:block/oak_sapling"}}"##).unwrap();
        bank.insert("assets/minecraft/models/block/cross.json",
            br##"{"ambientocclusion":false,
              "textures":{"particle":"#cross"},
              "elements":[
                {"from":[0.8,0,8],"to":[15.2,16,8],
                 "rotation":{"origin":[8,8,8],"axis":"y","angle":45,"rescale":true},
                 "shade":false,
                 "faces":{"north":{"uv":[0,0,16,16],"texture":"#cross"},
                          "south":{"uv":[0,0,16,16],"texture":"#cross"}}},
                {"from":[8,0,0.8],"to":[8,16,15.2],
                 "rotation":{"origin":[8,8,8],"axis":"y","angle":45,"rescale":true},
                 "shade":false,
                 "faces":{"west":{"uv":[0,0,16,16],"texture":"#cross"},
                          "east":{"uv":[0,0,16,16],"texture":"#cross"}}}
              ]}"##).unwrap();
        let state=s("minecraft:oak_sapling",&[]);
        let parts=bank.parts(&state).unwrap();
        assert_eq!(parts.len(),2);
        assert!(parts[0].rotation.is_some());
        assert_eq!(parts[0].bounds.min[2],parts[0].bounds.max[2]);
        assert_eq!(parts[1].bounds.min[0],parts[1].bounds.max[0]);
        assert_eq!(bank.face_defined(&state,Face::Top,0),Some(false));
        assert_eq!(bank.face_defined(&state,Face::NegZ,0),Some(true));
        assert_eq!(bank.face_defined(&state,Face::PosX,1),Some(true));
        assert_eq!(bank.element_unshaded(&state,0),Some(true));
        assert_eq!(bank.element_unshaded(&state,1),Some(true));
        assert_eq!(bank.element_face(&state,Face::NegZ,0).unwrap().texture,
            "minecraft:oak_sapling");
    }
    #[test]fn flat_plane_without_declared_normal_face_is_not_invented(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/invalid.json",
            br#"{"variants":{"":{"model":"test:block/invalid"}}}"#).unwrap();
        bank.insert("assets/test/models/block/invalid.json",
            br#"{"elements":[{"from":[1,0,8],"to":[15,16,8],
                "faces":{"up":{"texture":"test:block/leaf"}}}]}"#).unwrap();
        assert!(bank.parts(&s("test:invalid",&[])).is_none());
    }
    #[test]fn tilted_resource_model_parts_preserve_true_vertices(){
        let mut bank=ModelBank::default();
        bank.insert("assets/test/blockstates/lever.json",
            br#"{"variants":{"":{"model":"test:block/handle"}}}"#).unwrap();
        bank.insert("assets/test/models/block/handle.json",
            br#"{"elements":[{"from":[7,1,7],"to":[9,14,9],
               "rotation":{"axis":"z","origin":[8,8,8],"angle":22.5}}]}"#).unwrap();
        let state=s("test:lever",&[]);
        let parts=bank.parts(&state).unwrap();
        assert_eq!(parts.len(),1);
        assert!(parts[0].rotation.is_some());
        assert!(!parts[0].boundary(1,true));
        assert!(bank.elements(&state).is_none()); // legacy AABB route refuses
    }
    #[test]fn texture_variants_match_all_state_properties_without_guessing(){
        assert!(variant_matches("axis=y,facing=north",&s("x:block",&[("axis","y"),("facing","north")])));
        assert!(!variant_matches("axis=x",&s("x:block",&[("axis","y")])));
        assert!(!variant_matches("unsupported",&s("x:block",&[])));
    }
}