//! Native Rust + iced desktop with legacy MCEdit2 positional world arguments.
mod isometric;
mod slice_canvas;
mod gpu_mesh;
mod gpu_scene;
mod palette;
mod textures;
mod models;
mod resource_models;
use iced::widget::{button,canvas,column,container,row,scrollable,text,text_input};
use mcedit2_world::surface::SurfaceView;
use isometric::{Face,IsometricSurface};
use gpu_scene::GpuScene;
use slice_canvas::{Map2D,Plane};
use iced::{Element,Length,Theme};
use mcedit2_world::{compatibility_label,inspect_input,launch,WorldSummary};
use mcedit2_world::block_lookup::{parse_xyz,read_world_block};
use mcedit2_world::editing::{parse_bounds,parse_state,BeScalarPreview,Bounds,BrushShape,Preview,Session,Slice,SliceCell,VerticalSlice};
use mcedit2_world::region_plan::NativeRegionCandidate;
use mcedit2_world::clipboard::HorizontalTransform;
use mcedit2_world::region_transaction::{apply_single_region,apply_authenticated_be_single_region,finalize_single_region,rollback_pending_region};
use std::collections::{BTreeMap,VecDeque};
use mcedit2_world::far_view::{self,FarChunkRequest,FarChunkPatch};
use std::path::PathBuf;
use std::time::{SystemTime,UNIX_EPOCH};

/// Snapshot of the exact single-Region image explicitly approved in the UI.
/// No file is ever changed merely by generating this preview.
/// Snapshot-only read dispatch: the iced thread must never block on
/// decoding original chunks, and results must not cross a world/dimension
/// switch, a newer request, or an in-memory edit.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
enum NearMode{Surface,Volume,Horizontal,Vertical}
struct NearJob{
    nonce:u64,key:(usize,usize),mode:NearMode,bounds:Bounds,
    cut:Option<i32>,source:Session,
}
#[derive(Debug,Clone)]
enum NearResult{
    Surface(Result<SurfaceView,String>),
    Horizontal(Result<Slice,String>),
    Vertical(Result<VerticalSlice,String>),
    WorkerStopped,
}
fn dispatch_near(job:NearJob)->iced::Task<Message>{
    iced::Task::perform(async move {
        let (nonce,key)=(job.nonce,job.key);
        let (sender,receiver)=iced::futures::channel::oneshot::channel();
        std::thread::spawn(move ||{
            let b=job.bounds;
            let result=match job.mode{
                NearMode::Surface|NearMode::Volume=>{
                    let width=(b.maximum[0] as i64-b.minimum[0] as i64+1) as u8;
                    let depth=(b.maximum[2] as i64-b.minimum[2] as i64+1) as u8;
                    let read=if job.mode==NearMode::Volume{
                        mcedit2_world::surface::inspect_exposed_volume_cutaway(
                            &job.source,b.minimum[0],b.minimum[2],
                            b.minimum[1],b.maximum[1],width,depth,job.cut)
                    }else{
                        job.source.inspect_surface(b.minimum[0],b.minimum[2],
                            b.minimum[1],b.maximum[1],width,depth)
                    };
                    NearResult::Surface(read.map_err(|e|e.to_string()))
                }
                NearMode::Horizontal=>NearResult::Horizontal(
                    job.source.inspect_slice(b).map_err(|e|e.to_string())),
                NearMode::Vertical=>NearResult::Vertical(
                    job.source.inspect_vertical_slice(b).map_err(|e|e.to_string())),
            };
            let _=sender.send(result);
        });
        (nonce,key,receiver.await.unwrap_or(NearResult::WorkerStopped))
    },|(nonce,key,result)|Message::NearLoaded(nonce,key,result))
}

/// World directory/level.dat indexing also belongs off the iced UI loop.
fn dispatch_open(nonce:u64,path:PathBuf)->iced::Task<Message>{
    iced::Task::perform(async move {
        let (sender,receiver)=iced::futures::channel::oneshot::channel();
        std::thread::spawn(move ||{
            let result=inspect_input(&path).map_err(|e|e.to_string());
            let _=sender.send(result);
        });
        (nonce,receiver.await.unwrap_or_else(|_|
            Err("Original world indexing worker stopped".into())))
    },|(nonce,result)|Message::OpenFinished(nonce,result))
}
struct SaveIntent {
    key:(usize,usize),
    timestamp:u32,
    candidate:NativeRegionCandidate,
    be_mode:bool,
}
fn parse_region_name(name:&str)->Result<(i32,i32,&str),String>{
    let mut parts=name.trim().split('.');
    if parts.next()!=Some("r"){return Err("Use r.X.Z.mca or r.X.Z.mcr".into());}
    let rx=parts.next().ok_or("Missing Region X")?.parse::<i32>()
        .map_err(|_|"Region X must be a signed integer")?;
    let rz=parts.next().ok_or("Missing Region Z")?.parse::<i32>()
        .map_err(|_|"Region Z must be a signed integer")?;
    let ext=parts.next().ok_or("Missing Region extension")?;
    if !matches!(ext,"mca"|"mcr")||parts.next().is_some(){
        return Err("Only original r.X.Z.mca or r.X.Z.mcr filenames are accepted".into());
    }
    Ok((rx,rz,ext))
}
fn slice_cell_label(cell:&SliceCell)->String{
    match cell {
        SliceCell::MissingChunk => "·".into(),
        SliceCell::MissingSection => "?".into(),
        SliceCell::Present { original_block_entity:true,.. } => "@".into(),
        SliceCell::Present { state,.. } => state.name.rsplit(':')
            .next().unwrap_or("?").chars().next()
            .unwrap_or('?').to_ascii_uppercase().to_string(),
    }
}
fn shifted_slice(bounds:Bounds,dx:i32,dy:i32,dz:i32)->Result<String,String>{
    let add=|v:i32,delta:i32|v.checked_add(delta)
        .ok_or_else(||"Slice pan coordinate overflow".to_owned());
    let first=(add(bounds.minimum[0],dx)?,add(bounds.minimum[1],dy)?,
        add(bounds.minimum[2],dz)?);
    let last=(add(bounds.maximum[0],dx)?,add(bounds.maximum[1],dy)?,
        add(bounds.maximum[2],dz)?);
    Bounds::new(first,last).map_err(|e|e.to_string())?;
    Ok(format!("{},{},{};{},{},{}",first.0,first.1,first.2,
        last.0,last.1,last.2))
}

fn parse_scene_window(input:&str)->Result<Bounds,String>{
    let (first,last)=input.split_once(';')
        .ok_or("3D scene requires X1,Y1,Z1;X2,Y2,Z2")?;
    if last.contains(';'){return Err("Too many 3D viewport separators".into());}
    let a=mcedit2_world::block_lookup::parse_xyz(first).map_err(|e|e.to_string())?;
    let b=mcedit2_world::block_lookup::parse_xyz(last).map_err(|e|e.to_string())?;
    let minimum=[a.0.min(b.0),a.1.min(b.1),a.2.min(b.2)];
    let maximum=[a.0.max(b.0),a.1.max(b.1),a.2.max(b.2)];
    for v in [minimum,maximum]{
        Bounds::new((v[0],v[1],v[2]),(v[0],v[1],v[2]))
            .map_err(|e|e.to_string())?;
    }
    for (index,maximum_size) in [(0,64),(1,48),(2,64)]{
        if maximum[index] as i64-minimum[index] as i64+1>maximum_size{
            return Err("3D view is limited to 64x64x48 source cells".into());
        }
    }
    // A display-only selection can exceed the unchanged 4096 edit limit.
    // The strict read-only 196608-source-cell limit is enforced by the backend.
    Ok(Bounds{minimum,maximum})
}
fn shifted_scene(input:&str,dx:i32,dy:i32,dz:i32)->Result<String,String>{
    let area=parse_scene_window(input)?;
    let shift=|p:[i32;3]|->Result<[i32;3],String>{
        Ok([
            p[0].checked_add(dx).ok_or("Scene X overflow")?,
            p[1].checked_add(dy).ok_or("Scene Y overflow")?,
            p[2].checked_add(dz).ok_or("Scene Z overflow")?,
        ])
    };
    let a=shift(area.minimum)?;let b=shift(area.maximum)?;
    let text=format!("{},{},{};{},{},{}",a[0],a[1],a[2],b[0],b[1],b[2]);
    parse_scene_window(&text)?;
    Ok(text)
}

