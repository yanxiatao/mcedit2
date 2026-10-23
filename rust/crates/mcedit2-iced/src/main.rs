//! Native Rust + iced desktop with legacy MCEdit2 positional world arguments.
mod isometric;
use iced::widget::{button,canvas,column,container,row,scrollable,text,text_input};
use mcedit2_world::surface::SurfaceView;
use isometric::IsometricSurface;
use iced::{Element,Length,Theme};
use mcedit2_world::{compatibility_label,inspect_input,launch,WorldSummary};
use mcedit2_world::block_lookup::{parse_xyz,read_world_block};
use mcedit2_world::editing::{parse_bounds,parse_state,BeScalarPreview,Bounds,Preview,Session,Slice,SliceCell,VerticalSlice};
use mcedit2_world::region_plan::NativeRegionCandidate;
use mcedit2_world::clipboard::HorizontalTransform;
use mcedit2_world::region_transaction::{apply_single_region,finalize_single_region,rollback_pending_region};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime,UNIX_EPOCH};

/// Snapshot of the exact single-Region image explicitly approved in the UI.
/// No file is ever changed merely by generating this preview.
struct SaveIntent {
    key:(usize,usize),
    timestamp:u32,
    candidate:NativeRegionCandidate,
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

#[derive(Default)]
struct App{
    path:String,
    surface_area:String,
    surface_camera:u8,
    surface:Option<(usize,usize,SurfaceView)>,
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
    plugin_manifest_path:String,
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
    be_prepared:Option<(usize,usize,BeScalarPreview)>,
    edit_report:String,
    sessions:BTreeMap<(usize,usize),Session>,
    prepared:Option<(usize,usize,Preview)>,
    messages:Vec<String>,
    debug:bool,log:bool,verbosity:u8,readonly:bool,
}
#[derive(Debug,Clone)]
enum Message{
    Path(String),Open,Select(usize),Coordinates(String),Dimension(usize),
    SurfaceArea(String),InspectSurface,InspectVolume,PickSurface(i32,i32,i32),RotateSurface,
    BeScalarField(String),BeScalarValue(String),PreviewBeScalar,
    StageBeScalar,UndoBeScalar,RedoBeScalar,InspectStagedBe,
    BeSnapshotPath(String),ExportOriginalBe,VerifyOriginalBe,
    PluginManifestPath(String),PreviewPlugin,
    InspectBlock,InspectOriginalBlockEntity,SliceText(String),InspectSlice,PickSlice(i32,i32,i32),MoveSlice(i32,i32,i32),
    VerticalText(String),InspectVertical,PickVertical(usize,usize),
    SelectionText(String),TargetText(String),FilterText(String),
    CloneDestination(String), PreviewClone,PreviewMove,
    FillSeed(String), PreviewFill,
    PatchPath(String),ExportPatch,ImportPatch,AnalyzeNativePatch,AnalyzeRegionPlan,
    ClipboardPath(String),ClipboardDestination(String),ExportClipboard,PreviewPasteClipboard,
    PreviewTransformedClipboard(HorizontalTransform),
    SchematicPath(String),PreviewLegacySchematic,ExportLegacySchematic,
    SpongePath(String),PreviewSpongeV2,ExportSpongeV2,
    PrepareWorldSave,SavePhrase(String),ConfirmWorldSave,
    RecoverRegion(String),RecoveryPhrase(String),FinalizeRegion,RollbackRegion,
    PreviewEdit,ApplyEdit,UndoEdit,RedoEdit,DiscardEdits,Clear
}
impl App{
    fn inspect_surface(&mut self,exposed_volume:bool){
        self.surface=None;
        self.surface_report=match self.current_key(){
            None=>"Select a Java world first.".into(),
            Some(key)=>match parse_bounds(&self.surface_area){
                Err(e)=>format!("Invalid 3D surface bounds: {e}"),
                Ok(bounds)=>{
                    let w=bounds.maximum[0] as i64-bounds.minimum[0] as i64+1;
                    let depth=bounds.maximum[2] as i64-bounds.minimum[2] as i64+1;
                    if w<=0 || w>16 || depth<=0 || depth>16{
                        "Original 3D surface footprint must fit 16x16.".into()
                    }else{
                        match self.current_session(key).and_then(|session|
                            (if exposed_volume{
                                session.inspect_exposed_volume(
                                    bounds.minimum[0],bounds.minimum[2],
                                    bounds.minimum[1],bounds.maximum[1],
                                    w as u8,depth as u8)
                            }else{
                                session.inspect_surface(
                                    bounds.minimum[0],bounds.minimum[2],
                                    bounds.minimum[1],bounds.maximum[1],
                                    w as u8,depth as u8)
                            }).map_err(|e|e.to_string())){
                            Ok(result)=>{
                                let message=format!(
                                    "3D {}: {} visible voxels, {} missing source columns across {} chunks. Click top/side faces for exact XYZ; cropped boundary faces are viewport cuts, not inferred air.",
                                    if exposed_volume{"exposed multilayer volume"}else{"highest-column surface"},
                                    result.voxels.len(),result.missing_columns,result.source_chunks
                                );
                                self.surface=Some((key.0,key.1,result));
                                message
                            },
                            Err(e)=>format!("3D surface refused: {e}"),
                        }
                    }
                }
            }
        };
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
        self.edit_area=format!("{x},{y},{z};{x},{y},{z}");
        self.be_prepared=None;
        self.save_preview=None;self.prepared=None;
        self.surface_report=format!(
            "Selected exact original 3D voxel ({x},{y},{z}). Edits still require normal guarded preview and Apply."
        );
    }
    fn inspect_vertical_view(&mut self){
        self.vertical_grid=None;
        self.vertical_report=match self.current_key(){
            None=>"Select a Java world first.".into(),
            Some(key)=>match parse_bounds(&self.vertical_area){
                Err(e)=>format!("Invalid vertical area: {e}"),
                Ok(bounds)=>match self.current_session(key)
                    .and_then(|editor|editor.inspect_vertical_slice(bounds)
                        .map_err(|e|e.to_string())){
                    Ok(slice)=>{
                        let explanation=slice.text_map();
                        let h=slice.rows.len();
                        let width=slice.rows.first().map_or(0,Vec::len);
                        if width<=16&&h<=16 {
                            self.vertical_grid=Some((key.0,key.1,slice));
                            format!("Clickable {width}x{h} original vertical plane. Select exact blocks below.\n{explanation}")
                        }else{
                            format!("Text-only {width}x{h} original plane; use <=16x16 for clickable cells.\n{explanation}")
                        }
                    },
                    Err(e)=>format!("Vertical inspection refused: {e}"),
                }
            }
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
        self.edit_area=format!("{x},{y},{z};{x},{y},{z}");
        self.query_coordinates=format!("{x}, {y}, {z}");
        self.be_prepared=None;
        self.save_preview=None;
        self.prepared=None;
        self.block_result=format!("Selected vertical plane original block at ({x},{y},{z}); inspect the source Block Entity before editing.");
    }
    fn inspect_slice_view(&mut self){
        self.slice_grid=None;
        self.slice_result=match self.current_key() {
            None=>"Select an indexed Java world first.".into(),
            Some(key)=>match parse_bounds(&self.slice_area) {
                Err(e)=>format!("Invalid 2D slice bounds: {e}"),
                Ok(bounds)=>{
                    let width=(bounds.maximum[0] as i64-bounds.minimum[0] as i64)+1;
                    let depth=(bounds.maximum[2] as i64-bounds.minimum[2] as i64)+1;
                    if bounds.minimum[1]!=bounds.maximum[1]{
                        "2D slice view requires one Y layer.".into()
                    }else {
                        match self.current_session(key)
                            .and_then(|session|session.inspect_slice(bounds)
                                .map_err(|e|e.to_string())) {
                            Ok(slice)=>{
                                let legend=slice.text_map();
                                if width<=16 && depth<=16 {
                                    self.slice_grid=Some((key.0,key.1,slice));
                                    format!("Interactive 2D grid ({width}x{depth}). Click a cell to inspect/select; @ = original Block Entity, ? = missing Section, · = missing chunk.\n{legend}")
                                } else {
                                    format!("Text-only slice ({width}x{depth}); use a 16x16 or smaller window for clickable cells.\n{legend}")
                                }
                            }
                            Err(e)=>format!("2D slice read refused: {e}"),
                        }
                    }
                }
            }
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
            Ok((SaveIntent{key:(wi,di),timestamp,candidate},label))
        })();
        match result {
            Ok((intent,label))=>{self.save_preview=Some(intent);self.edit_report=label;},
            Err(e)=>self.edit_report=format!("Disk save preflight refused: {e}"),
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
            surface_area:"0,-64,0;15,-49,15".into(),..Default::default()
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
        self.edit_report=match result{
            Ok(count)=>format!(
                "Staged {count} block changes as ONE Undo. MEMORY ONLY; no world file was saved."
            ),
            Err(error)=>format!("Edit transaction REFUSED before mutation: {error}"),
        };
    }
    fn history(&mut self, redo:bool){
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
        self.edit_report=match result{
            Ok(true)=>format!("{} in-memory original block transaction; no disk writes.",
                if redo{"Redid"}else{"Undid"}),
            Ok(false)=>"No matching in-memory history to undo/redo.".into(),
            Err(error)=>format!("Undo/Redo REFUSED: {error}"),
        };
    }
}
fn update(app:&mut App,message:Message){
    match message{
        Message::Path(x)=>app.path=x,
        Message::SurfaceArea(value)=>{app.surface_area=value;app.surface=None;},
        Message::InspectSurface=>app.inspect_surface(false),
        Message::InspectVolume=>app.inspect_surface(true),
        Message::PickSurface(x,y,z)=>app.pick_surface(x,y,z),
        Message::RotateSurface=>app.surface_camera=(app.surface_camera+1)%4,
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
            app.slice_grid=None;
            app.vertical_grid=None;
            if app.path.trim().is_empty(){
                app.messages.push("Enter a Java world folder or level.dat path".into());
            } else {
                match inspect_input(app.path.trim()){
                    Ok(world)=>{
                        app.current=Some(app.worlds.len());
                        app.dimension_index=0;
                        app.block_result.clear();app.slice_result.clear();
                        app.prepared=None;
                        app.edit_report.clear();
                        app.worlds.push(world);
                    }
                    Err(e)=>app.messages.push(format!("Cannot inspect world: {e}"))
                }
            }
        }
        Message::Select(index)=>{
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
        Message::BeSnapshotPath(value)=>app.be_snapshot_path=value,
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
            app.be_prepared=None;
            app.surface=None;
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
}
fn view(app:&App)->Element<'_,Message>{
    let controls=row![
        text_input("Java world folder, level.dat, or level.dat_old",&app.path)
            .on_input(Message::Path).on_submit(Message::Open)
            .width(Length::Fill),
        button("Inspect world").on_press(Message::Open),
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
            text_input("3D voxel surface X1,Y1,Z1;X2,Y2,Z2 (16x16x32 max)",
                &app.surface_area).on_input(Message::SurfaceArea).width(Length::Fill),
            button("Top-surface 3D").on_press(Message::InspectSurface),
            button("Exposed multi-layer 3D").on_press(Message::InspectVolume)
        ].spacing(8));
        if let Some((wi,di,view))=&app.surface{
            if app.current_key()==Some((*wi,*di)){
                body=body.push(button(text(format!("Rotate 3D camera 90° (view {})",app.surface_camera)))
                    .on_press(Message::RotateSurface));
                body=body.push(canvas(IsometricSurface{
                    surface:view.clone(),camera_quarters:app.surface_camera
                })
                    .width(Length::Fill).height(Length::Fixed(440.0)));
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
                let mut visual=column![].spacing(1);
                for (iz,cells) in slice.rows.iter().enumerate() {
                    let mut row_buttons=row![].spacing(1);
                    for (ix,cell) in cells.iter().enumerate() {
                        let x=slice.bounds.minimum[0]+ix as i32;
                        let y=slice.bounds.minimum[1];
                        let z=slice.bounds.minimum[2]+iz as i32;
                        row_buttons=row_buttons.push(
                            button(text(slice_cell_label(cell)).size(12))
                                .width(Length::Fixed(28.0))
                                .height(Length::Fixed(24.0))
                                .on_press(Message::PickSlice(x,y,z))
                        );
                    }
                    visual=visual.push(row_buttons);
                }
                body=body.push(text("Interactive 2D blocks (click to inspect and select a 1x1 edit area)"));
                body=body.push(scrollable(visual).height(Length::Fixed(420.0)));
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
                let mut rows=column![].spacing(1);
                for (y_index,line) in plane.rows.iter().enumerate(){
                    let mut cells=row![].spacing(1);
                    for (x_index,cell) in line.iter().enumerate(){
                        cells=cells.push(
                            button(text(slice_cell_label(cell)).size(12))
                                .width(Length::Fixed(28.0))
                                .height(Length::Fixed(24.0))
                                .on_press(Message::PickVertical(y_index,x_index))
                        );
                    }
                    rows=rows.push(cells);
                }
                body=body.push(text("Vertical slice: top row is highest Y, @ indicates original BE."));
                body=body.push(scrollable(rows).height(Length::Fixed(420.0)));
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
        .title("MCEdit2 — Rust + iced").theme(Theme::Dark).run()
}

#[cfg(test)]
mod tests{
    use super::{parse_region_name,shifted_slice,slice_cell_label};
    use mcedit2_world::editing::{Bounds,SliceCell};
    use mcedit2_world::sections::State;
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
