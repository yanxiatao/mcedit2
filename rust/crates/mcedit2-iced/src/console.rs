//! 统一程序的 --inspect 只读命令行模式。
use mcedit2_world::{compatibility_label, inspect_input, launch};
use std::process::ExitCode;
pub(crate) fn run(options: &launch::Arguments) -> ExitCode {
    if options.worlds.is_empty() {
        log::error!("--inspect 需要至少一个世界路径；不加 --inspect 则打开 GUI。");
        return ExitCode::from(2);
    }
    let mut failures = 0usize;
    for path in &options.worlds {
        match inspect_input(path) {
            Ok(world) => {
                println!("World: {}", world.root.display());
                println!(
                    "Name: {}",
                    world.level_name.as_deref().unwrap_or("(unknown)")
                );
                println!(
                    "DataVersion: {}",
                    world
                        .data_version
                        .map_or_else(|| "absent".into(), |x| x.to_string())
                );
                println!(
                    "Version name: {}",
                    world.version_name.as_deref().unwrap_or("(unknown)")
                );
                println!(
                    "Original level storage: {}",
                    world
                        .storage_format
                        .map_or_else(|| "not specified".into(), |x| x.to_string())
                );
                println!("Compatibility: {}", compatibility_label(&world));
                for dim in &world.dimensions {
                    println!(
                        "  {} regions={} chunks={} format={}",
                        dim.name, dim.regions, dim.chunks, dim.extension
                    );
                }
                for problem in &world.warnings {
                    log::warn!("{problem}");
                }
            }
            Err(e) => {
                failures += 1;
                log::error!("Cannot inspect {}: {e}", path.display());
            }
        }
    }
    if options.debug || options.verbosity > 0 {
        log::info!(
            "Inspected {} requested world(s), {failures} refused.",
            options.worlds.len()
        );
    }
    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
