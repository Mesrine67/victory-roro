//! How the title's shaders stand to the host's waves. For every console
//! container (`*.cdxil`, as `VICTORY_SHADER_SAVE` writes them) in a folder,
//! one line: the file's stem, the stage, the console wave size (64 or 32),
//! the threads of a group, and how the translation for a host of 32-lane
//! waves runs it.
//!
//! `cargo run --release -p victory-shader --example waves -- <folder> [host lanes]`

use victory_shader::native::StageIo;
use victory_shader::{Container, FourCC};

fn main() {
    let mut args = std::env::args().skip(1);
    let folder = args.next().expect("a folder of .cdxil files");
    let lanes: u32 = args.next().and_then(|n| n.parse().ok()).unwrap_or(32);
    victory_shader::emit::set_host_max_wave(lanes);
    let mut paths: Vec<_> = std::fs::read_dir(&folder)
        .expect("the folder")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "cdxil"))
        .collect();
    paths.sort();
    println!("shader\tstage\tconsole_wave\tgroup_threads\ttranslates\thost_waves");
    for path in paths {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let Ok(data) = std::fs::read(&path) else { continue };
        let native = Container::parse(&data)
            .ok()
            .and_then(|c| c.part(FourCC::DXIL).map(|p| p.data.to_vec()))
            .and_then(|dxil| victory_shader::native_programs(&dxil).ok())
            .and_then(|programs| programs.into_iter().find(|p| !matches!(p.io, StageIo::None)));
        let Some(native) = native else {
            println!("{stem}\t?\t?\t?\tno native program\t?");
            continue;
        };
        let threads = match &native.io {
            StageIo::Cs(cs) => cs.threads.iter().product::<u32>(),
            StageIo::Ms(ms) => ms.threads,
            _ => 0,
        };
        let done = victory_shader::emit::translate(&data);
        let how = match (&done, victory_shader::emit::last_waves()) {
            (Err(e), _) => format!("- ({e})"),
            (Ok(_), None) => "?".to_owned(),
            (Ok(_), Some((None, ..))) if native.wave64 => "host waves of 32, no exchange between them set up".to_owned(),
            (Ok(_), Some((None, ..))) => "as on the console".to_owned(),
            (Ok(_), Some((Some(lower_only), local, any, split))) => {
                let mut how = if lower_only {
                    "two-wave form, group fits one host wave".to_owned()
                } else if local {
                    "two host waves, each a wave of its own".to_owned()
                } else {
                    "two host waves exchanging through shared memory with group barriers".to_owned()
                };
                if any {
                    how += ", every mask per host wave";
                }
                if split {
                    how += ", a group barrier before every shared-memory access";
                }
                how
            }
        };
        println!(
            "{stem}\t{:?}\t{}\t{threads}\t{}\t{how}",
            native.stage,
            if native.wave64 { 64 } else { 32 },
            done.is_ok()
        );
    }
}
