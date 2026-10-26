//! Command-line audit using original MCEdit2 flag grammar.
use mcedit2_world::{compatibility_label, inspect_input, launch};
use std::process::ExitCode;
fn main() -> ExitCode {
    let options = match launch::parse_process() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}\n{}", launch::HELP);
            return ExitCode::from(2);
        }
    };
    if options.help {
        print!("{}", launch::HELP);
        return ExitCode::SUCCESS;
    }
    if let Err(e) = launch::validate(&options) {
        eprintln!("{e}");
        return ExitCode::from(2);
    }
    if options.reset_prefs {
        if let Err(e) = launch::reset_rust_prefs() {
            eprintln!("Separate Rust preferences reset refused: {e}");
            return ExitCode::FAILURE;
        }
    }
    if options.worlds.is_empty() {
        eprintln!("No file path given; run the iced binary mcedit2 for GUI.");
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
                    eprintln!("WARNING: {problem}");
                }
            }
            Err(e) => {
                failures += 1;
                eprintln!("Cannot inspect {}: {e}", path.display());
            }
        }
    }
    if options.debug || options.verbosity > 0 {
        eprintln!(
            "Inspected {} requested world(s), {failures} refused.",
            options.worlds.len()
        );
    }
    if options.log {
        eprintln!("Original -log accepted; actual Rust GPU tracing not yet implemented.");
    }
    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
