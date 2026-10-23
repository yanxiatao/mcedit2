//! Native Rust + iced desktop with legacy MCEdit2 positional world arguments.
use iced::widget::{button,column,container,row,scrollable,text,text_input};
use iced::{Element,Length,Theme};
use mcedit2_world::{compatibility_label,inspect_input,launch,WorldSummary};
use mcedit2_world::block_lookup::{parse_xyz,read_world_block};
use mcedit2_world::editing::{parse_bounds,parse_state,Preview,Session};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime,UNIX_EPOCH};

#[derive(Default)]
struct App{
    path:String,
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
    clone_destination:String,
    fill_seed:String,
    patch_path:String,
    edit_report:String,
    sessions:BTreeMap<(usize,usize),Session>,
    prepared:Option<(usize,usize,Preview)>,
    messages:Vec<String>,
    debug:bool,log:bool,verbosity:u8,readonly:bool,
}
#[derive(Debug,Clone)]
enum Message{
    Path(String),Open,Select(usize),Coordinates(String),Dimension(usize),
    InspectBlock,SliceText(String),InspectSlice,
    SelectionText(String),TargetText(String),FilterText(String),
    CloneDestination(String), PreviewClone,
    FillSeed(String), PreviewFill,
    PatchPath(String),ExportPatch,ImportPatch,AnalyzeNativePatch,AnalyzeRegionPlan,
    PreviewEdit,ApplyEdit,UndoEdit,RedoEdit,DiscardEdits,Clear
}
impl App{
    fn boot(args:launch::Arguments)->Self{
        let mut app=App{
            debug:args.debug,log:args.log,verbosity:args.verbosity,
            readonly:args.view,query_coordinates:"0, 64, 0".into(),
            edit_area:"0, 64, 0; 0, 64, 0".into(),
            slice_area:"0, 64, 0; 15, 64, 15".into(),
            edit_target:"minecraft:stone".into(),
            clone_destination:"0, 64, 0".into(),
            fill_seed:"0, 64, 0".into(),..Default::default()
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
    fn prepare_clone(&mut self){
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
        Message::Open=>{
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
            if index<app.worlds.len(){
                app.current=Some(index);
                app.dimension_index=0;
                app.block_result.clear();app.slice_result.clear();
                app.prepared=None;
                app.edit_report.clear();
            }
        }
        Message::Coordinates(value)=>app.query_coordinates=value,
        Message::Dimension(index)=>{
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
        Message::SliceText(value)=>app.slice_area=value,
        Message::InspectSlice=>{
            app.slice_result=match app.current_key(){
                None=>"Select an indexed Java world first.".into(),
                Some(key)=>{
                    match parse_bounds(&app.slice_area) {
                        Err(e)=>format!("Invalid slice bounds: {e}"),
                        Ok(bounds)=>match app.current_session(key).and_then(|session|
                            session.inspect_slice(bounds).map_err(|e|e.to_string())) {
                            Ok(slice)=>slice.text_map(),
                            Err(e)=>format!("2D slice read refused: {e}"),
                        }
                    }
                }
            };
        },
        Message::SelectionText(value)=>{app.edit_area=value;app.prepared=None;}
        Message::TargetText(value)=>{app.edit_target=value;app.prepared=None;}
        Message::FilterText(value)=>{app.edit_filter=value;app.prepared=None;}
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
        Message::FillSeed(value)=>{app.fill_seed=value;app.prepared=None;}
        Message::PreviewFill=>app.prepare_fill(),
        Message::CloneDestination(value)=>{app.clone_destination=value;app.prepared=None;}
        Message::PreviewClone=>app.prepare_clone(),
        Message::PreviewEdit=>app.prepare_batch(),
        Message::ApplyEdit=>app.apply_batch(),
        Message::UndoEdit=>app.history(false),
        Message::RedoEdit=>app.history(true),
        Message::DiscardEdits=>{
            app.prepared=None;
            if let Some(key)=app.current_key(){
                if let Some(editor)=app.sessions.get_mut(&key){
                    editor.discard();
                }
            }
            app.edit_report="Discarded all staged edits for this world/dimension; original disk untouched.".into();
        }
        Message::Clear=>{
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
        text("Current milestone: original world files READ ONLY; edit previews and Undo/Redo are IN MEMORY.")
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
            button("Inspect exact block (read-only)").on_press(Message::InspectBlock)
        ].spacing(12);
        body=body.push(lookup);
        if !app.block_result.is_empty(){
            body=body.push(text(&app.block_result));
        }
        body=body.push(row![
            text_input("2D X/Z map: X1,Y,Z1; X2,Y,Z2",&app.slice_area)
                .on_input(Message::SliceText).width(Length::Fill),
            button("Inspect 2D slice").on_press(Message::InspectSlice)
        ].spacing(8));
        if !app.slice_result.is_empty(){
            body=body.push(text(&app.slice_result).font(iced::Font::MONOSPACE));
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
            button("Preview block-only clone").on_press(Message::PreviewClone)
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