/// Only exact same source chunk + requested LOD can be reused. Never
/// use a cached distant sample for editor selection or source provenance.
const FAR_CACHE_PATCHES:usize=4096;
const FAR_CACHE_VOXELS:usize=90_000;
type FarCacheKey=(i32,i32,u8);
struct FarProgress{
    nonce:u64,key:(usize,usize),center:(i32,i32),radius:u16,
    /// Immutable source snapshot: only display navigation can reuse it;
    /// any staging/undo/commit invalidates this entire FarProgress.
    source:std::sync::Arc<Session>,
    remaining:VecDeque<FarChunkRequest>,total:usize,complete:usize,errors:usize,
    cache:BTreeMap<FarCacheKey,std::sync::Arc<FarChunkPatch>>,
    cache_order:VecDeque<FarCacheKey>,
    cached_voxels:usize,
}
fn far_cache_key(req:FarChunkRequest)->FarCacheKey{(req.cx,req.cz,req.span)}
impl FarProgress {
    fn remember(&mut self,req:FarChunkRequest,patch:&std::sync::Arc<FarChunkPatch>){
        let key=far_cache_key(req);
        if self.cache.contains_key(&key){return;}
        // A large original chunk cannot starve future distant reuse.
        if patch.voxels.len()>FAR_CACHE_VOXELS{return;}
        self.cached_voxels+=patch.voxels.len();
        self.cache.insert(key,std::sync::Arc::clone(patch));
        self.cache_order.push_back(key);
        while self.cache.len()>FAR_CACHE_PATCHES
            ||self.cached_voxels>FAR_CACHE_VOXELS{
            let Some(old)=self.cache_order.pop_front()else{break};
            if let Some(evicted)=self.cache.remove(&old){
                self.cached_voxels=self.cached_voxels.saturating_sub(evicted.voxels.len());
            }
        }
    }
}
#[derive(PartialEq,Eq)]
struct RenderSignature{
    /// World selection, source snapshot address and sampled source counts.
    source:Option<(usize,usize,usize,usize,usize,usize)>,
    geometry:Option<(usize,usize)>,
    camera:u8,yaw:i32,pitch:i32,zoom:u32,pan_x:u32,pan_y:u32,
    show_textures:bool,
}
#[derive(Default)]
struct App{
    path:String,
    surface_area:String,
    surface_exposed:bool,
    surface_cut_top:Option<i32>,
    surface_frame:bool,
    surface_axes:bool,
    surface_chunk_grid:bool,
    surface_section_guides:bool,
    show_textures:bool,
    use_gpu:bool,
    gpu_quality:gpu_mesh::Quality,
    texture_bank:Option<std::sync::Arc<textures::TextureBank>>,
    texture_animation_tick:u64,
    /// Persist projected faces across reactive iced view rebuilds. Only
    /// geometry/source/camera invalidates it; hover and PNG ticks do not.
    render_face_cache:std::sync::Arc<
        std::sync::Mutex<Option<(iced::Size,
            std::sync::Arc<Vec<isometric::RenderFace>>)>>,
    >,
    render_gpu_cache:std::sync::Arc<
        std::sync::Mutex<Option<(iced::Size,
            std::sync::Arc<Vec<gpu_mesh::Vertex>>)>>,
    >,
    texture_path:String,
    texture_report:String,
    surface_camera:u8,
    surface_yaw:i32,
    surface_pitch:i32,
    surface_zoom:f32,
    surface_pan_x:f32,
    surface_pan_y:f32,
    selected_surface:Option<(i32,i32,i32)>,
    selected_surface_face:Option<Face>,
    corner_a:Option<(i32,i32,i32)>,
    corner_b:Option<(i32,i32,i32)>,
    surface_hover:Option<(i32,i32,i32)>,
    surface:Option<(usize,usize,std::sync::Arc<SurfaceView>)>,
    far:Option<FarProgress>,far_nonce:u64,
    near_nonce:u64,
    near_job:Option<NearJob>,
    near_busy:bool,
    open_nonce:u64,
    open_busy:bool,
    open_pending:Option<(u64,PathBuf)>,
    surface_report:String,
    slice_grid:Option<(usize,usize,Slice)>,
    vertical_grid:Option<(usize,usize,VerticalSlice)>,
    vertical_area:String,
    vertical_report:String,
    worlds:Vec<WorldSummary>,
    current:Option<usize>,
    dimension_index:usize,
    query_coordinates:String,
    block_result:String,
    slice_area:String,
    slice_result:String,
    edit_area:String,
    edit_target:String,
    edit_filter:String,
    be_scalar_field:String,
    be_scalar_value:String,
    be_snapshot_path:String,
    entity_snapshot_path:String,
    be_journal_path:String,
    be_batch_path:String,
    plugin_manifest_path:String,
    region_batch_path:String,
    clone_destination:String,
    fill_seed:String,
    patch_path:String,
    clipboard_path:String,
    clipboard_destination:String,
    schematic_path:String,
    sponge_path:String,
    save_phrase:String,
    recover_region:String,
    recovery_phrase:String,
    save_preview:Option<SaveIntent>,
    be_save_phrase:String,
    be_prepared:Option<(usize,usize,BeScalarPreview)>,
    edit_report:String,
    sessions:BTreeMap<(usize,usize),Session>,
    prepared:Option<(usize,usize,Preview)>,
    messages:Vec<String>,
    debug:bool,log:bool,verbosity:u8,readonly:bool,
}
#[derive(Debug,Clone)]
enum Message{
    Path(String),Open,OpenFinished(u64,Result<WorldSummary,String>),
    Select(usize),Coordinates(String),Dimension(usize),
    TexturePath(String),LoadTextures,UnloadTextures,ToggleTextures,ToggleGpu,CycleGpuQuality,
    TextureTick,
    SurfaceArea(String),InspectSurface,InspectVolume,PickSurface(i32,i32,i32),RotateSurface,
    PickSurfaceFace(i32,i32,i32,Face),
    Shift3D(i32,i32,i32),Center3DOnPicked,ScenePreset(u8),
    StartFarView(u16),ShiftFarView(i32,i32),
    FarBatchLoaded(u64,Vec<(FarChunkRequest,Result<FarChunkPatch,String>)>),
    NearLoaded(u64,(usize,usize),NearResult),
    Toggle3DFrame,Toggle3DAxes,Toggle3DChunkGrid,Toggle3DSections,
    Cut3DTop(i32),Reset3DCut,
    Set3DCornerA,Set3DCornerB,Use3DBox,Clear3DBox,
    ZoomSurface(i8),PanSurface(i32,i32),ResetSurfaceCamera,
    OrbitSurface(i32),PitchSurface(i32),OrbitPitchSurface(i32,i32),
    HoverSurface(Option<(i32,i32,i32)>),
    BeScalarField(String),BeScalarValue(String),PreviewBeScalar,
    StageBeScalar,UndoBeScalar,RedoBeScalar,InspectStagedBe,InspectBeTree,
    BeSnapshotPath(String),ExportOriginalBe,VerifyOriginalBe,
    EntitySnapshotPath(String),InspectEntities,ExportEntities,VerifyEntities,
    BeJournalPath(String),ExportBeJournal,ImportBeJournal,
    BeBatchPath(String),PreviewBeBatch,StageBeBatch,
    PluginManifestPath(String),PreviewPlugin,
    InspectBlock,InspectOriginalBlockEntity,SliceText(String),InspectSlice,PickSlice(i32,i32,i32),MoveSlice(i32,i32,i32),
    VerticalText(String),InspectVertical,PickVertical(usize,usize),
    SelectionText(String),TargetText(String),FilterText(String),
    CloneDestination(String), PreviewClone,PreviewMove,
    FillSeed(String), PreviewFill,PreviewBrush(BrushShape),
    PatchPath(String),ExportPatch,ImportPatch,AnalyzeNativePatch,AnalyzeRegionPlan,
    ClipboardPath(String),ClipboardDestination(String),ExportClipboard,PreviewPasteClipboard,
    PreviewTransformedClipboard(HorizontalTransform),
    SchematicPath(String),PreviewLegacySchematic,ExportLegacySchematic,
    SpongePath(String),PreviewSpongeV2,ExportSpongeV2,PreviewSpongeV3,ExportSpongeV3,
    PreviewBeRegion,PrepareWorldSave,SavePhrase(String),ConfirmWorldSave,
    RegionBatchPath(String),PreviewRegionBatch,ExportRegionBatch,VerifyRegionBatch,
    PrepareBeSave,BeSavePhrase(String),ConfirmBeSave,
    RecoverRegion(String),RecoveryPhrase(String),FinalizeRegion,RollbackRegion,
    PreviewEdit,ApplyEdit,UndoEdit,RedoEdit,DiscardEdits,Clear
}
impl App{
    fn render_signature(&self)->RenderSignature{
        RenderSignature{
            source:self.surface.as_ref().map(|(wi,di,view)|{
                (*wi,*di,std::sync::Arc::as_ptr(view) as usize,
                    view.voxels.len(),view.source_tiles.len(),view.source_chunks)
            }),
            geometry:self.texture_bank.as_ref().map(|bank|bank.geometry_id()),
            camera:self.surface_camera,yaw:self.surface_yaw,pitch:self.surface_pitch,
            zoom:self.surface_zoom.to_bits(),pan_x:self.surface_pan_x.to_bits(),
            pan_y:self.surface_pan_y.to_bits(),
            show_textures:self.show_textures,
        }
    }
    fn invalidate_near(&mut self){
        self.near_nonce=self.near_nonce.wrapping_add(1);
        self.near_job=None;
    }
    fn enqueue_near(&mut self,key:(usize,usize),mode:NearMode,
        bounds:Bounds,cut:Option<i32>)->Result<(),String>{
        let source=self.current_session(key)?.clone();
        self.near_nonce=self.near_nonce.wrapping_add(1);
        self.near_job=Some(NearJob{
            nonce:self.near_nonce,key,mode,bounds,cut,source,
        });
        Ok(())
    }
    fn inspect_surface(&mut self,exposed_volume:bool){
        self.invalidate_near();
        self.far=None;
        self.surface_exposed=exposed_volume;
        if !exposed_volume{self.surface_cut_top=None;}
        self.surface=None;
        self.selected_surface=None;self.selected_surface_face=None;
        self.surface_hover=None;self.corner_a=None;self.corner_b=None;
        self.surface_report=match self.current_key(){
            None=>"Select a Java world first.".into(),
            Some(key)=>match parse_scene_window(&self.surface_area){
                Err(e)=>format!("Invalid 3D source bounds: {e}"),
                Ok(bounds)=>{
                    let width=bounds.maximum[0] as i64-bounds.minimum[0] as i64+1;
                    let depth=bounds.maximum[2] as i64-bounds.minimum[2] as i64+1;
                    if !(1..=64).contains(&width)||!(1..=64).contains(&depth){
                        "3D source footprint must fit 64x64.".into()
                    }else{
                        let mode=if exposed_volume{NearMode::Volume}
                            else{NearMode::Surface};
                        match self.enqueue_near(key,mode,bounds,self.surface_cut_top){
                            Ok(())=>format!(
                                "Loading original 3D {} in a read-only worker; UI remains responsive.",
                                if exposed_volume{"exposed volume"}else{"top surface"}),
                            Err(e)=>format!("3D source refused: {e}"),
                        }
                    }
                }
            }
        };
    }
    fn near_loaded(&mut self,nonce:u64,key:(usize,usize),result:NearResult){
        // Exactly one reader runs at a time, including when its result is
        // obsolete. Release the slot before the nonce/identity check, so the
        // newest queued request can start even after a stale completion.
        self.near_busy=false;
        // An earlier worker may finish after another world, region view,
        // staging operation or changed input. Its data must never publish.
        if nonce!=self.near_nonce||self.current_key()!=Some(key){return;}
        match result{
            NearResult::Surface(result)=>{
                self.surface_report=match result{
                    Ok(scene)=>{
                        let report=format!(
                            "Read-only 3D: {} original-visible voxels, {} unknown columns, {} source chunks; edit limit remains 4096.",
                            scene.voxels.len(),scene.missing_columns,scene.source_chunks);
                        self.surface=Some((key.0,key.1,std::sync::Arc::new(scene)));
                        report
                    }
                    Err(e)=>format!("3D source read refused: {e}"),
                };
            }
            NearResult::Horizontal(result)=>{
                self.slice_result=match result{
                    Ok(slice)=>{
                        let width=slice.rows.first().map_or(0,Vec::len);
                        let height=slice.rows.len();
                        let legend=if width<=16&&height<=16{
                            format!("\n{}",slice.text_map())
                        }else{String::new()};
                        self.slice_grid=Some((key.0,key.1,slice));
                        format!(
                            "Interactive {width}x{height} original 2D map. Gray=air, purple=missing chunk, amber=missing Section, gold=BE.{legend}")
                    }
                    Err(e)=>format!("Horizontal source read refused: {e}"),
                };
            }
            NearResult::Vertical(result)=>{
                self.vertical_report=match result{
                    Ok(slice)=>{
                        let width=slice.rows.first().map_or(0,Vec::len);
                        let height=slice.rows.len();
                        let legend=if width<=16&&height<=16{
                            format!("\n{}",slice.text_map())
                        }else{String::new()};
                        self.vertical_grid=Some((key.0,key.1,slice));
                        format!("Interactive {width}x{height} vertical source map. Highest Y first; unknown source cells are not editable.{legend}")
                    }
                    Err(e)=>format!("Vertical source read refused: {e}"),
                };
            }
            NearResult::WorkerStopped=>{
                self.surface_report="Background source read worker stopped; no partial data accepted.".into();
                self.slice_result=self.surface_report.clone();
                self.vertical_report=self.surface_report.clone();
            }
        }
    }
    fn selection_box(&self)->Option<Bounds>{
        let a=self.corner_a?;let b=self.corner_b?;
        Bounds::new(a,b).ok()
    }
    fn set_3d_corner(&mut self,first:bool){
        let Some(pos)=self.selected_surface else{
            self.surface_report="Click a visible original 3D voxel before setting a corner.".into();return;
        };
        if first{self.corner_a=Some(pos);}else{self.corner_b=Some(pos);}
        self.surface_report=match (self.corner_a,self.corner_b){
            (Some(a),Some(b))=>match Bounds::new(a,b){
                Ok(bounds)=>format!("3D selection A={a:?}, B={b:?}; {} original block positions. Use box to edit only after review.",
                    bounds.volume().unwrap_or(0)),
                Err(e)=>format!("3D box exceeds safe edit limits: {e}; choose closer corners."),
            },
            _=>format!("3D {} corner set to {pos:?}; select the other corner.",
                if first{"A"}else{"B"}),
        };
    }
    fn use_3d_box(&mut self){
        let Some(bounds)=self.selection_box() else{
            self.surface_report="Set two 3D corners within the 4096-block edit budget.".into();return;
        };
        let a=bounds.minimum;let b=bounds.maximum;
        self.edit_area=format!("{},{},{};{},{},{}",a[0],a[1],a[2],b[0],b[1],b[2]);
        self.save_preview=None;self.prepared=None;self.be_prepared=None;
        self.surface_report=format!("Copied {}-cell original XYZ box to edit bounds; no blocks staged. Preview before Apply.",
            bounds.volume().unwrap_or(0));
    }
    fn move_cutaway(&mut self,delta:i32){
        let Ok(bounds)=parse_scene_window(&self.surface_area) else{
            self.surface_report="Inspect a valid 3D scene before cutting".into();return;
        };
        let current=self.surface_cut_top.unwrap_or(bounds.maximum[1]);
        self.surface_cut_top=Some(current.saturating_add(delta)
            .clamp(bounds.minimum[1],bounds.maximum[1]));
        self.inspect_surface(true);
    }
    fn shift_3d(&mut self,dx:i32,dy:i32,dz:i32){
        match shifted_scene(&self.surface_area,dx,dy,dz){
            Ok(text)=>{self.surface_area=text;self.inspect_surface(self.surface_exposed);}
            Err(e)=>self.surface_report=format!("3D navigation refused: {e}"),
        }
    }
    fn center_3d(&mut self){
        let pos=self.selected_surface.or_else(||
            mcedit2_world::block_lookup::parse_xyz(&self.query_coordinates).ok());
        let Some((x,y,z))=pos else{
            self.surface_report="Pick an original XYZ before centering.".into();
            return;
        };
        let cx=x.div_euclid(16)*16;
        let cz=z.div_euclid(16)*16;
        let y0=y.saturating_sub(8);
        let y1=y0.saturating_add(15);
        let next=format!("{cx},{y0},{cz};{},{y1},{}",cx+15,cz+15);
        match parse_scene_window(&next){
            Ok(_)=>{self.surface_area=next;self.inspect_surface(self.surface_exposed);}
            Err(e)=>self.surface_report=format!("3D center refused: {e}"),
        }
    }
    fn start_far_view(&mut self,radius:u16,requested:Option<(i32,i32)>)
        ->iced::Task<Message>{
        self.invalidate_near();
        let previous=self.far.take();
        self.far_nonce=self.far_nonce.wrapping_add(1);
        self.surface=None;self.surface_cut_top=None;self.surface_hover=None;
        self.selected_surface=None;self.selected_surface_face=None;
        self.corner_a=None;self.corner_b=None;
        let Some(key)=self.current_key() else{
            self.surface_report="Select a Java world for distant terrain.".into();
            return iced::Task::none();
        };
        let center=requested.or_else(||
            parse_xyz(&self.query_coordinates).ok().map(|(x,_,z)|(x,z)))
            .or_else(||parse_scene_window(&self.surface_area).ok().and_then(|v|{
                let x=(v.minimum[0] as i64+v.maximum[0] as i64)/2;
                let z=(v.minimum[2] as i64+v.maximum[2] as i64)/2;
                Some((i32::try_from(x).ok()?,i32::try_from(z).ok()?))
            })).unwrap_or((0,0));
        // Pan/recenter keeps the immutable source snapshot so overlapping
        // chunk requests can reuse their already-decoded data. Mutations
        // clear self.far before entering this method.
        let compatible=previous.filter(|prior|prior.key==key);
        let source=if let Some(prior)=compatible.as_ref(){
            std::sync::Arc::clone(&prior.source)
        }else{
            match self.current_session(key){
                Ok(session)=>std::sync::Arc::new(session.clone()),
                Err(e)=>{self.surface_report=e;return iced::Task::none();}
            }
        };
        let version=source.original_world().data_version;
        let (y_min,y_max)=if version.is_some_and(|v|v>=2860){
            (-64,319)
        }else{(0,255)};
        let plan=match far_view::plan_far_chunks(center.0,center.1,radius){
            Ok(plan)=>plan,
            Err(e)=>{self.surface_report=e.to_string();return iced::Task::none();}
        };
        let mut view=match far_view::empty_far_view(center.0,center.1,
            y_min,y_max,radius){
            Ok(view)=>view,
            Err(e)=>{self.surface_report=e.to_string();return iced::Task::none();}
        };
        let (mut cache,mut order,mut cached_voxels)=compatible.map_or_else(
            ||(BTreeMap::new(),VecDeque::new(),0),
            |prior|(prior.cache,prior.cache_order,prior.cached_voxels));
        // If the user pan moves a far camera, fill the new view with cached
        // authentic source patches before scheduling fresh region reads.
        let mut pending=VecDeque::new();
        let mut reused=0;
        for request in plan{
            if let Some(patch)=cache.get(&far_cache_key(request)){
                far_view::append_chunk(&mut view,(**patch).clone());
                reused+=1;
            }else{
                pending.push_back(request);
            }
        }
        let total=pending.len()+reused;
        self.surface=Some((key.0,key.1,std::sync::Arc::new(view)));
        self.surface_exposed=false;
        self.surface_report=format!(
            "Streaming {radius} chunk radius: {reused} cached tiles, {} pending source reads. No entities or edits.",
            pending.len());
        self.far=Some(FarProgress{nonce:self.far_nonce,key,center,radius,
            source,total,complete:reused,errors:0,
            remaining:pending,cache,cache_order:order,cached_voxels});
        self.next_far_batch()
    }
    fn next_far_batch(&mut self)->iced::Task<Message>{
        let Some(progress)=self.far.as_mut() else{
            return iced::Task::none();
        };
        if progress.remaining.is_empty(){return iced::Task::none();}
        let take=progress.remaining.len().min(
            far_view::batch_size_for(progress.remaining.front().unwrap().span));
        let requests:Vec<_>=progress.remaining.drain(..take).collect();
        let (nonce,key)=(progress.nonce,progress.key);
        let source=std::sync::Arc::clone(&progress.source);
        let (min_y,max_y)=match self.surface.as_ref(){
            Some((wi,di,view))if(*wi,*di)==key=>(view.y_min,view.y_max),
            _=>return iced::Task::none(),
        };
        // Task::perform executes Futures; synchronous region reads inside
        // the future would monopolize its executor. Move only bounded
        // read-only I/O onto an OS worker and await its one-shot response.
        iced::Task::perform(async move {
            let fallback_requests=requests.clone();
            let (sender,receiver)=iced::futures::channel::oneshot::channel();
            std::thread::spawn(move ||{
                let mut region_cache=mcedit2_world::block_lookup::ReadOnlyRegionCache::new(
                    source.original_world(),source.dimension());
                let results=requests.into_iter().map(|req|{
                    let result=match &mut region_cache{
                        Ok(reader)=>far_view::inspect_far_chunk_cached(
                            &source,reader,req,min_y,max_y)
                            .map_err(|e|format!("chunk {},{}: {e}",req.cx,req.cz)),
                        Err(e)=>Err(format!("Distant original Region source refused: {e}")),
                    };
                    (req,result)
                }).collect::<Vec<_>>();
                let _=sender.send(results);
            });
            receiver.await.unwrap_or_else(|_|{
                fallback_requests.into_iter().map(|req|
                    (req,Err("Distant source read worker stopped unexpectedly".into()))
                ).collect()
            })
        },move |results|Message::FarBatchLoaded(nonce,results))
    }
    fn finish_far_batch(&mut self,nonce:u64,
        results:Vec<(FarChunkRequest,Result<FarChunkPatch,String>)>)
        ->iced::Task<Message>{
        let current=self.current_key();
        let Some(progress)=self.far.as_mut()else{
            return iced::Task::none();
        };
        if progress.nonce!=nonce||current!=Some(progress.key){
            return iced::Task::none();
        }
        let Some((wi,di,view))=self.surface.as_mut()else{
            return iced::Task::none();
        };
        if (*wi,*di)!=progress.key{return iced::Task::none();}
        for (request,result) in results{
            progress.complete+=1;
            match result{
                Ok(patch)=>{
                    let cached=std::sync::Arc::new(patch);
                    progress.remember(request,&cached);
                    far_view::append_chunk(std::sync::Arc::make_mut(view),(*cached).clone());
                },
                Err(e)=>{
                    progress.errors+=1;
                    if progress.errors<=3{
                        self.surface_report=format!("Distant source warning: {e}");
                    }
                }
            }
        }
        self.surface_report=format!(
            "Far terrain: {}/{} chunk tiles, {} source errors, radius {} chunks. Distant LOD is never an editable block.",
            progress.complete,progress.total,progress.errors,progress.radius);
        self.next_far_batch()
    }
    fn scene_preset(&mut self,side:u8){
        if !matches!(side,16|32|64){return;}
        let current=match parse_scene_window(&self.surface_area){
            Ok(bounds)=>bounds,
            Err(e)=>{self.surface_report=format!("Cannot resize viewport: {e}");return;}
        };
        // Preserve the existing low corner and current Y minimum. Enlarging
        // the view never stages an edit or increases the edit batch limit.
        let min=current.minimum;
        let far=match (min[0].checked_add(side as i32-1),
            min[2].checked_add(side as i32-1)){
            (Some(x),Some(z))=>(x,z),
            _=>{self.surface_report="Viewport coordinate overflow".into();return;}
        };
        let height=if side>=32{48}else{32};
        let Some(y1)=min[1].checked_add(height-1)else{
            self.surface_report="Viewport Y overflow".into();return;
        };
        let next=format!("{},{},{};{},{},{}",min[0],min[1],min[2],
            far.0,y1,far.1);
        match parse_scene_window(&next){
            Ok(_)=>{self.surface_area=next;self.surface_cut_top=None;
                self.inspect_surface(self.surface_exposed);}
            Err(e)=>self.surface_report=format!("Viewport resize refused: {e}")
        }
    }
    fn pick_surface(&mut self,x:i32,y:i32,z:i32){
        let Some((wi,di,view))=self.surface.as_ref() else{return};
        if self.current_key()!=Some((*wi,*di))
            ||!view.voxels.iter().any(|v|v.x==x&&v.y==y&&v.z==z){
            self.surface=None;
            self.surface_report="3D view changed; reload before editing.".into();
            return;
        }
        self.query_coordinates=format!("{x}, {y}, {z}");
        self.selected_surface=Some((x,y,z));
        self.edit_area=format!("{x},{y},{z};{x},{y},{z}");
        self.be_prepared=None;
        self.save_preview=None;self.prepared=None;
        self.surface_report=format!(
            "Selected exact original 3D voxel ({x},{y},{z}). Edits still require normal guarded preview and Apply."
        );
    }
    fn inspect_vertical_view(&mut self){
        self.invalidate_near();
        self.vertical_grid=None;
        self.vertical_report=match (self.current_key(),parse_bounds(&self.vertical_area)){
            (None,_)=>"Select a Java world first.".into(),
            (_,Err(e))=>format!("Invalid vertical area: {e}"),
            (Some(key),Ok(bounds))=>match self.enqueue_near(
                key,NearMode::Vertical,bounds,None){
                Ok(())=>"Reading vertical source plane in a read-only worker.".into(),
                Err(e)=>format!("Vertical source refused: {e}"),
            },
        };
    }
    fn pick_vertical(&mut self,row:usize,col:usize){
        let Some((wi,di,plane))=self.vertical_grid.as_ref() else{return};
        if self.current_key()!=Some((*wi,*di)){
            self.vertical_grid=None;
            self.vertical_report="Vertical plane stale; re-inspect selected dimension.".into();
            return;
        }
        let Some((x,y,z))=plane.coordinate(row,col) else{return};
        if !matches!(plane.rows.get(row).and_then(|line|line.get(col)),
            Some(SliceCell::Present{..})){
            self.vertical_report="Cannot pick missing original chunk/Section.".into();
            return;
        }
        self.edit_area=format!("{x},{y},{z};{x},{y},{z}");
        self.query_coordinates=format!("{x}, {y}, {z}");
        self.be_prepared=None;
        self.save_preview=None;
        self.prepared=None;
        self.block_result=format!("Selected vertical plane original block at ({x},{y},{z}); inspect the source Block Entity before editing.");
    }
    fn inspect_slice_view(&mut self){
        self.invalidate_near();
        self.slice_grid=None;
        self.slice_result=match (self.current_key(),parse_bounds(&self.slice_area)){
            (None,_)=>"Select an indexed Java world first.".into(),
            (_,Err(e))=>format!("Invalid 2D slice bounds: {e}"),
            (Some(key),Ok(bounds))=>match self.enqueue_near(
                key,NearMode::Horizontal,bounds,None){
                Ok(())=>"Reading horizontal source slice in a read-only worker.".into(),
                Err(e)=>format!("Horizontal source refused: {e}"),
            },
        };
    }
    fn pan_slice(&mut self,dx:i32,dy:i32,dz:i32){
        match parse_bounds(&self.slice_area)
            .map_err(|e|e.to_string())
            .and_then(|area|shifted_slice(area,dx,dy,dz)) {
            Ok(next)=>{
                self.slice_area=next;
                self.inspect_slice_view();
            }
            Err(e)=>self.slice_result=format!("Cannot move the slice: {e}"),
        }
    }
    fn pick_slice(&mut self,x:i32,y:i32,z:i32){
        let Some((world,dimension,slice))=self.slice_grid.as_ref() else {
            self.slice_result="No current interactive slice to select.".into();
            return;
        };
        if self.current_key()!=Some((*world,*dimension)) {
            self.slice_grid=None;
            self.slice_result="World or dimension switched; reload the slice.".into();
            return;
        }
        if !(slice.bounds.minimum[0]..=slice.bounds.maximum[0]).contains(&x)
            ||!(slice.bounds.minimum[2]..=slice.bounds.maximum[2]).contains(&z)
            ||y!=slice.bounds.minimum[1]{
            self.slice_result="Clicked coordinates outside original slice.".into();
            return;
        }
        let row=(z-slice.bounds.minimum[2]) as usize;
        let col=(x-slice.bounds.minimum[0]) as usize;
        let Some(cell)=slice.rows.get(row).and_then(|cells|cells.get(col)) else {
            self.slice_result="Slice contains no block at selected coordinate.".into();
            return;
        };
        let detail=match cell {
            SliceCell::MissingChunk=>"Missing original chunk; edits prohibited".into(),
            SliceCell::MissingSection=>"Missing original Section; edits prohibited".into(),
            SliceCell::Present {state,original_block_entity:true}=>format!(
                "{} (ORIGINAL Block Entity: replacement prohibited)",
                mcedit2_world::editing::render_state(state)
            ),
            SliceCell::Present {state,original_block_entity:false}=>
                mcedit2_world::editing::render_state(state),
        };
        self.query_coordinates=format!("{x}, {y}, {z}");
        self.edit_area=format!("{x},{y},{z};{x},{y},{z}");
        self.be_prepared=None;
        self.prepared=None;
        self.save_preview=None;
        self.save_phrase.clear();
        self.block_result=format!("Selected interactive 2D cell ({x},{y},{z}): {detail}");
    }
    fn clear_cached_world(&mut self, root:&std::path::Path) {
        self.prepared=None;
        self.save_preview=None;
        self.save_phrase.clear();
        self.sessions.retain(|(world_index,_),_|
            self.worlds.get(*world_index)
                .is_some_and(|world|world.root!=root)
        );
        self.block_result.clear();
        self.slice_result.clear();
        self.slice_grid=None;
        self.vertical_grid=None;
        self.surface=None;
        self.be_prepared=None;
    }
    fn current_world_context(&self)->Result<(usize,usize,WorldSummary,String),String>{
        let (wi,di)=self.current_key().ok_or("Select an original world and dimension")?;
        let world=self.worlds.get(wi).ok_or("Selected world disappeared")?.clone();
        let dimension=world.dimensions.get(di)
            .ok_or("Selected dimension disappeared")?.name.clone();
        Ok((wi,di,world,dimension))
    }
    fn prepare_save(&mut self){
        self.save_preview=None;
        self.save_phrase.clear();
        self.prepared=None;
        if self.readonly {
            self.edit_report="-view forbids writing Minecraft world files.".into();
            return;
        }
        let result=(||->Result<(SaveIntent,String),String>{
            let (wi,di,world,_)=self.current_world_context()?;
            if self.sessions.iter().any(|((index,_),session)|
                self.worlds.get(*index).is_some_and(|other|
                    other.root==world.root) && *index!=wi && session.has_pending_changes()
            ){
                return Err("Another window entry for this world has staged edits; close/discard duplicates before saving".into());
            }
            if self.sessions.iter().any(|((index,d),session)|
                self.worlds.get(*index).is_some_and(|other|
                    other.root==world.root) && (*index,*d)!=(wi,di)
                    && session.has_pending_changes()
            ){
                return Err("Another dimension has pending edits; export a journal first".into());
            }
            let now=SystemTime::now().duration_since(UNIX_EPOCH)
                .map_err(|_|"System time precedes UNIX epoch")?;
            let timestamp=u32::try_from(now.as_secs())
                .map_err(|_|"System clock cannot be stored in a Region timestamp")?;
            let mut plans=self.current_session((wi,di))?
                .prepare_native_region_candidates(timestamp)
                .map_err(|e|e.to_string())?;
            if plans.len()!=1 {
                return Err("Saving edits in more than ONE region is refused; export a journal or split edits".into());
            }
            let candidate=plans.pop().ok_or("No Region candidate")?;
            let label=format!(
                "CHECK ORIGINAL REGION: {}. {} edited chunk(s); {} original bytes -> {} proposed bytes. This WILL REPLACE the named Region FILE after immutable backup. Close Minecraft and other editors. Type SAVE to confirm. No files have been changed.",
                candidate.path.display(),candidate.modified_chunks,
                candidate.original_bytes,candidate.planned_bytes,
            );
            Ok((SaveIntent{key:(wi,di),timestamp,candidate,be_mode:false},label))
        })();
        match result {
            Ok((intent,label))=>{self.save_preview=Some(intent);self.edit_report=label;},
            Err(e)=>self.edit_report=format!("Disk save preflight refused: {e}"),
        }
    }
    fn prepare_be_save(&mut self){
        self.save_preview=None;
        self.save_phrase.clear();
        self.be_save_phrase.clear();
        self.be_prepared=None;
        if self.readonly{
            self.edit_report="-view forbids BE disk saves.".into();
            return;
        }
        let result=(||->Result<(SaveIntent,String),String>{
            let (wi,di,world,_)=self.current_world_context()?;
            if self.sessions.iter().any(|((index,d),session)|
                self.worlds.get(*index).is_some_and(|other|other.root==world.root)
                &&(*index,*d)!=(wi,di)&&session.has_pending_changes()
            ){
                return Err("Another world/dimension has staged edits; BE save requires exclusive original world ownership".into());
            }
            let timestamp=u32::try_from(SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_|"System clock before epoch")?.as_secs())
                .map_err(|_|"System clock cannot be represented as Region timestamp")?;
            let mut plans=self.current_session((wi,di))?
                .preview_be_region_candidates(timestamp).map_err(|e|e.to_string())?;
            if plans.len()!=1{
                return Err("BE save supports exactly one original Region; split or checkpoint multiple-Region edits".into());
            }
            let candidate=plans.pop().ok_or("Missing BE candidate")?;
            if candidate.be_replay.is_empty(){
                return Err("BE save requires one or more exact scalar proofs".into());
            }
            let msg=format!(
                "HIGH-RISK verified BE save: {}. {} original scalar proofs, {} replaced chunks, {} -> {} bytes. The original world MUST be closed and backed up. This writes ONE Region only, retaining an immutable original backup and a pending rollback marker. Type SAVE_BE_NBT exactly; use FINALIZE or ROLLBACK afterward.",
                candidate.path.display(),candidate.be_replay.len(),
                candidate.modified_chunks,candidate.original_bytes,
                candidate.planned_bytes
            );
            Ok((SaveIntent{key:(wi,di),timestamp,candidate,be_mode:true},msg))
        })();
        match result{
            Ok((intent,msg))=>{self.save_preview=Some(intent);self.edit_report=msg;},
            Err(e)=>self.edit_report=format!("BE save preflight refused: {e}"),
        }
    }
    fn confirm_be_save(&mut self){
        if self.readonly||self.be_save_phrase!="SAVE_BE_NBT"{
            self.edit_report="BE disk save requires exact SAVE_BE_NBT confirmation and non-read-only mode.".into();
            return;
        }
        let Some(intent)=self.save_preview.take()else{
            self.edit_report="Prepare a fresh BE Region disk save preview first.".into();
            return;
        };
        self.be_save_phrase.clear();
        if !intent.be_mode||intent.candidate.be_replay.is_empty()
            ||self.current_key()!=Some(intent.key){
            self.edit_report="Wrong save mode or original world changed; BE save refused.".into();
            return;
        }
        let result=(||->Result<_,String>{
            let (wi,di,world,dimension)=self.current_world_context()?;
            let current=self.current_session((wi,di))?
                .preview_be_region_candidates(intent.timestamp).map_err(|e|e.to_string())?;
            if current.len()!=1
                ||current[0].path!=intent.candidate.path
                ||current[0].original_region_sha256!=intent.candidate.original_region_sha256
                ||current[0].image!=intent.candidate.image{
                return Err("BE NBT, source world or staged blocks changed since preview; regenerate it".into());
            }
            let receipt=apply_authenticated_be_single_region(
                &world,&dimension,&intent.candidate,"SAVE_BE_NBT"
            ).map_err(|e|e.to_string())?;
            Ok((world.root,receipt))
        })();
        match result{
            Ok((root,receipt))=>{
                self.clear_cached_world(&root);
                self.edit_report=format!(
                    "Authenticated BE Region written: {}. Immutable original backup {}. PENDING marker {}. MUST FINALIZE or ROLLBACK before reopening/editing the world.",
                    receipt.region_path.display(),receipt.backup_path.display(),
                    receipt.marker_path.display()
                );
            },
            Err(e)=>{
                self.edit_report=format!(
                    "BE Region save refused/failed: {e}. If pending marker exists, resolve via FINALIZE/ROLLBACK before retrying."
                );
            }
        }
    }
    fn confirm_save(&mut self) {
        if self.readonly || self.save_phrase!="SAVE"{
            self.edit_report="Saving requires explicit SAVE confirmation and non-read-only mode.".into();
            return;
        }
        let Some(intent)=self.save_preview.take() else {
            self.edit_report="Prepare a fresh one-Region save preview first.".into();
            return;
        };
        self.save_phrase.clear();
        if intent.be_mode||!intent.candidate.be_replay.is_empty(){
            self.edit_report="Normal SAVE refuses original BE scalar changes; use SAVE_BE_NBT.".into();
            return;
        }
        if self.current_key()!=Some(intent.key){
            self.edit_report="Selected world changed since save preview. Refused.".into();
            return;
        }
        let result=(||->Result<_,String>{
            let (wi,di,world,dimension)=self.current_world_context()?;
            let current=self.current_session((wi,di))?
                .prepare_native_region_candidates(intent.timestamp)
                .map_err(|e|e.to_string())?;
            if current.len()!=1
                || current[0].path!=intent.candidate.path
                || current[0].original_region_sha256!=intent.candidate.original_region_sha256
                || current[0].image!=intent.candidate.image {
                return Err("World or staged blocks changed since preview; generate a new preview".into());
            }
            // No arbitrary image is accepted: the transaction independently
            // verifies exact NBT block-only replay, original sector bytes,
            // source world root, original file hash and backup state.
            let receipt=apply_single_region(&world,&dimension,&intent.candidate)
                .map_err(|e|e.to_string())?;
            Ok((world.root,receipt))
        })();
        match result {
            Ok((root,receipt))=>{
                self.clear_cached_world(&root);
                self.edit_report=format!(
                    "Single Region atomically replaced: {}. ORIGINAL backup: {}. PENDING recovery marker: {}. The write is NOT finalized. Enter the Region name below and FINALIZE or ROLLBACK. Do not open Minecraft until resolved.",
                    receipt.region_path.display(),receipt.backup_path.display(),
                    receipt.marker_path.display());
            }
            Err(e)=>self.edit_report=format!(
                "Disk save refused/failed: {e}. If an .mcedit2.pending.json marker exists, resolve it with the recovery controls BEFORE further edits."),
        }
    }
    fn recover_region(&mut self,rollback:bool){
        self.save_preview=None;
        self.save_phrase.clear();
        self.prepared=None;
        let confirmation=if rollback{"ROLLBACK"}else{"FINALIZE"};
        if self.readonly || self.recovery_phrase!=confirmation {
            self.edit_report=format!("Enter {confirmation} exactly; -view forbids recovery writes.");
            return;
        }
        let result=(||->Result<_,String>{
            let (_,_,world,dimension)=self.current_world_context()?;
            if self.sessions.iter().any(|((index,_),session)|
                self.worlds.get(*index).is_some_and(|other|
                    other.root==world.root) && session.has_pending_changes()
            ){
                return Err("World has staged edits; export/discard them before resolving a disk transaction".into());
            }
            let (rx,rz,ext)=parse_region_name(&self.recover_region)?;
            let receipt=if rollback{
                rollback_pending_region(&world,&dimension,rx,rz,ext)
            }else{
                finalize_single_region(&world,&dimension,rx,rz,ext)
            }.map_err(|e|e.to_string())?;
            Ok((world.root,receipt))
        })();
        self.recovery_phrase.clear();
        match result {
            Ok((root,receipt))=>{
                self.clear_cached_world(&root);
                self.edit_report=format!(
                    "{}: {}. Immutable original backup retained: {}. Transaction record: {}",
                    if rollback{"RESTORED ORIGINAL"}else{"FINALIZED WRITE"},
                    receipt.region_path.display(),receipt.backup_path.display(),
                    receipt.marker_path.display());
            },
            Err(e)=>self.edit_report=format!("Transaction resolution refused: {e}"),
        }
    }
    fn boot(args:launch::Arguments)->Self{
        let mut app=App{
            debug:args.debug,log:args.log,verbosity:args.verbosity,
            readonly:args.view,query_coordinates:"0, 64, 0".into(),
            edit_area:"0, 64, 0; 0, 64, 0".into(),
            slice_area:"0, 64, 0; 15, 64, 15".into(),
            edit_target:"minecraft:stone".into(),
            clone_destination:"0, 64, 0".into(),
            fill_seed:"0, 64, 0".into(),
            clipboard_destination:"0, 64, 0".into(),
            vertical_area:"0,-64,0;15,-49,0".into(),
            surface_area:"0,-64,0;15,-49,15".into(),
            surface_zoom:1.0,surface_pitch:35,
            surface_frame:true,surface_axes:true,surface_chunk_grid:true,
            surface_section_guides:false,
            show_textures:true,use_gpu:true,..Default::default()
        };
        for path in &args.worlds {
            match inspect_input(path) {
                Ok(world)=>{
                    app.current=Some(app.worlds.len());
                    app.dimension_index=0;
                    app.block_result.clear();app.slice_result.clear();
                    app.prepared=None;
                    app.edit_report.clear();
                    app.worlds.push(world);
                }
                Err(e)=>app.messages.push(format!("{}: {e}",path.display()))
            }
        }
        if let Some(path)=args.worlds.first(){app.path=path.to_string_lossy().to_string();}
        app
    }

    fn current_key(&self)->Option<(usize,usize)>{
        self.current.map(|index|(index,self.dimension_index))
    }
    fn current_session(&mut self, key:(usize,usize))->Result<&mut Session,String>{
        if !self.sessions.contains_key(&key){
            let world=self.worlds.get(key.0).ok_or("Selected world is no longer open")?.clone();
            let dimension=world.dimensions.get(key.1)
                .ok_or("Select an indexed original world dimension")?.name.clone();
            let session=Session::new(world,dimension).map_err(|e|e.to_string())?;
            self.sessions.insert(key,session);
        }
        self.sessions.get_mut(&key).ok_or_else(||"Missing editor session".into())
    }
    fn prepare_batch(&mut self){
        self.save_preview=None;
        self.prepared=None;
        let Some(key)=self.current_key() else {
            self.edit_report="Open a Java world and select a dimension first.".into();return;
        };
        let outcome=(||->Result<Preview,String>{
            let bounds=parse_bounds(&self.edit_area).map_err(|e|e.to_string())?;
            let target=parse_state(&self.edit_target).map_err(|e|e.to_string())?;
            let filter=if self.edit_filter.trim().is_empty(){None}
                else{Some(parse_state(&self.edit_filter).map_err(|e|e.to_string())?)};
            let editor=self.current_session(key)?;
            editor.prepare(bounds,&target,filter.as_ref()).map_err(|e|e.to_string())
        })();
        match outcome{
            Ok(preview)=>{
                self.edit_report=format!("Read-only preview: {}",preview.summary());
                self.prepared=Some((key.0,key.1,preview));
            }
            Err(error)=>self.edit_report=format!("Edit preview REFUSED: {error}"),
        }
    }
    fn prepare_move(&mut self){
        self.prepared=None;
        self.save_preview=None;
        self.slice_grid=None;
        self.vertical_grid=None;
        let result=(||->Result<Preview,String>{
            let area=parse_bounds(&self.edit_area).map_err(|e|e.to_string())?;
            let dest=parse_xyz(&self.clone_destination).map_err(|e|e.to_string())?;
            let key=self.current_key().ok_or("Select an original Java world")?;
            self.current_session(key)?
                .prepare_move(area,dest).map_err(|e|e.to_string())
        })();
        match result {
            Ok(preview)=>{
                self.edit_report=format!(
                    "Move preview: {}. Source will be filled with air, overlapping destination wins. No original BE NBT is moved.",
                    preview.summary()
                );
                if let Some((wi,di))=self.current_key(){
                    self.prepared=Some((wi,di,preview));
                }
            },
            Err(e)=>self.edit_report=format!("3D Move refused: {e}"),
        }
    }
    fn prepare_clone(&mut self){
        self.save_preview=None;
        self.prepared=None;
        let Some(key)=self.current_key() else {
            self.edit_report="Open a Java world and select a dimension first.".into();return;
        };
        let outcome=(||->Result<Preview,String>{
            let bounds=parse_bounds(&self.edit_area).map_err(|e|e.to_string())?;
            let destination=parse_xyz(&self.clone_destination).map_err(|e|e.to_string())?;
            self.current_session(key)?
                .prepare_copy(bounds,destination).map_err(|e|e.to_string())
        })();
        match outcome {
            Ok(preview)=>{
                self.edit_report=format!("Read-only original block clone preview: {}",preview.summary());
                self.prepared=Some((key.0,key.1,preview));
            },
            Err(e)=>self.edit_report=format!("Block-only clone REFUSED: {e}"),
        }
    }
    fn prepare_fill(&mut self){
        self.save_preview=None;
        self.prepared=None;
        let Some(key)=self.current_key() else {
            self.edit_report="Open an original Java world first.".into();return;
        };
        let outcome=(||->Result<Preview,String>{
            let bounds=parse_bounds(&self.edit_area).map_err(|e|e.to_string())?;
            let seed=parse_xyz(&self.fill_seed).map_err(|e|e.to_string())?;
            let target=parse_state(&self.edit_target).map_err(|e|e.to_string())?;
            self.current_session(key)?
                .prepare_connected_fill(bounds,seed,&target).map_err(|e|e.to_string())
        })();
        match outcome {
            Ok(preview)=>{
                self.edit_report=format!("3D connected-fill preview: {}",preview.summary());
                self.prepared=Some((key.0,key.1,preview));
            },
            Err(e)=>self.edit_report=format!("Connected fill REFUSED: {e}"),
        }
    }
    fn apply_batch(&mut self){
        let displayed=self.surface.is_some();
        self.surface=None;
        self.save_preview=None;
        self.slice_grid=None;
        self.vertical_grid=None;
        if self.readonly {
            self.edit_report="-view disallows editor changes, including the in-memory overlay.".into();
            return;
        }
        let Some((world,dimension,preview))=self.prepared.take() else {
            self.edit_report="Preview the exact selection before applying changes.".into();return;
        };
        if self.current_key()!=Some((world,dimension)){
            self.edit_report="Changed world/dimension since preview; operation refused.".into();
            return;
        }
        let result=self.current_session((world,dimension))
            .and_then(|editor|editor.commit(preview).map_err(|e|e.to_string()));
        let succeeded=result.is_ok();
        self.edit_report=match result{
            Ok(count)=>format!(
                "Staged {count} block changes as ONE Undo. MEMORY ONLY; no world file was saved."
            ),
            Err(error)=>format!("Edit transaction REFUSED before mutation: {error}"),
        };
        if succeeded&&displayed{self.inspect_surface(self.surface_exposed);}
    }
    fn history(&mut self, redo:bool){
        let displayed=self.surface.is_some();
        self.surface=None;
        self.save_preview=None;
        self.slice_grid=None;
        self.vertical_grid=None;
        self.prepared=None;
        let Some(key)=self.current_key() else{return};
        let result=self.current_session(key).and_then(|editor|{
            if redo {editor.redo()} else {editor.undo()}
                .map_err(|e|e.to_string())
        });
        let succeeded=matches!(result,Ok(true));
        self.edit_report=match result{
            Ok(true)=>format!("{} in-memory original block transaction; no disk writes.",
                if redo{"Redid"}else{"Undid"}),
            Ok(false)=>"No matching in-memory history to undo/redo.".into(),
            Err(error)=>format!("Undo/Redo REFUSED: {error}"),
        };
        if succeeded&&displayed{self.inspect_surface(self.surface_exposed);}
    }
}
fn update(app:&mut App,message:Message)->iced::Task<Message>{
    let before=app.render_signature();
    // Source mutations or stale-view navigation revoke all outstanding
    // near-field worker snapshots. Rendering-only camera/texture events do
    // not revoke an otherwise valid in-flight read.
    if matches!(&message,
        Message::ApplyEdit|Message::UndoEdit|Message::RedoEdit|
        Message::DiscardEdits|Message::ImportPatch|Message::Open|
        Message::Select(_)|Message::Dimension(_)|Message::Clear|
        Message::StageBeScalar|Message::UndoBeScalar|Message::RedoBeScalar|
        Message::StageBeBatch|Message::ImportBeJournal|
        Message::ConfirmWorldSave|Message::ConfirmBeSave|
        Message::FinalizeRegion|Message::RollbackRegion|
        Message::SurfaceArea(_)|Message::SliceText(_)|
        Message::VerticalText(_))
    {
        app.invalidate_near();
    }
    let task=update_impl(app,message);
    if before!=app.render_signature(){
        let mut cache=app.render_face_cache.lock()
            .unwrap_or_else(|poison|poison.into_inner());
        *cache=None;
        let mut gpu=app.render_gpu_cache.lock()
            .unwrap_or_else(|poison|poison.into_inner());
        *gpu=None;
    }
    let mut tasks=vec![task];
    if !app.near_busy{
        if let Some(job)=app.near_job.take(){
            app.near_busy=true;
            tasks.push(dispatch_near(job));
        }
    }
    if !app.open_busy{
        if let Some((nonce,path))=app.open_pending.take(){
            app.open_busy=true;
            tasks.push(dispatch_open(nonce,path));
        }
    }
    iced::Task::batch(tasks)
}
fn update_impl(app:&mut App,message:Message)->iced::Task<Message>{
    // A cached far-height patch reflects one original+overlay snapshot.
    // Any mutation must invalidate it before the next camera move.
    if matches!(&message,
        Message::ApplyEdit|Message::UndoEdit|Message::RedoEdit|
        Message::DiscardEdits|Message::ImportPatch|Message::Open|
        Message::Select(_)|Message::Dimension(_)|Message::StageBeScalar|
        Message::UndoBeScalar|Message::RedoBeScalar|
        Message::StageBeBatch|Message::ImportBeJournal|
        Message::ConfirmWorldSave|Message::ConfirmBeSave|
        Message::FinalizeRegion|Message::RollbackRegion|
        Message::Clear)
    {
        app.far=None;
    }
    match message{
        Message::Path(x)=>app.path=x,
        Message::TexturePath(x)=>app.texture_path=x,
        Message::TextureTick=>{
            app.texture_animation_tick=app.texture_animation_tick.wrapping_add(1);
            if let Some(bank)=app.texture_bank.as_ref(){
                if let Some(frame)=bank.advance_animation(app.texture_animation_tick){
                    app.texture_bank=Some(std::sync::Arc::new(frame));
                }
            }
        },
        Message::ToggleTextures=>app.show_textures=!app.show_textures,
        Message::ToggleGpu=>app.use_gpu=!app.use_gpu,
        Message::CycleGpuQuality=>app.gpu_quality=app.gpu_quality.next(),
        Message::UnloadTextures=>{
            app.texture_bank=None;
            app.texture_report="3D texture assets unloaded. Original world unchanged.".into();
        },
        Message::LoadTextures=>{
            match textures::TextureBank::open(std::path::Path::new(app.texture_path.trim())){
                Ok(bank)=>{
                    app.texture_report=format!(
                        "Resource textures loaded read-only: {} PNGs, {} animated sprites, {} model/blockstate JSONs, {} warnings. {}",
                        bank.count(),bank.animation_count(),bank.model_count(),bank.warnings.len(),
                        bank.warnings.first().map(String::as_str).unwrap_or(""));
                    app.texture_animation_tick=0;
                    app.texture_bank=Some(std::sync::Arc::new(bank));
                    app.show_textures=true;
                },
                Err(e)=>app.texture_report=format!("Resource pack refused (previous assets retained): {e}"),
            }
        },
        Message::SurfaceArea(value)=>{app.surface_area=value;app.surface=None;app.far=None;app.surface_cut_top=None;},
        Message::InspectSurface=>app.inspect_surface(false),
        Message::InspectVolume=>app.inspect_surface(true),
        Message::Shift3D(x,y,z)=>app.shift_3d(x,y,z),
        Message::Center3DOnPicked=>app.center_3d(),
        Message::ScenePreset(side)=>app.scene_preset(side),
        Message::StartFarView(radius)=>return app.start_far_view(radius,None),
        Message::ShiftFarView(dx,dz)=>{
            let Some(far)=app.far.as_ref()else{return iced::Task::none()};
            let (radius,(x,z))=(far.radius,far.center);
            let Some(nx)=x.checked_add(dx.saturating_mul(16))else{
                return iced::Task::none();
            };
            let Some(nz)=z.checked_add(dz.saturating_mul(16))else{
                return iced::Task::none();
            };
            return app.start_far_view(radius,Some((nx,nz)));
        },
        Message::FarBatchLoaded(nonce,patches)=>
            return app.finish_far_batch(nonce,patches),
        Message::NearLoaded(nonce,key,result)=>app.near_loaded(nonce,key,result),
        Message::Toggle3DFrame=>app.surface_frame=!app.surface_frame,
        Message::Toggle3DAxes=>app.surface_axes=!app.surface_axes,
        Message::Toggle3DChunkGrid=>app.surface_chunk_grid=!app.surface_chunk_grid,
        Message::Toggle3DSections=>app.surface_section_guides=!app.surface_section_guides,
        Message::Set3DCornerA=>app.set_3d_corner(true),
        Message::Set3DCornerB=>app.set_3d_corner(false),
        Message::Use3DBox=>app.use_3d_box(),
        Message::Clear3DBox=>{
            app.corner_a=None;app.corner_b=None;
            app.surface_report="3D selection box cleared; world and staged data unchanged.".into();
        },
        Message::Cut3DTop(delta)=>app.move_cutaway(delta),
        Message::Reset3DCut=>{
            app.surface_cut_top=None;
            app.inspect_surface(app.surface_exposed);
        },
        Message::PickSurface(x,y,z)=>app.pick_surface(x,y,z),
        Message::PickSurfaceFace(x,y,z,face)=>{
            app.pick_surface(x,y,z);
            if app.selected_surface==Some((x,y,z)){
                app.selected_surface_face=Some(face);
                app.surface_report=format!(
                    "Selected original voxel ({x},{y},{z}) {:?} face; normal {:?}. Edits require preview/Apply.",
                    face,face.normal());
            }
        },
        Message::RotateSurface=>app.surface_camera=(app.surface_camera+1)%4,
        Message::OrbitSurface(deg)=>app.surface_yaw=(app.surface_yaw+deg).rem_euclid(360),
        Message::OrbitPitchSurface(yaw,pitch)=>{
            app.surface_yaw=(app.surface_yaw+yaw).rem_euclid(360);
            app.surface_pitch=(app.surface_pitch+pitch).clamp(15,80);
        },
        Message::HoverSurface(pos)=>app.surface_hover=pos,
        Message::PitchSurface(deg)=>app.surface_pitch=(app.surface_pitch+deg).clamp(15,80),
        Message::ZoomSurface(n)=>app.surface_zoom=(app.surface_zoom+n as f32*0.25).clamp(0.5,3.0),
        Message::PanSurface(dx,dy)=>{
            app.surface_pan_x=(app.surface_pan_x+dx as f32).clamp(-1000.0,1000.0);
            app.surface_pan_y=(app.surface_pan_y+dy as f32).clamp(-1000.0,1000.0);
        },
        Message::ResetSurfaceCamera=>{
            app.surface_zoom=1.0;app.surface_pan_x=0.0;app.surface_pan_y=0.0;
            app.surface_camera=0;app.surface_yaw=0;app.surface_pitch=35;
        },
        Message::ClipboardPath(value)=>app.clipboard_path=value,
        Message::SchematicPath(value)=>app.schematic_path=value,
        Message::SpongePath(value)=>app.sponge_path=value,
        Message::ClipboardDestination(value)=>app.clipboard_destination=value,
        Message::ExportClipboard=>{
            app.save_preview=None;
            app.prepared=None;
            let path=PathBuf::from(app.clipboard_path.trim());
            let result=(||->Result<usize,String>{
                let bounds=parse_bounds(&app.edit_area).map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select an original Java world")?;
                app.current_session(key)?
                    .export_clipboard(bounds,&path).map_err(|e|e.to_string())
            })();
            app.edit_report=match result{
                Ok(n)=>format!("Exported {n} selected block states to a create-new .mcedit2clip file. Block Entities/entities excluded; Minecraft world unchanged."),
                Err(e)=>format!("Block-only clipboard export refused: {e}"),
            };
        },
        Message::ExportSpongeV3=>{
            let path=PathBuf::from(app.sponge_path.trim());
            let result=(||->Result<usize,String>{
                let area=parse_bounds(&app.edit_area).map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select original world")?;
                app.current_session(key)?.export_sponge_v3(area,&path)
                    .map_err(|e|e.to_string())
            })();
            app.edit_report=match result{
                Ok(n)=>format!("Exported {n} named source blocks to Sponge v3; unknown BEs/entities/biomes are not silently discarded."),
                Err(e)=>format!("Sponge v3 export refused: {e}"),
            };
        },
        Message::PreviewSpongeV3=>{
            app.save_preview=None;app.prepared=None;
            let path=PathBuf::from(app.sponge_path.trim());
            let result=(||->Result<Preview,String>{
                let dest=parse_xyz(&app.clipboard_destination).map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select original world")?;
                app.current_session(key)?.prepare_import_sponge_v3(&path,dest)
                    .map_err(|e|e.to_string())
            })();
            match result{
                Ok(preview)=>{
                    app.edit_report=format!("Sponge v3 exact DataVersion safe paste preview: {}",preview.summary());
                    if let Some((wi,di))=app.current_key(){app.prepared=Some((wi,di,preview));}
                },
                Err(e)=>app.edit_report=format!("Sponge v3 import refused: {e}"),
            }
        },
        Message::ExportSpongeV2=>{
            app.save_preview=None;
            app.prepared=None;
            let path=PathBuf::from(app.sponge_path.trim());
            let result=(||->Result<usize,String>{
                let area=parse_bounds(&app.edit_area).map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select an original world")?;
                app.current_session(key)?
                    .export_sponge_v2(area,&path).map_err(|e|e.to_string())
            })();
            app.edit_report=match result{
                Ok(n)=>format!("Exported {n} named block states to new Sponge v2 .schem (no BE/entity/biome NBT). World unchanged."),
                Err(e)=>format!("Sponge v2 export refused: {e}"),
            };
        },
        Message::PreviewSpongeV2=>{
            app.save_preview=None;
            app.prepared=None;
            let path=PathBuf::from(app.sponge_path.trim());
            let result=(||->Result<Preview,String>{
                let target=parse_xyz(&app.clipboard_destination).map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select an original world")?;
                app.current_session(key)?
                    .prepare_import_sponge_v2(&path,target).map_err(|e|e.to_string())
            })();
            match result {
                Ok(preview)=>{
                    app.edit_report=format!("Sponge v2 exact-DataVersion block-only preview: {}",preview.summary());
                    if let Some((wi,di))=app.current_key(){
                        app.prepared=Some((wi,di,preview));
                    }
                },
                Err(e)=>app.edit_report=format!("Sponge v2 import refused: {e}"),
            }
        },
        Message::ExportLegacySchematic=>{
            app.save_preview=None;
            app.prepared=None;
            let path=PathBuf::from(app.schematic_path.trim());
            let result=(||->Result<usize,String>{
                let area=parse_bounds(&app.edit_area).map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select an original Java world")?;
                app.current_session(key)?
                    .export_legacy_schematic(area,&path)
                    .map_err(|e|e.to_string())
            })();
            app.edit_report=match result{
                Ok(n)=>format!("Exported {n} ORIGINAL numeric block states to new old MCEdit .schematic. All BE/entity data deliberately excluded. Source world unchanged."),
                Err(e)=>format!("Legacy schematic export refused: {e}"),
            };
        },
        Message::PreviewLegacySchematic=>{
            app.save_preview=None;
            app.prepared=None;
            let path=PathBuf::from(app.schematic_path.trim());
            let result=(||->Result<Preview,String>{
                let destination=parse_xyz(&app.clipboard_destination)
                    .map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select an original Java world")?;
                app.current_session(key)?
                    .prepare_import_legacy_schematic(&path,destination)
                    .map_err(|e|e.to_string())
            })();
            match result{
                Ok(preview)=>{
                    app.edit_report=format!("Legacy schematic block-only preview: {}",preview.summary());
                    if let Some((wi,di))=app.current_key(){
                        app.prepared=Some((wi,di,preview));
                    }
                },
                Err(e)=>app.edit_report=format!("Schematic import refused: {e}"),
            }
        },
        Message::PreviewTransformedClipboard(operation)=>{
            app.save_preview=None;
            app.prepared=None;
            let path=PathBuf::from(app.clipboard_path.trim());
            let result=(||->Result<Preview,String>{
                let target=parse_xyz(&app.clipboard_destination)
                    .map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select a world and dimension")?;
                app.current_session(key)?
                    .prepare_paste_clipboard_file_transformed(&path,target,operation)
                    .map_err(|e|e.to_string())
            })();
            match result{
                Ok(preview)=>{
                    app.edit_report=format!("Safe {:?} clipboard paste preview: {}",
                        operation,preview.summary());
                    if let Some((wi,di))=app.current_key(){
                        app.prepared=Some((wi,di,preview));
                    }
                },
                Err(e)=>app.edit_report=format!("Clipboard transformation refused: {e}"),
            }
        },
        Message::PreviewPasteClipboard=>{
            app.save_preview=None;
            app.prepared=None;
            let path=PathBuf::from(app.clipboard_path.trim());
            let result=(||->Result<Preview,String>{
                let destination=parse_xyz(&app.clipboard_destination)
                    .map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select an original Java world")?;
                app.current_session(key)?
                    .prepare_paste_clipboard_file(&path,destination)
                    .map_err(|e|e.to_string())
            })();
            match result{
                Ok(preview)=>{
                    app.edit_report=format!("Clipboard block-only paste PREVIEW: {}. Stage via existing Undo-able apply button.",preview.summary());
                    if let Some((wi,di))=app.current_key(){
                        app.prepared=Some((wi,di,preview));
                    }
                },
                Err(e)=>app.edit_report=format!("Clipboard paste refused: {e}"),
            }
        },
        Message::PickSlice(x,y,z)=>app.pick_slice(x,y,z),
        Message::MoveSlice(dx,dy,dz)=>app.pan_slice(dx,dy,dz),
        Message::SavePhrase(value)=>app.save_phrase=value,
        Message::BeSavePhrase(value)=>app.be_save_phrase=value,
        Message::PrepareBeSave=>app.prepare_be_save(),
        Message::ConfirmBeSave=>app.confirm_be_save(),
        Message::RecoverRegion(value)=>app.recover_region=value,
        Message::RecoveryPhrase(value)=>app.recovery_phrase=value,
        Message::PrepareWorldSave=>app.prepare_save(),
        Message::ConfirmWorldSave=>app.confirm_save(),
        Message::FinalizeRegion=>app.recover_region(false),
        Message::RollbackRegion=>app.recover_region(true),
        Message::Open=>{
            app.be_prepared=None;
            app.surface=None;
            app.save_preview=None;
            app.slice_grid=None;app.vertical_grid=None;
            app.open_nonce=app.open_nonce.wrapping_add(1);
            app.open_pending=None;
            if app.path.trim().is_empty(){
                app.messages.push("Enter a Java world folder or level.dat path".into());
            }else{
                let path=PathBuf::from(app.path.trim());
                app.open_pending=Some((app.open_nonce,path));
                app.messages.push("Indexing original Java world in a read-only worker.".into());
            }
        }
        Message::OpenFinished(nonce,result)=>{
            // Release the single indexer slot EVEN if another world selection
            // superseded this read. A queued latest open can then start.
            app.open_busy=false;
            if nonce==app.open_nonce{
                match result{
                    Ok(world)=>{
                        app.current=Some(app.worlds.len());
                        app.dimension_index=0;
                        app.block_result.clear();app.slice_result.clear();
                        app.prepared=None;app.edit_report.clear();
                        app.messages.push(format!(
                            "Indexed {} original Region files and {} chunks.",
                            world.total_regions(),world.total_chunks()
                        ));
                        app.worlds.push(world);
                    }
                    Err(e)=>app.messages.push(format!("Cannot inspect world: {e}")),
                }
            }
        }
        Message::Select(index)=>{
            app.open_nonce=app.open_nonce.wrapping_add(1);
            app.open_pending=None;
            app.be_prepared=None;
            app.surface=None;
            app.save_preview=None;
            app.slice_grid=None;
            app.vertical_grid=None;
            if index<app.worlds.len(){
                app.current=Some(index);
                app.dimension_index=0;
                app.block_result.clear();app.slice_result.clear();
                app.prepared=None;
                app.edit_report.clear();
            }
        }
        Message::Coordinates(value)=>{app.query_coordinates=value;app.be_prepared=None;},
        Message::Dimension(index)=>{
            app.open_nonce=app.open_nonce.wrapping_add(1);
            app.open_pending=None;
            app.be_prepared=None;
            app.surface=None;
            app.save_preview=None;
            app.slice_grid=None;
            app.vertical_grid=None;
            if app.current.and_then(|i|app.worlds.get(i))
                .is_some_and(|w|index<w.dimensions.len()){
                app.dimension_index=index;
                app.block_result.clear();app.slice_result.clear();
                app.prepared=None;
                app.edit_report.clear();
            }
        }
        Message::InspectBlock=>{
            app.block_result=match app.current.and_then(|i|app.worlds.get(i)){
                None=>"Select an indexed original Java world first.".into(),
                Some(world)=>match world.dimensions.get(app.dimension_index){
                    None=>"Selected world has no indexed dimensions.".into(),
                    Some(dimension)=>match parse_xyz(&app.query_coordinates){
                        Err(e)=>format!("Invalid query coordinates: {e}"),
                        Ok((x,y,z))=>{
                            let key=(app.current.unwrap_or_default(),app.dimension_index);
                            let result=if let Some(session)=app.sessions.get(&key){
                                session.visible_block((x,y,z))
                            } else {
                                read_world_block(world,&dimension.name,x,y,z)
                            };
                            match result{
                                Ok(record)=>record.description(),
                                Err(e)=>format!("Original block read refused: {e}"),
                            }
                        }
                    }
                }
            };
        },
        Message::BeScalarField(value)=>{app.be_scalar_field=value;app.be_prepared=None;},
        Message::EntitySnapshotPath(value)=>app.entity_snapshot_path=value,
        Message::InspectEntities=>{
            let outcome=(||->Result<String,String>{
                let p=parse_xyz(&app.query_coordinates).map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select original world")?;
                let report=app.current_session(key)?
                    .inspect_entities(p.0.div_euclid(16),p.2.div_euclid(16))
                    .map_err(|e|e.to_string())?;
                let mut out=format!("Original {} chunk {:?}, {} entities, SHA-256 {}. READ ONLY.\n",
                    report.source,report.chunk,report.entities.len(),report.source_sha256);
                for e in report.entities.iter().take(64){
                    out.push_str(&format!("id={} Pos={:?} fields={} original NBT={} bytes SHA={}\n",
                        e.id.as_deref().unwrap_or("unknown"),e.pos,e.fields,e.bytes,e.sha256));
                }
                Ok(out)
            })();
            app.block_result=outcome.unwrap_or_else(|e|format!("Entity inspection refused: {e}"));
        },
        Message::ExportEntities=>{
            let file=PathBuf::from(app.entity_snapshot_path.trim());
            let outcome=(||->Result<usize,String>{
                let p=parse_xyz(&app.query_coordinates).map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select original world")?;
                app.current_session(key)?.export_original_entities(
                    p.0.div_euclid(16),p.2.div_euclid(16),&file
                ).map_err(|e|e.to_string())
            })();
            app.block_result=match outcome{
                Ok(n)=>format!("Exported {n} bytes of exact original entity snapshot. World unchanged."),
                Err(e)=>format!("Entity snapshot export refused: {e}"),
            };
        },
        Message::VerifyEntities=>{
            let file=PathBuf::from(app.entity_snapshot_path.trim());
            let outcome=app.current_key().ok_or_else(||"Select original world".to_string())
                .and_then(|key|app.current_session(key))
                .and_then(|editor|editor.verify_original_entities(&file)
                    .map_err(|e|e.to_string()));
            app.block_result=match outcome{
                Ok(n)=>format!("Verified {n} original entity NBT bytes against current source."),
                Err(e)=>format!("Entity snapshot verification refused: {e}"),
            };
        },
        Message::BeSnapshotPath(value)=>app.be_snapshot_path=value,
        Message::BeJournalPath(value)=>app.be_journal_path=value,
        Message::BeBatchPath(value)=>app.be_batch_path=value,
        Message::RegionBatchPath(value)=>app.region_batch_path=value,
        Message::PreviewRegionBatch=>{
            let result=(||->Result<_,String>{
                let key=app.current_key().ok_or("Select original world")?;
                let time=u32::try_from(SystemTime::now()
                    .duration_since(UNIX_EPOCH).map_err(|_|"System clock before UNIX epoch")?.as_secs())
                    .map_err(|_|"Region timestamp cannot represent system clock")?;
                app.current_session(key)?.prepare_region_batch_report(time)
                    .map_err(|e|e.to_string())
            })();
            app.edit_report=match result{
                Ok(report)=>format!("Verified {} original Region candidates across this dimension, {} BE scalar proof steps. READ-ONLY: multi-Region save is not atomic or available.",report.regions.len(),report.regions.iter().map(|r|r.be_scalar_steps).sum::<usize>()),
                Err(e)=>format!("Region batch preflight refused: {e}"),
            };
        },
        Message::ExportRegionBatch=>{
            let path=PathBuf::from(app.region_batch_path.trim());
            let result=(||->Result<usize,String>{
                let key=app.current_key().ok_or("Select original world")?;
                let time=u32::try_from(SystemTime::now()
                    .duration_since(UNIX_EPOCH).map_err(|_|"Invalid system time")?.as_secs())
                    .map_err(|_|"Invalid Region timestamp")?;
                app.current_session(key)?.export_region_batch_report(time,&path)
                    .map_err(|e|e.to_string())
            })();
            app.edit_report=match result{
                Ok(n)=>format!("Wrote create-new original-byte dry-run manifest for {n} Region files; no Minecraft files changed."),
                Err(e)=>format!("Region batch export refused: {e}"),
            };
        },
        Message::VerifyRegionBatch=>{
            let path=PathBuf::from(app.region_batch_path.trim());
            let result=app.current_key().ok_or_else(||"Select original world".to_string())
                .and_then(|key|app.current_session(key))
                .and_then(|session|session.verify_region_batch_report(&path)
                    .map_err(|e|e.to_string()));
            app.edit_report=match result{
                Ok(n)=>format!("Verified {n} exact Region candidates against original world bytes and current overlay; no disk writes."),
                Err(e)=>format!("Region manifest verification refused: {e}"),
            };
        },
        Message::PreviewBeBatch=>{
            let file=PathBuf::from(app.be_batch_path.trim());
            let result=app.current_key().ok_or_else(||"Select a world".to_string())
                .and_then(|key|app.current_session(key))
                .and_then(|session|session.preview_be_operation_file(&file)
                    .map_err(|e|e.to_string()));
            app.edit_report=match result{
                Ok((count,chunks))=>format!("Validated all {count} BE scalar edits against source NBT across {chunks} chunks. Nothing staged or written."),
                Err(e)=>format!("BE batch preview refused: {e}"),
            };
        },
        Message::StageBeBatch=>{
            if app.readonly {app.edit_report="-view forbids BE changes".into();}
            else{
                app.be_prepared=None;app.prepared=None;app.save_preview=None;
                let file=PathBuf::from(app.be_batch_path.trim());
                let result=app.current_key().ok_or_else(||"Select a world".to_string())
                    .and_then(|key|app.current_session(key))
                    .and_then(|session|session.stage_be_operation_file(&file)
                        .map_err(|e|e.to_string()));
                app.edit_report=match result{
                    Ok(n)=>format!("Atomically staged all {n} BE scalar operations IN MEMORY. Existing BE Undo available; world disk unchanged."),
                    Err(e)=>format!("Entire BE batch refused: {e}"),
                };
                app.surface=None;app.slice_grid=None;app.vertical_grid=None;
            }
        },
        Message::ExportBeJournal=>{
            let file=PathBuf::from(app.be_journal_path.trim());
            let result=app.current_key().ok_or_else(||"Select an original world".to_string())
                .and_then(|key|app.current_session(key))
                .and_then(|editor|editor.export_be_journal(&file).map_err(|e|e.to_string()));
            app.edit_report=match result{
                Ok(n)=>format!("Exported {n} source-hashed BE scalar operations to detached create-new journal; original world untouched."),
                Err(e)=>format!("BE checkpoint export refused: {e}"),
            };
        },
        Message::ImportBeJournal=>{
            if app.readonly{app.edit_report="-view forbids BE journal import.".into();}
            else{
                app.be_prepared=None;app.prepared=None;app.save_preview=None;
                let file=PathBuf::from(app.be_journal_path.trim());
                let result=app.current_key().ok_or_else(||"Select an original world".to_string())
                    .and_then(|key|app.current_session(key))
                    .and_then(|editor|editor.import_be_journal(&file).map_err(|e|e.to_string()));
                app.edit_report=match result{
                    Ok(n)=>format!("Restored {n} verified BE scalar changes IN MEMORY. Each step separately undoable; Region world save remains blocked."),
                    Err(e)=>format!("BE journal restore refused without modifying session: {e}"),
                };
                app.surface=None;app.slice_grid=None;app.vertical_grid=None;
            }
        },
        Message::PluginManifestPath(value)=>app.plugin_manifest_path=value,
        Message::PreviewPlugin=>{
            app.prepared=None;
            app.save_preview=None;
            let filename=PathBuf::from(app.plugin_manifest_path.trim());
            let result=(||->Result<Preview,String>{
                let key=app.current_key().ok_or("Select an original world/dimension")?;
                app.current_session(key)?.prepare_plugin_file(&filename)
                    .map_err(|e|e.to_string())
            })();
            match result{
                Ok(preview)=>{
                    app.edit_report=format!("Declarative plugin PREVIEW: {}. Stage using existing Undo-able Apply control; no arbitrary executable is run.",preview.summary());
                    if let Some((wi,di))=app.current_key(){
                        app.prepared=Some((wi,di,preview));
                    }
                },
                Err(e)=>app.edit_report=format!("Plugin manifest preflight refused: {e}"),
            }
        },
        Message::ExportOriginalBe=>{
            let path=PathBuf::from(app.be_snapshot_path.trim());
            let result=(||->Result<usize,String>{
                let point=parse_xyz(&app.query_coordinates).map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select original world/dimension")?;
                app.current_session(key)?.export_original_be_snapshot(point,&path)
                    .map_err(|e|e.to_string())
            })();
            app.block_result=match result{
                Ok(size)=>format!("Exported immutable original Block Entity NBT ({} raw bytes) to external snapshot. World untouched.",size),
                Err(e)=>format!("Opaque BE snapshot export refused: {e}"),
            };
        },
        Message::VerifyOriginalBe=>{
            let path=PathBuf::from(app.be_snapshot_path.trim());
            let result=(||->Result<((i32,i32,i32),usize),String>{
                let key=app.current_key().ok_or("Select original world/dimension")?;
                app.current_session(key)?.verify_original_be_snapshot(&path)
                    .map_err(|e|e.to_string())
            })();
            app.block_result=match result{
                Ok((xyz,size))=>format!("Verified {} exact original NBT bytes at XYZ {:?} against the current source chunk; no data imported or modified.",size,xyz),
                Err(e)=>format!("Opaque BE snapshot verification refused: {e}"),
            };
        },
        Message::BeScalarValue(value)=>{app.be_scalar_value=value;app.be_prepared=None;},
        Message::PreviewBeScalar=>{
            app.be_prepared=None;
            let outcome=(||->Result<_,String>{
                let point=parse_xyz(&app.query_coordinates).map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select original world and dimension")?;
                let field=app.be_scalar_field.clone();
                let value=app.be_scalar_value.clone();
                let proposal=app.current_session(key)?.prepare_be_edit(point,&field,&value)
                    .map_err(|e|e.to_string())?;
                Ok((key.0,key.1,proposal))
            })();
            match outcome{
                Ok((wi,di,preview))=>{
                    app.block_result=format!(
                        "Verified BE scalar proposal: {} NBT bytes, SHA-256 {}. STAGE is memory-only; Region saving blocked while BE edits remain.",
                        preview.patched_nbt.len(),
                        preview.sha256()
                    );
                    app.be_prepared=Some((wi,di,preview));
                },
                Err(e)=>app.block_result=format!("BE preview refused: {e}"),
            }
        },
        Message::StageBeScalar=>{
            if app.readonly{app.block_result="-view forbids BE staging.".into();}
            else {
                let outcome=app.be_prepared.take()
                    .ok_or("Preview original BE scalar change first".to_owned())
                    .and_then(|(wi,di,prepared)|{
                        if app.current_key()!=Some((wi,di)){
                            return Err("Changed dimension/world since BE preview".into());
                        }
                        app.current_session((wi,di))?.commit_be_edit(prepared)
                            .map_err(|e|e.to_string())
                    });
                app.block_result=match outcome{
                    Ok(())=>"BE NBT staged with separate Undo; Minecraft world unchanged and disk save BLOCKED.".into(),
                    Err(e)=>format!("BE stage refused: {e}"),
                };
                app.save_preview=None;app.surface=None;app.slice_grid=None;app.vertical_grid=None;
            }
        },
        Message::UndoBeScalar=>{
            app.be_prepared=None;
            app.save_preview=None;
            if app.readonly{app.block_result="-view forbids BE Undo.".into();}
            else{
                let r=app.current_key().ok_or("Select original world".to_owned())
                    .and_then(|k|app.current_session(k))
                    .and_then(|s|s.undo_be_edit().map_err(|e|e.to_string()));
                app.block_result=match r{
                    Ok(true)=>"BE scalar Undo completed in memory.".into(),
                    Ok(false)=>"No BE scalar Undo history.".into(),
                    Err(e)=>format!("BE Undo refused: {e}"),
                };
                app.surface=None;app.slice_grid=None;app.vertical_grid=None;
            }
        },
        Message::RedoBeScalar=>{
            app.be_prepared=None;
            app.save_preview=None;
            if app.readonly{app.block_result="-view forbids BE Redo.".into();}
            else{
                let r=app.current_key().ok_or("Select original world".to_owned())
                    .and_then(|k|app.current_session(k))
                    .and_then(|s|s.redo_be_edit().map_err(|e|e.to_string()));
                app.block_result=match r{
                    Ok(true)=>"BE scalar Redo completed in memory.".into(),
                    Ok(false)=>"No BE scalar Redo history.".into(),
                    Err(e)=>format!("BE Redo refused: {e}"),
                };
                app.surface=None;app.slice_grid=None;app.vertical_grid=None;
            }
        },
        Message::PreviewBeRegion=>{
            app.save_preview=None;
            let result=app.current_key().ok_or_else(||"Select original world".to_string()).and_then(|key|
                app.current_session(key)
            ).and_then(|editor|editor.preview_be_region_candidates(0).map_err(|e|e.to_string()));
            app.edit_report=match result{
                Ok(regions)=>format!("BE-AUTHENTICATED IN-MEMORY Region preview: {} affected Region files; {} total bytes in candidates. Disk saving requires the separate fresh SAVE_BE_NBT transaction and backup confirmation.",regions.len(),
                    regions.iter().map(|r|r.planned_bytes).sum::<usize>()),
                Err(e)=>format!("BE-aware Region preview refused: {e}"),
            };
        },
        Message::InspectStagedBe=>{
            let r=(||->Result<String,String>{
                let p=parse_xyz(&app.query_coordinates).map_err(|e|e.to_string())?;
                let k=app.current_key().ok_or("Select original world")?;
                Ok(app.current_session(k)?.inspect_staged_be(p)
                    .map_err(|e|e.to_string())?.map_or_else(
                        ||"No original/staged BE found.".into(),|be|be.report()))
            })();
            app.block_result=r.unwrap_or_else(|e|format!("BE inspection refused: {e}"));
        },
        Message::InspectBeTree=>{
            let result=(||->Result<String,String>{
                let pos=parse_xyz(&app.query_coordinates).map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select an original world")?;
                let tree=app.current_session(key)?.inspect_original_be_tree(pos)
                    .map_err(|e|e.to_string())?;
                if tree.is_empty(){return Ok("No original BE NBT at this XYZ.".into());}
                let mut message=format!("Original modded BE tree: {} visible nested paths (read only). Safely editable scalar paths marked EDIT; no arrays/list elements are edited.\n",tree.len());
                for item in tree{
                    message.push_str(&format!(
                        "{} kind={} bytes={} children={} [{}]\n",
                        item.path,item.kind,item.payload_bytes,item.immediate_children,
                        if item.safely_editable_scalar{"EDIT"}else{"READ ONLY"}
                    ));
                }
                Ok(message)
            })();
            app.block_result=result.unwrap_or_else(|e|format!("BE tree refused: {e}"));
        },
        Message::InspectOriginalBlockEntity=>{
            let result=(||->Result<String,String>{
                let point=parse_xyz(&app.query_coordinates).map_err(|e|e.to_string())?;
                let key=app.current_key().ok_or("Select an original Java world")?;
                let be=app.current_session(key)?
                    .inspect_original_block_entity(point)
                    .map_err(|e|e.to_string())?;
                Ok(match be{
                    Some(data)=>data.report(),
                    None=>format!("No ORIGINAL Block Entity is recorded at ({},{},{}).",point.0,point.1,point.2),
                })
            })();
            app.block_result=result.unwrap_or_else(|e|format!("Block Entity inspection refused: {e}"));
        },
        Message::VerticalText(value)=>{
            app.vertical_area=value;app.vertical_grid=None;
        },
        Message::InspectVertical=>app.inspect_vertical_view(),
        Message::PickVertical(row,col)=>app.pick_vertical(row,col),
        Message::SliceText(value)=>{
            app.slice_area=value;
            app.slice_grid=None;
        },
        Message::InspectSlice=>app.inspect_slice_view(),
        Message::SelectionText(value)=>{
            app.edit_area=value;app.prepared=None;app.save_preview=None;app.save_phrase.clear();
        }
        Message::TargetText(value)=>{
            app.edit_target=value;app.prepared=None;app.save_preview=None;app.save_phrase.clear();
        }
        Message::FilterText(value)=>{
            app.edit_filter=value;app.prepared=None;app.save_preview=None;app.save_phrase.clear();
        }
        Message::AnalyzeRegionPlan=>{
            let timestamp=SystemTime::now().duration_since(UNIX_EPOCH)
                .ok().and_then(|d|u32::try_from(d.as_secs()).ok());
            app.edit_report=match (app.current_key(),timestamp){
                (None,_)=>"Select an original Java world first.".into(),
                (_,None)=>"System clock outside supported Region timestamp range.".into(),
                (Some(key),Some(timestamp))=>app.current_session(key)
                    .and_then(|editor|editor.prepare_native_region_candidates(timestamp)
                        .map_err(|e|e.to_string()))
                    .map(|plans|format!(
                        "Original Region dry-run: {} Region images, {} changed chunks, {} original bytes -> {} candidate bytes. NO FILES WRITTEN.",
                        plans.len(),
                        plans.iter().map(|p|p.modified_chunks).sum::<usize>(),
                        plans.iter().map(|p|p.original_bytes).sum::<usize>(),
                        plans.iter().map(|p|p.planned_bytes).sum::<usize>(),
                    ))
                    .unwrap_or_else(|e|format!("Region dry-run refused: {e}")),
            };
        },
        Message::AnalyzeNativePatch=>{
            app.edit_report=match app.current_key(){
                None=>"Open a world before analyzing native NBT changes.".into(),
                Some(key)=>app.current_session(key)
                    .and_then(|editor|editor.prepare_native_palette_candidates().map_err(|e|e.to_string()))
                    .map(|chunks|format!("Original-byte in-memory NBT preview: {} chunks and {} blocks eligible; NO Minecraft files saved.",
                        chunks.len(),chunks.iter().map(|v|v.edited_blocks).sum::<usize>()))
                    .unwrap_or_else(|e|format!("Lossless native save-preview REFUSED: {e}")),
            };
        },
        Message::PatchPath(value)=>app.patch_path=value,
        Message::ExportPatch=>{
            let key=app.current_key();
            let path=PathBuf::from(app.patch_path.trim());
            app.edit_report=match key {
                None=>"Select a Java world before exporting a journal.".into(),
                Some(key)=>app.current_session(key)
                    .and_then(|editor|editor.export_patch(&path).map_err(|e|e.to_string()))
                    .map(|n|format!("Exported {n} original-NBT-verified block edits to external journal. Minecraft world NOT saved."))
                    .unwrap_or_else(|e|format!("Journal export refused: {e}")),
            };
        },
        Message::ImportPatch=>{
            app.surface=None;
            app.save_preview=None;
            app.slice_grid=None;
            app.vertical_grid=None;
            app.prepared=None;
            if app.readonly {
                app.edit_report="-view prohibits importing staged edits.".into();
            } else {
                let key=app.current_key();
                let path=PathBuf::from(app.patch_path.trim());
                app.edit_report=match key {
                    None=>"Select a Java world before importing a journal.".into(),
                    Some(key)=>app.current_session(key)
                        .and_then(|editor|editor.import_patch(&path).map_err(|e|e.to_string()))
                        .map(|n|format!("Restored {n} verified block edits as one Undo group. World files still untouched."))
                        .unwrap_or_else(|e|format!("Journal import refused with no staged mutations: {e}")),
                };
            }
        },
        Message::FillSeed(value)=>{
            app.fill_seed=value;app.prepared=None;app.save_preview=None;app.save_phrase.clear();
        }
        Message::PreviewFill=>app.prepare_fill(),
        Message::PreviewBrush(shape)=>{
            app.prepared=None;app.save_preview=None;
            let result=(||->Result<Preview,String>{
                let bounds=parse_bounds(&app.edit_area).map_err(|e|e.to_string())?;
                let state=parse_state(&app.edit_target).map_err(|e|e.to_string())?;
                let filter=if app.edit_filter.trim().is_empty(){None}
                    else{Some(parse_state(&app.edit_filter).map_err(|e|e.to_string())?)};
                let key=app.current_key().ok_or("Select a dimension")?;
                app.current_session(key)?.prepare_brush(bounds,shape,&state,filter.as_ref())
                    .map_err(|e|e.to_string())
            })();
            match result{
                Ok(preview)=>{
                    app.edit_report=format!("{shape:?} brush preview: {}. One guarded Undo group.",preview.summary());
                    if let Some((wi,di))=app.current_key(){app.prepared=Some((wi,di,preview));}
                },
                Err(e)=>app.edit_report=format!("3D brush refused: {e}"),
            }
        },
        Message::CloneDestination(value)=>{
            app.clone_destination=value;app.prepared=None;app.save_preview=None;app.save_phrase.clear();
        }
        Message::PreviewClone=>app.prepare_clone(),
        Message::PreviewMove=>app.prepare_move(),
        Message::PreviewEdit=>app.prepare_batch(),
        Message::ApplyEdit=>app.apply_batch(),
        Message::UndoEdit=>app.history(false),
        Message::RedoEdit=>app.history(true),
        Message::DiscardEdits=>{
            app.be_save_phrase.clear();
            app.be_prepared=None;
            app.surface=None;
            app.save_preview=None;
            app.slice_grid=None;
            app.vertical_grid=None;
            app.prepared=None;
            if let Some(key)=app.current_key(){
                if let Some(editor)=app.sessions.get_mut(&key){
                    editor.discard();
                }
            }
            app.edit_report="Discarded all staged edits for this world/dimension; original disk untouched.".into();
        }
        Message::Clear=>{
            app.open_nonce=app.open_nonce.wrapping_add(1);
            app.open_pending=None;
            app.be_save_phrase.clear();
            app.be_prepared=None;
            app.surface=None;app.far=None;
            app.save_preview=None;
            app.slice_grid=None;
            app.vertical_grid=None;
            app.recovery_phrase.clear();
            app.save_phrase.clear();
            app.path.clear();app.worlds.clear();
            app.sessions.clear();app.prepared=None;app.edit_report.clear();
            app.current=None;app.dimension_index=0;
            app.query_coordinates="0, 64, 0".into();
            app.block_result.clear();app.slice_result.clear();app.messages.clear();
        }
    }
    iced::Task::none()
}
fn view(app:&App)->Element<'_,Message>{
    let controls=row![
        text_input("Java world folder, level.dat, or level.dat_old",&app.path)
            .on_input(Message::Path).on_submit(Message::Open)
            .width(Length::Fill),
        button(if app.open_busy{"Indexing world…"}else{"Inspect world"})
            .on_press(Message::Open),
        button("Close all (discard staged)").on_press(Message::Clear)
    ].spacing(10);
    let mut body=column![
        text("MCEdit2 — Rust + iced").size(26),
        text("Original Java, Anvil, MCRegion, modern and snapshot world inspection"),
        controls,
        text("Edits are staged IN MEMORY. Experimental, manually confirmed single-Region saves require immutable backups and unresolved transactions must be finalized or rolled back.")
    ].spacing(14);
    if app.debug||app.log||app.verbosity>0||app.readonly{
        body=body.push(text(format!(
            "Original launch flags: -debug={} -log={} verbosity={} -view={}",
            app.debug,app.log,app.verbosity,app.readonly)));
    }
    if !app.worlds.is_empty(){
        let mut tabs=row![].spacing(8);
        for (index,world) in app.worlds.iter().enumerate(){
            tabs=tabs.push(button(text(format!(
                "{} ({})",world.level_name.as_deref().unwrap_or("Unnamed"),index+1
            ))).on_press(Message::Select(index)));
        }
        body=body.push(tabs);
    }
    if let Some(world)=app.current.and_then(|i|app.worlds.get(i)){
        body=body.push(text(format!(
            "{} / {}",world.root.display(),compatibility_label(world))));
        body=body.push(text(format!(
            "Name={} | Version={} | DataVersion={} | Region files={} | Chunks={}",
            world.level_name.as_deref().unwrap_or("?"),
            world.version_name.as_deref().unwrap_or("?"),
            world.data_version.map_or_else(||"?".into(),|x|x.to_string()),
            world.total_regions(),world.total_chunks())));
        let mut dimensions=row![].spacing(7);
        for (index,dim) in world.dimensions.iter().enumerate(){
            dimensions=dimensions.push(button(text(format!(
                "{} ({} chunks)",dim.name,dim.chunks
            ))).on_press(Message::Dimension(index)));
        }
        body=body.push(dimensions);
        let lookup=row![
            text_input("X, Y, Z e.g. -32, -64, 16",&app.query_coordinates)
                .on_input(Message::Coordinates)
                .on_submit(Message::InspectBlock)
                .width(Length::Fill),
            button("Inspect exact block (read-only)").on_press(Message::InspectBlock),
            button("Inspect original BE NBT (read-only)").on_press(Message::InspectOriginalBlockEntity)
        ].spacing(12);
        body=body.push(lookup);
        if !app.block_result.is_empty(){
            body=body.push(text(&app.block_result));
        }
        body=body.push(row![
            text_input("Resource pack folder, .zip, or client .jar (read only)",
                &app.texture_path).on_input(Message::TexturePath)
                .on_submit(Message::LoadTextures).width(Length::Fill),
            button("Load textures").on_press(Message::LoadTextures),
            button("Unload").on_press(Message::UnloadTextures),
            button(if app.show_textures{"Palette only"}else{"Show textures"})
                .on_press(Message::ToggleTextures),
            button(if app.use_gpu{"Use Canvas fallback"}else{"Use wgpu renderer"})
                .on_press(Message::ToggleGpu),
            button(text(format!("GPU detail: {}",app.gpu_quality.label())))
                .on_press(Message::CycleGpuQuality),
        ].spacing(7));
        if !app.texture_report.is_empty(){body=body.push(text(&app.texture_report));}
        body=body.push(row![
            text_input("Read-only 3D XYZ window (max 64x64x48; edit limit stays 4096)",
                &app.surface_area).on_input(Message::SurfaceArea).width(Length::Fill),
            button("Top-surface 3D").on_press(Message::InspectSurface),
            button("Exposed multi-layer 3D").on_press(Message::InspectVolume)
        ].spacing(8));
        body=body.push(row![
            button("16×16×32 inspect").on_press(Message::ScenePreset(16)),
            button("32×32×48 inspect").on_press(Message::ScenePreset(32)),
            button("64×64×48 inspect").on_press(Message::ScenePreset(64)),
            text("Exact near-field preview; edit limit remains 4096.")
        ].spacing(8));
        body=body.push(row![
            text("Progressive distant terrain:"),
            button("16 chunks").on_press(Message::StartFarView(16)),
            button("32 chunks").on_press(Message::StartFarView(32)),
            button("64 chunks").on_press(Message::StartFarView(64)),
            button("128 chunks").on_press(Message::StartFarView(128)),
            button("256 chunks").on_press(Message::StartFarView(256)),
            button("512 chunks").on_press(Message::StartFarView(512)),
            button("1024 chunks").on_press(Message::StartFarView(1024)),
        ].spacing(6));
        body=body.push(row![
            text("Very sparse panorama (source chunks only):"),
            button("1536 chunks").on_press(Message::StartFarView(1536)),
            button("2047 chunks").on_press(Message::StartFarView(2047)),
            text("Maximum span 65,520 blocks across; only verified columns are shown."),
        ].spacing(6));
        if let Some(far)=app.far.as_ref(){
            if app.current_key()==Some(far.key){
                body=body.push(row![
                    text(format!("Far preview {}/{} tiles, errors {}. LOD tiles are non-editable.",
                        far.complete,far.total,far.errors)),
                    button("Far X -16 chunks").on_press(Message::ShiftFarView(-16,0)),
                    button("Far X +16 chunks").on_press(Message::ShiftFarView(16,0)),
                    button("Far Z -16 chunks").on_press(Message::ShiftFarView(0,-16)),
                    button("Far Z +16 chunks").on_press(Message::ShiftFarView(0,16)),
                ].spacing(6));
            }
        }
        body=body.push(row![
            button("X -16").on_press(Message::Shift3D(-16,0,0)),
            button("X +16").on_press(Message::Shift3D(16,0,0)),
            button("Z -16").on_press(Message::Shift3D(0,0,-16)),
            button("Z +16").on_press(Message::Shift3D(0,0,16)),
            button("Y -16").on_press(Message::Shift3D(0,-16,0)),
            button("Y +16").on_press(Message::Shift3D(0,16,0)),
            button("Center on XYZ").on_press(Message::Center3DOnPicked),
        ].spacing(5));
        if let Some((wi,di,view))=&app.surface{
            if app.current_key()==Some((*wi,*di)){
                body=body.push(row![
                    button(text(format!("Rotate 90° ({})",app.surface_camera)))
                        .on_press(Message::RotateSurface),
                    button("Zoom +").on_press(Message::ZoomSurface(1)),
                    button("Zoom -").on_press(Message::ZoomSurface(-1)),
                    button("Pan ←").on_press(Message::PanSurface(-40,0)),
                    button("Pan →").on_press(Message::PanSurface(40,0)),
                    button("Pan ↑").on_press(Message::PanSurface(0,-40)),
                    button("Pan ↓").on_press(Message::PanSurface(0,40)),
                    button("Reset camera").on_press(Message::ResetSurfaceCamera),
                ].spacing(5));
                let stats=view.statistics();
                body=body.push(text(format!(
                    "Scene: {} visible voxels / {} exposed source faces / {} inspected original chunk slots; BE={}, staged={}, liquid={}, glass={}, foliage={}, missing columns={}",
                    stats.voxels,stats.exposed_faces,stats.original_chunks,
                    stats.original_be,stats.staged,stats.liquids,
                    stats.transparent,stats.foliage,stats.missing_columns
                )));
                let unknown_tiles=view.source_tiles.iter().filter(|t|!t.source_present).count();
                let partial_tiles=view.source_tiles.iter().filter(|t|
                    t.source_present&&t.unknown_columns>0).count();
                body=body.push(text(format!(
                    "Chunk tiles: {} present / {} absent / {} partially unknown. Green=known, amber=unknown columns or Sections, red=missing original chunk; display only.",
                    view.source_tiles.len()-unknown_tiles,unknown_tiles,partial_tiles
                )));
                body=body.push(row![
                    button(if app.surface_frame{"Hide 3D bounds"}else{"Show 3D bounds"})
                        .on_press(Message::Toggle3DFrame),
                    button(if app.surface_axes{"Hide XYZ axes"}else{"Show XYZ axes"})
                        .on_press(Message::Toggle3DAxes),
                    button(if app.surface_chunk_grid{"Hide chunk grid"}else{"Show chunk grid"})
                        .on_press(Message::Toggle3DChunkGrid),
                    button(if app.surface_section_guides{"Hide section Y"}else{"Show section Y"})
                        .on_press(Message::Toggle3DSections),
                    text("Axes: X red / Y green / Z blue. Outlined box is view clipping, not world geometry.")
                ].spacing(6));
                body=body.push(row![
                    button("Corner A from picked").on_press(Message::Set3DCornerA),
                    button("Corner B from picked").on_press(Message::Set3DCornerB),
                    button("Use 3D box as edit selection").on_press(Message::Use3DBox),
                    button("Clear 3D box").on_press(Message::Clear3DBox),
                    text(format!("A={:?} / B={:?}",app.corner_a,app.corner_b)),
                ].spacing(5));
                if app.surface_exposed{
                    body=body.push(row![
                        text(format!("3D cutaway top: {} (view only)",
                            app.surface_cut_top.map_or_else(||"full scene".into(),|y|y.to_string()))),
                        button("Cut Y -8").on_press(Message::Cut3DTop(-8)),
                        button("Cut Y -1").on_press(Message::Cut3DTop(-1)),
                        button("Cut Y +1").on_press(Message::Cut3DTop(1)),
                        button("Cut Y +8").on_press(Message::Cut3DTop(8)),
                        button("Restore full height").on_press(Message::Reset3DCut)
                    ].spacing(5));
                }
                body=body.push(text(format!(
                    "3D renderer: {} / sampler {}. Source exposed faces: {}. GPU/picking cap {} visible faces ({} triangles); original NBT unchanged.",
                    if app.use_gpu{"wgpu atlas triangles + Canvas interaction"}else{"Canvas fallback"},
                    app.gpu_quality.label(),stats.exposed_faces,
                    isometric::MAX_RENDER_FACES,isometric::MAX_RENDER_FACES*2
                )));
                if let Some(bank)=&app.texture_bank{
                    let atlas=bank.atlas_size().map_or_else(||"none".to_string(),
                        |(w,h)|format!("{w}x{h} RGBA"));
                    body=body.push(text(format!(
                        "Texture atlas: {} | {} namespaced PNG tiles | {} animated sprites | {} model/blockstate JSONs | {} warnings; CPU/GPU in sync.",
                        atlas,bank.count(),bank.animation_count(),bank.model_count(),bank.warnings.len())));
                    if let Some((x,y,z))=app.selected_surface{
                        if let Some(v)=view.voxels.iter().find(|v|(v.x,v.y,v.z)==(x,y,z)){
                            body=body.push(text(format!(
                                "Selected original state {}: {} resource-model cuboids (if supported); no edits applied.",
                                v.state.name,bank.geometry(&v.state).map_or_else(
                                    ||"fallback".to_string(),|boxes|boxes.len().to_string()))));
                        }
                    }
                }
                body=body.push(text("3D controls: right-drag orbit, middle-drag pan, mouse wheel zoom, left-click select. Mouse hover reads original voxel data only."));
                if let Some((hx,hy,hz))=app.surface_hover{
                    if let Some(voxel)=view.voxels.iter().find(|v|(v.x,v.y,v.z)==(hx,hy,hz)){
                        body=body.push(text(format!(
                            "Hover XYZ ({hx}, {hy}, {hz}) / chunk ({},{}) / section Y {} — {} | BE={} | staged={}",
                            hx.div_euclid(16),hz.div_euclid(16),hy.div_euclid(16),
                            voxel.state.name,voxel.original_block_entity,voxel.staged
                        )));
                    }
                }
                body=body.push(row![
                    button("Orbit -15°").on_press(Message::OrbitSurface(-15)),
                    button("Orbit +15°").on_press(Message::OrbitSurface(15)),
                    button("Look lower").on_press(Message::PitchSurface(-10)),
                    button("Look higher").on_press(Message::PitchSurface(10)),
                    text(format!("yaw {}° / pitch {}° / zoom {:.2}x",
                        (app.surface_camera as i32*90+app.surface_yaw).rem_euclid(360),
                        app.surface_pitch,app.surface_zoom)),
                ].spacing(5));
                let surface_canvas=IsometricSurface{
                    surface:std::sync::Arc::clone(view),camera_quarters:app.surface_camera,
                    texture_bank:app.texture_bank.clone(),show_textures:app.show_textures,
                    yaw_degrees:app.surface_yaw,pitch_degrees:app.surface_pitch,
                    zoom:app.surface_zoom,pan_x:app.surface_pan_x,
                    pan_y:app.surface_pan_y,selected:app.selected_surface,
                    hovered:app.surface_hover,
                    selection_box:app.selection_box(),
                    show_frame:app.surface_frame,show_axes:app.surface_axes,
                    show_chunk_grid:app.surface_chunk_grid,
                    show_section_guides:app.surface_section_guides,
                    gpu_faces:app.use_gpu,
                    gpu_quality:app.gpu_quality,
                    frame_cache:std::sync::Arc::clone(&app.render_face_cache),
                    gpu_frame_cache:std::sync::Arc::clone(&app.render_gpu_cache),
                };
                let stage:Element<'_,Message>=if app.use_gpu{
                    iced::widget::stack![
                        iced::widget::shader(GpuScene(surface_canvas.clone()))
                            .width(Length::Fill).height(Length::Fixed(440.0)),
                        canvas(surface_canvas).width(Length::Fill)
                            .height(Length::Fixed(440.0)),
                    ].into()
                }else{
                    canvas(surface_canvas).width(Length::Fill)
                        .height(Length::Fixed(440.0)).into()
                };
                body=body.push(stage);
            }
        }
        if !app.surface_report.is_empty(){
            body=body.push(text(&app.surface_report));
        }
        body=body.push(row![
            text_input("Existing Block Entity scalar field (not id/x/y/z)",&app.be_scalar_field)
                .on_input(Message::BeScalarField).width(Length::Fill),
            text_input("Exact replacement scalar value",&app.be_scalar_value)
                .on_input(Message::BeScalarValue).width(Length::Fill),
            button("Preview BE scalar").on_press(Message::PreviewBeScalar),
            button("Stage BE in memory").on_press(Message::StageBeScalar),
        ].spacing(8));
        body=body.push(row![
            button("Undo BE").on_press(Message::UndoBeScalar),
            button("Redo BE").on_press(Message::RedoBeScalar),
            button("Inspect staged BE").on_press(Message::InspectStagedBe),
            button("Inspect original nested NBT tree").on_press(Message::InspectBeTree),
            button("Verify BE Region (memory only)").on_press(Message::PreviewBeRegion),
        ].spacing(8));
        body=body.push(text("Experimental BE-aware single-Region SAVE — EXACT existing scalar tags only; closed world, backup, pending recovery REQUIRED."));
        body=body.push(row![
            button("Preview BE save (single Region)").on_press(Message::PrepareBeSave),
            text_input("Type SAVE_BE_NBT to write",&app.be_save_phrase)
                .on_input(Message::BeSavePhrase).width(Length::Fill),
            button("Confirm BE Region save").on_press(Message::ConfirmBeSave),
        ].spacing(8));
        body=body.push(row![
            text_input("Declarative BE scalar .mcedit2beops.json",&app.be_batch_path)
                .on_input(Message::BeBatchPath).width(Length::Fill),
            button("Preview whole BE batch").on_press(Message::PreviewBeBatch),
            button("Stage all BE edits").on_press(Message::StageBeBatch),
        ].spacing(8));
        body=body.push(row![
            text_input("Outside-world BE edit .mcedit2bejournal.json",&app.be_journal_path)
                .on_input(Message::BeJournalPath).width(Length::Fill),
            button("Checkpoint staged BE edits").on_press(Message::ExportBeJournal),
            button("Restore BE edits (memory)").on_press(Message::ImportBeJournal),
        ].spacing(8));
        body=body.push(row![
            text_input("External .mcedit2entities original source snapshot",&app.entity_snapshot_path)
                .on_input(Message::EntitySnapshotPath).width(Length::Fill),
            button("Inspect original entities").on_press(Message::InspectEntities),
            button("Export entity NBT").on_press(Message::ExportEntities),
            button("Verify entity source").on_press(Message::VerifyEntities),
        ].spacing(8));
        body=body.push(row![
            text_input("Outside-world .mcedit2be original NBT snapshot",&app.be_snapshot_path)
                .on_input(Message::BeSnapshotPath).width(Length::Fill),
            button("Export exact original BE NBT").on_press(Message::ExportOriginalBe),
            button("Verify BE source unchanged").on_press(Message::VerifyOriginalBe),
        ].spacing(8));
        body=body.push(row![
            text_input("2D X/Z map: X1,Y,Z1; X2,Y,Z2",&app.slice_area)
                .on_input(Message::SliceText).width(Length::Fill),
            button("Inspect 2D slice").on_press(Message::InspectSlice)
        ].spacing(8));
        body=body.push(row![
            button("← 16").on_press(Message::MoveSlice(-16,0,0)),
            button("16 →").on_press(Message::MoveSlice(16,0,0)),
            button("↑ Z").on_press(Message::MoveSlice(0,0,-16)),
            button("↓ Z").on_press(Message::MoveSlice(0,0,16)),
            button("Y + 1").on_press(Message::MoveSlice(0,1,0)),
            button("Y - 1").on_press(Message::MoveSlice(0,-1,0)),
        ].spacing(5));
        if let Some((world_index,dimension_index,slice))=app.slice_grid.as_ref() {
            if app.current_key()==Some((*world_index,*dimension_index)) {
                body=body.push(text(
                    "Original-state horizontal map: click colored source blocks to inspect; gray=air, purple=missing chunk, amber diagonal=missing Section, gold=BE."
                ));
                body=body.push(canvas(Map2D{
                    plane:Plane::Horizontal(slice),
                    selected:parse_xyz(&app.query_coordinates).ok(),
                    texture_bank:app.texture_bank.as_deref(),
                    show_textures:app.show_textures
                }).width(Length::Fill).height(Length::Fixed(420.0)));
            }
        }
        if !app.slice_result.is_empty(){
            body=body.push(text(&app.slice_result).font(iced::Font::MONOSPACE));
        }
        body=body.push(row![
            text_input("Vertical X/Y or Z/Y plane: X1,Y1,Z1; X2,Y2,Z2",
                &app.vertical_area)
                .on_input(Message::VerticalText).width(Length::Fill),
            button("Inspect vertical plane").on_press(Message::InspectVertical)
        ].spacing(8));
        if let Some((wi,di,plane))=app.vertical_grid.as_ref(){
            if app.current_key()==Some((*wi,*di)){
                body=body.push(text(
                    "Original vertical plane (highest Y at top): click source blocks; gold=BE, amber=missing Section, purple=missing chunk."
                ));
                body=body.push(canvas(Map2D{
                    plane:Plane::Vertical(plane),
                    selected:parse_xyz(&app.query_coordinates).ok(),
                    texture_bank:app.texture_bank.as_deref(),
                    show_textures:app.show_textures
                }).width(Length::Fill).height(Length::Fixed(420.0)));
            }
        }
        if !app.vertical_report.is_empty(){
            body=body.push(text(&app.vertical_report).font(iced::Font::MONOSPACE));
        }
        let editing_inputs=column![
            text("3D block edit overlay — preview, stage and Undo/Redo (NOT SAVED)").size(19),
            text_input("X1,Y1,Z1; X2,Y2,Z2", &app.edit_area)
                .on_input(Message::SelectionText),
            row![
                text_input("Target minecraft:stone or numeric:5[data=2]", &app.edit_target)
                    .on_input(Message::TargetText).width(Length::Fill),
                text_input("Optional original state filter (blank means all)", &app.edit_filter)
                    .on_input(Message::FilterText).width(Length::Fill)
            ].spacing(8),
        ].spacing(8);
        body=body.push(editing_inputs);
        body=body.push(row![
            text_input("Clone target minimum X,Y,Z", &app.clone_destination)
                .on_input(Message::CloneDestination).width(Length::Fill),
            button("Preview block-only clone").on_press(Message::PreviewClone),
            button("Preview block-only MOVE").on_press(Message::PreviewMove)
        ].spacing(8));
        body=body.push(row![
            text_input("Flood-fill seed X,Y,Z inside selection", &app.fill_seed)
                .on_input(Message::FillSeed).width(Length::Fill),
            button("Preview connected 3D fill").on_press(Message::PreviewFill)
        ].spacing(8));
        body=body.push(row![
            button("Hollow box").on_press(Message::PreviewBrush(BrushShape::HollowBox)),
            button("Ellipsoid").on_press(Message::PreviewBrush(BrushShape::Ellipsoid)),
            button("Ellipsoid shell").on_press(Message::PreviewBrush(BrushShape::EllipsoidShell)),
            button("Cylinder Y").on_press(Message::PreviewBrush(BrushShape::CylinderY)),
            button("Cylinder shell Y").on_press(Message::PreviewBrush(BrushShape::CylinderShellY)),
        ].spacing(6));
        let apply=if app.prepared.is_some() && !app.readonly {
            button("Stage approved preview (memory)").on_press(Message::ApplyEdit)
        } else {button("Stage approved preview (memory)")};
        body=body.push(row![
            button("Preview full 3D edit").on_press(Message::PreviewEdit),
            apply,
            button("Undo").on_press(Message::UndoEdit),
            button("Redo").on_press(Message::RedoEdit),
            button("Discard edits").on_press(Message::DiscardEdits)
        ].spacing(8));
        body=body.push(row![
            button("Analyze lossless native NBT preview").on_press(Message::AnalyzeNativePatch),
            button("Analyze append-only Region save plan").on_press(Message::AnalyzeRegionPlan)
        ].spacing(8));
        body=body.push(text("Block-only clipboard (same original world/dimension; will not copy machines, BE NBT or entities)"));
        body=body.push(row![
            text_input("External .mcedit2clip file",&app.clipboard_path)
                .on_input(Message::ClipboardPath).width(Length::Fill),
            button("Copy selection to clipboard file").on_press(Message::ExportClipboard)
        ].spacing(8));
        body=body.push(row![
            text_input("Paste destination X,Y,Z",&app.clipboard_destination)
                .on_input(Message::ClipboardDestination).width(Length::Fill),
            button("Preview clipboard paste").on_press(Message::PreviewPasteClipboard)
        ].spacing(8));
        body=body.push(row![
            button("Rotate 90°").on_press(Message::PreviewTransformedClipboard(HorizontalTransform::Rotate90)),
            button("Rotate 180°").on_press(Message::PreviewTransformedClipboard(HorizontalTransform::Rotate180)),
            button("Rotate 270°").on_press(Message::PreviewTransformedClipboard(HorizontalTransform::Rotate270)),
            button("Mirror X").on_press(Message::PreviewTransformedClipboard(HorizontalTransform::MirrorX)),
            button("Mirror Z").on_press(Message::PreviewTransformedClipboard(HorizontalTransform::MirrorZ)),
        ].spacing(6));
        body=body.push(row![
            text_input("Original MCEdit .schematic path",&app.schematic_path)
                .on_input(Message::SchematicPath).width(Length::Fill),
            button("Preview numeric schematic import").on_press(Message::PreviewLegacySchematic),
            button("Export numeric schematic").on_press(Message::ExportLegacySchematic)
        ].spacing(8));
        body=body.push(row![
            text_input("Modern Sponge v2 .schem file",&app.sponge_path)
                .on_input(Message::SpongePath).width(Length::Fill),
            button("Export selection to Sponge v2").on_press(Message::ExportSpongeV2),
            button("Preview Sponge v2 paste").on_press(Message::PreviewSpongeV2),
            button("Export Sponge v3").on_press(Message::ExportSpongeV3),
            button("Preview Sponge v3 paste").on_press(Message::PreviewSpongeV3),
        ].spacing(8));
        body=body.push(row![
            text_input("External .mcedit2regionplan.json",&app.region_batch_path)
                .on_input(Message::RegionBatchPath).width(Length::Fill),
            button("Preflight all Regions").on_press(Message::PreviewRegionBatch),
            button("Export Region plan").on_press(Message::ExportRegionBatch),
            button("Verify plan").on_press(Message::VerifyRegionBatch),
        ].spacing(8));
        body=body.push(row![
            text_input("Declarative .mcedit2plugin.json file",&app.plugin_manifest_path)
                .on_input(Message::PluginManifestPath).width(Length::Fill),
            button("Preview safe plugin replacement").on_press(Message::PreviewPlugin),
        ].spacing(8));
        body=body.push(row![
            text_input("External .mcedit2patch path (outside the world)",&app.patch_path)
                .on_input(Message::PatchPath).width(Length::Fill),
            button("Export edit journal").on_press(Message::ExportPatch),
            if !app.readonly {
                button("Import verified journal").on_press(Message::ImportPatch)
            } else {
                button("Import verified journal")
            }
        ].spacing(8));
        if !app.readonly {
            body=body.push(text("EXPERIMENTAL SINGLE-REGION DISK SAVE — make a separate world copy first. Minecraft must be closed.").size(16));
            body=body.push(button("Prepare one-Region disk-save preview")
                .on_press(Message::PrepareWorldSave));
            if let Some(intent)=app.save_preview.as_ref() {
                body=body.push(text(format!(
                    "Source: {} | edited chunks: {} | {} -> {} bytes | original SHA-256 {}",
                    intent.candidate.path.display(),intent.candidate.modified_chunks,
                    intent.candidate.original_bytes,intent.candidate.planned_bytes,
                    intent.candidate.original_region_sha256,
                )));
                body=body.push(row![
                    text_input("Type SAVE to write a verified single Region",&app.save_phrase)
                        .on_input(Message::SavePhrase).width(Length::Fill),
                    if app.save_phrase=="SAVE" {
                        button("Replace Region and keep backup").on_press(Message::ConfirmWorldSave)
                    }else{
                        button("Replace Region and keep backup")
                    }
                ].spacing(8));
            }
            body=body.push(text("Transaction recovery: for a pending Region, enter its exact filename (e.g. r.-1.0.mca) and type FINALIZE or ROLLBACK. Backups are retained."));
            body=body.push(row![
                text_input("Pending r.X.Z.mca or r.X.Z.mcr",&app.recover_region)
                    .on_input(Message::RecoverRegion).width(Length::Fill),
                text_input("FINALIZE or ROLLBACK",&app.recovery_phrase)
                    .on_input(Message::RecoveryPhrase).width(Length::Fill),
            ].spacing(8));
            body=body.push(row![
                if app.recovery_phrase=="FINALIZE" {
                    button("Finalize verified Region").on_press(Message::FinalizeRegion)
                }else{button("Finalize verified Region")},
                if app.recovery_phrase=="ROLLBACK" {
                    button("Restore original Region").on_press(Message::RollbackRegion)
                }else{button("Restore original Region")}
            ].spacing(8));
        }
        let stats=app.current_key().and_then(|key|app.sessions.get(&key));
        if let Some(session)=stats{
            body=body.push(text(format!(
                "Staged cells: {} | Undo groups: {} | Redo groups: {} | NEVER WRITTEN TO DISK",
                session.pending_cells(),session.undo_depth(),session.redo_depth()
            )));
        }
        if !app.edit_report.is_empty(){body=body.push(text(&app.edit_report));}
        for warning in &world.warnings{
            body=body.push(text(format!("Warning: {warning}")));
        }
    }
    for message in &app.messages{body=body.push(text(message));}
    container(scrollable(body).height(Length::Fill))
        .width(Length::Fill).height(Length::Fill).padding(20).into()
}
fn subscription(app:&App)->iced::Subscription<Message>{
    if app.show_textures && app.surface.is_some()
        &&app.texture_bank.as_ref().is_some_and(|bank|bank.has_animation()){
        iced::time::every(iced::time::Duration::from_millis(50))
            .map(|_|Message::TextureTick)
    }else{
        iced::Subscription::none()
    }
}
fn main()->iced::Result{
    let args=match launch::parse_process(){
        Ok(options)=>options,
        Err(e)=>{eprintln!("{e}\n{}",launch::HELP);std::process::exit(2)}
    };
    if args.help{print!("{}",launch::HELP);return Ok(());}
    if let Err(e)=launch::validate(&args){
        eprintln!("{e}");std::process::exit(2);
    }
    if args.reset_prefs {
        if let Err(e)=launch::reset_rust_prefs(){
            eprintln!("Cannot reset Rust settings: {e}");
            std::process::exit(1);
        }
    }
    iced::application(move||App::boot(args.clone()),update,view)
        .subscription(subscription)
        .title("MCEdit2 — Rust + iced").theme(Theme::Dark).run()
}

#[cfg(test)]
mod tests{
    use super::{parse_region_name,shifted_slice,slice_cell_label,
        parse_scene_window,shifted_scene};
    use mcedit2_world::editing::{Bounds,SliceCell};
    use mcedit2_world::sections::State;
    #[test]
    fn superseded_indexer_and_near_read_cannot_publish_stale_results(){
        let mut app=super::App::default();
        app.open_nonce=8;
        app.open_busy=true;
        let before=app.messages.len();
        let _=super::update_impl(&mut app,super::Message::OpenFinished(
            7,Err("a stale bad world".into())));
        assert!(!app.open_busy);
        assert_eq!(app.messages.len(),before);
        app.near_nonce=12;
        app.near_busy=true;
        app.near_loaded(11,(0,0),super::NearResult::WorkerStopped);
        assert!(!app.near_busy);
        assert!(app.surface_report.is_empty());
        assert!(app.slice_grid.is_none());
    }
    #[test]
    fn rapid_open_requests_keep_only_latest_source_path(){
        let mut app=super::App::default();
        app.path="original-A".into();
        let _=super::update_impl(&mut app,super::Message::Open);
        let old=app.open_nonce;
        app.path="original-B".into();
        let _=super::update_impl(&mut app,super::Message::Open);
        assert_eq!(app.open_pending.as_ref().unwrap().1,
            std::path::PathBuf::from("original-B"));
        assert_ne!(app.open_nonce,old);
        let _=super::update_impl(&mut app,super::Message::Clear);
        assert!(app.open_pending.is_none());
    }
    #[test]
    fn far_tile_cache_is_bounded_and_lod_sensitive(){
        use super::{FarProgress,far_cache_key,FAR_CACHE_PATCHES,FAR_CACHE_VOXELS};
        use mcedit2_world::far_view::{FarChunkPatch,FarChunkRequest};
        use mcedit2_world::surface::SourceChunkTile;
        let sample=FarChunkRequest{cx:12,cz:-8,span:8};
        let tile=SourceChunkTile{
            chunk_x:12,chunk_z:-8,source_present:false,
            known_columns:0,unknown_columns:4,exposed_voxels:0,
        };
        let patch=std::sync::Arc::new(FarChunkPatch{tile,voxels:vec![]});
        assert_ne!(far_cache_key(sample),far_cache_key(FarChunkRequest{
            span:16,..sample
        }));
        assert!(FAR_CACHE_PATCHES<=4096);
        assert!(FAR_CACHE_VOXELS<=90_000);
        assert_eq!(patch.voxels.len(),0);
    }
    #[test]
    fn render_signature_ignores_animation_and_hover_but_tracks_camera_changes(){
        let mut app=super::App::default();
        let original=app.render_signature();
        app.surface_hover=Some((4,64,6));
        app.texture_animation_tick=19;
        assert!(original==app.render_signature());
        app.surface_yaw=45;
        assert!(original!=app.render_signature());
    }
    #[test]
    fn viewport_3d_navigation_is_independent_of_block_edit_budget(){
        let a=parse_scene_window("0,-64,0;15,-33,15").unwrap();
        assert_eq!(a.volume().unwrap(),8192);
        assert_eq!(shifted_scene("0,-64,0;15,-33,15",-16,0,-16).unwrap(),
            "-16,-64,-16;-1,-33,-1");
        assert_eq!(parse_scene_window("0,-64,0;63,-17,63").unwrap()
            .volume().unwrap(),196_608);
        assert!(parse_scene_window("0,-64,0;64,-33,15").is_err());
        assert!(parse_scene_window("0,-64,0;15,-16,15").is_err());
        assert!(shifted_scene("29999980,0,0;29999995,15,15",16,0,0).is_err());
        assert_eq!(shifted_scene("-32,-64,-32;-1,-17,-1",16,0,16).unwrap(),
            "-16,-64,-16;15,-17,15");
    }
    #[test]
    fn interactive_slice_shift_and_cell_symbols_reject_overflow(){
        let bounds=Bounds::new((-16,-64,0),(-1,-64,15)).unwrap();
        assert_eq!(
            shifted_slice(bounds,16,1,-16).unwrap(),
            "0,-63,-16;15,-63,-1"
        );
        let edge=Bounds::new((29_999_999,320,0),(29_999_999,320,0)).unwrap();
        assert!(shifted_slice(edge,16,0,0).is_err());
        assert_eq!(slice_cell_label(&SliceCell::MissingChunk),"·");
        assert_eq!(slice_cell_label(&SliceCell::MissingSection),"?");
        assert_eq!(slice_cell_label(&SliceCell::Present {
            state:State{name:"minecraft:oak_log".into(),properties:vec![]},
            original_block_entity:false,
        }),"O");
        assert_eq!(slice_cell_label(&SliceCell::Present {
            state:State{name:"mod:machine".into(),properties:vec![]},
            original_block_entity:true,
        }),"@");
    }
    #[test]
    fn original_region_name_parser_accepts_signed_coords_and_refuses_extra_paths(){
        assert_eq!(parse_region_name("r.-1.2.mca").unwrap(),(-1,2,"mca"));
        assert_eq!(parse_region_name("r.0.-9.mcr").unwrap(),(0,-9,"mcr"));
        for value in ["../r.0.0.mca","r.0.0.mca.tmp","r.a.0.mca",
            "r.0.0.mcc","r.0.0.mca/evil","r.0.0.mca\\other","r.0.0",
            "r.2147483648.0.mca"]{
            assert!(parse_region_name(value).is_err(),"{value}");
        }
    }
}
