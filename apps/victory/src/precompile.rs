//! `victory precompile`: makes the host pipelines of every pipeline the
//! title created in earlier runs, for the host GPU in use, so that a run
//! finds them in the pipeline library instead of compiling them when they
//! are first drawn with.
//!
//! A run lists what the title creates (`created-pipelines.bin` in the
//! cache; the title creates nearly all of its pipelines while it loads).
//! This command finds the listed shaders in the title's own files, makes
//! each pipeline as a run would, and saves the library as it goes: a second
//! call picks up where an interrupted one stopped (what is in the library
//! loads in milliseconds).
//!
//! It must run as the title does: the same build and the same translator
//! switches (`VICTORY_SHADER_LOOP_LIMIT`, `VICTORY_WAVE_*`, ...), or the
//! shaders it makes are not the ones a run asks for, and on the same GPU
//! (`VICTORY_ADAPTER`): the library belongs to one adapter and driver.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use victory_hle::modules::d3d12x::{self, Created, Precompiled};
use victory_shader::Container;

use crate::shader::{MAX_FILE, Source, sources};

pub struct Options {
    /// Where the title's shaders are looked for (default: the game folder).
    pub paths: Vec<PathBuf>,
    /// Report what would be made and stop.
    pub plan: bool,
    /// Only pipelines an earlier run has drawn with.
    pub used: bool,
    /// At most this many pipelines.
    pub limit: Option<usize>,
    pub jobs: Option<usize>,
    /// Seconds between saves of the library.
    pub save_every: u64,
    /// Diagnostics: list every shader container of the title's files here.
    pub list_shaders: Option<PathBuf>,
    /// Diagnostics: shader hashes to make compute pipelines of, in place
    /// of the runs' list.
    pub compute_shaders: Option<PathBuf>,
}

/// Shader containers by the first 8 bytes of their BLAKE3, as a run's log
/// and the list of created pipelines name them.
type Containers = HashMap<[u8; 8], Arc<[u8]>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A count with thousands apart.
fn n(v: usize) -> String {
    let digits = v.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A time span in words.
fn span(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    match s {
        0..120 => format!("{s} s"),
        120..7200 => format!("{} min", s / 60),
        _ => format!("{} h {:02} min", s / 3600, s % 3600 / 60),
    }
}

/// Files likeliest to hold shaders come first: the search ends as soon as
/// every wanted one is found.
fn rank(path: &Path) -> u8 {
    let name = path.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.contains("shader") {
        0
    } else if name.ends_with(".exe") || name.ends_with(".dll") {
        1
    } else {
        2
    }
}

/// The shader containers in the files under `paths`: those in `wanted`, or
/// all of them. As `victory warm` finds them: packed records and plain
/// containers, cut to the container's own size as the title passes it.
/// `names` takes the names packed records carry (the title's own, like
/// `smh_sim_0:CS_InitSimBuffer_de9d2ad2_Wrapped`), for reading logs.
fn scan(
    paths: &[PathBuf],
    wanted: Option<&HashSet<[u8; 8]>>,
    threads: usize,
    names: Option<&Mutex<HashMap<[u8; 8], String>>>,
) -> Containers {
    let mut files = Vec::new();
    let mut large = Vec::new();
    for root in paths {
        for entry in walkdir::WalkDir::new(root).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_file() {
                continue;
            }
            let len = entry.metadata().map_or(0, |m| m.len());
            if len > MAX_FILE {
                large.push(entry.into_path());
            } else {
                files.push((entry.into_path(), len));
            }
        }
    }
    files.sort_by_key(|(path, len)| (rank(path), *len));
    let found = Mutex::new(Containers::new());
    let started = Instant::now();
    let mut read = 0;
    for (path, len) in &files {
        if wanted.is_some_and(|w| lock(&found).len() >= w.len()) {
            break;
        }
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(e) => {
                println!("precompile   not read: {} ({e})", path.display());
                continue;
            }
        };
        read += 1;
        let jobs = sources(&data);
        let next = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..threads.min(jobs.len()) {
                s.spawn(|| {
                    while let Some(job) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let Some(bytes) = job.bytes() else { continue };
                        if Container::parse(&bytes).is_err() {
                            continue;
                        }
                        let Some(total) = bytes.get(0x18..0x1c).and_then(|b| b.try_into().ok()) else {
                            continue;
                        };
                        let total = u32::from_le_bytes(total) as usize;
                        let bytes = &bytes[..total.min(bytes.len())];
                        let hash = blake3::hash(bytes);
                        let id: [u8; 8] = hash.as_bytes()[..8].try_into().expect("8 of 32 bytes");
                        if wanted.is_some_and(|w| !w.contains(&id)) {
                            continue;
                        }
                        if let (Some(names), Source::Packed(record)) = (names, job)
                            && let Some(name) = record.name
                        {
                            lock(names).entry(id).or_insert_with(|| name.to_owned());
                        }
                        lock(&found).entry(id).or_insert_with(|| Arc::from(bytes));
                    }
                });
            }
        });
        if !jobs.is_empty() {
            println!(
                "{:6.0} s  searched {} ({} MB): {} shaders found so far",
                started.elapsed().as_secs_f64(),
                path.display(),
                len >> 20,
                n(lock(&found).len())
            );
        }
    }
    let found = found.into_inner().unwrap_or_else(|e| e.into_inner());
    let all = wanted.is_some_and(|w| found.len() >= w.len());
    println!(
        "precompile   searched {read} of {} files in {}; {} shaders found",
        files.len(),
        span(started.elapsed().as_secs_f64()),
        n(found.len())
    );
    if !all && !large.is_empty() {
        println!(
            "precompile   {} files above {} GB were not searched (as `victory warm` leaves them), e.g. {}",
            large.len(),
            MAX_FILE >> 30,
            large[0].display()
        );
    }
    found
}

/// Outcomes, in the order of [`Counts`].
const READY: usize = 0;
const LEFT_OUT: usize = 1;
const NO_CONTAINER: usize = 2;
const UNTRANSLATED: usize = 3;
const REFUSED: usize = 4;
const DAMAGED: usize = 5;
type Counts = [AtomicUsize; 6];

/// Counts down the workers still running, also when one panics.
struct Alive<'a>(&'a AtomicUsize);

impl Drop for Alive<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn run(o: Options) -> Result<()> {
    let cfg = victory_core::Config::load()?;
    d3d12x::use_shader_cache(cfg.cache_subdir("shaders")?);
    let threads = o
        .jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get().saturating_sub(2)))
        .max(1);
    if let Some(file) = &o.list_shaders {
        let paths = crate::default_paths(o.paths)?;
        let names = Mutex::new(HashMap::new());
        let found = scan(&paths, None, threads, Some(&names));
        let names = lock(&names);
        // Id, size, and the title's name for it where its record has one.
        let mut lines: Vec<String> = found
            .iter()
            .map(|(id, bytes)| {
                let hex = id.iter().map(|b| format!("{b:02x}")).collect::<String>();
                format!("{hex} {} {}", bytes.len(), names.get(id).map_or("?", String::as_str))
            })
            .collect();
        lines.sort();
        std::fs::write(file, lines.join("\n") + "\n").with_context(|| format!("writing {}", file.display()))?;
        println!("precompile   {} shader containers listed in {}", n(lines.len()), file.display());
        return Ok(());
    }

    let listed = match &o.compute_shaders {
        Some(file) => {
            let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
            let mut listed = Vec::new();
            for hash in text.split_whitespace() {
                let id = u64::from_str_radix(hash, 16)
                    .ok()
                    .filter(|_| hash.len() == 16)
                    .with_context(|| format!("{hash}: not a 16-digit shader hash"))?;
                listed.push((Created::compute(id.to_be_bytes()), false));
            }
            println!("precompile   --compute-shaders: a compute pipeline of each of {} shaders, not the runs' list", listed.len());
            listed
        }
        None => d3d12x::created_pipelines(),
    };
    if listed.is_empty() {
        println!(
            "precompile   nothing to do yet: no run has listed the title's pipelines.\n\
             precompile   Start the title once and let it load into the world (it creates nearly all of\n\
             precompile   its pipelines while it loads), then call this again."
        );
        return Ok(());
    }
    let drawn = listed.iter().filter(|(_, drawn)| *drawn).count();
    let compute = listed.iter().filter(|(p, _)| p.computes()).count();
    let mesh = listed.iter().filter(|(p, _)| p.meshes()).count();
    println!(
        "precompile   listed by earlier runs: {} pipelines ({} compute, {} mesh, {} other); {} of them drawn with so far",
        n(listed.len()),
        n(compute),
        n(mesh),
        n(listed.len() - compute - mesh),
        n(drawn)
    );

    // The card: the library belongs to one adapter, and a library made on
    // another replaces it at the next save.
    let (card, memory) = d3d12x::host_adapter().context("no host D3D12 device: nothing can be compiled")?;
    let wanted_card = std::env::var("VICTORY_ADAPTER").unwrap_or_default().trim().to_lowercase();
    if wanted_card.is_empty() {
        println!("precompile   for {card} ({memory} MB), the default adapter (VICTORY_ADAPTER picks another)");
    } else if card.to_lowercase().contains(&wanted_card) {
        println!("precompile   for {card} ({memory} MB)");
    } else {
        bail!(
            "VICTORY_ADAPTER={wanted_card} matches no adapter (the default is {card}): nothing was compiled, \
             so that the pipeline library of the wanted card is not replaced by another card's"
        );
    }

    // What a run has drawn with first, then the compute passes (many run
    // once: a late one is a lost pass), then the rest; each group in the
    // order the title created it.
    let mut work: Vec<&(Created, bool)> = listed.iter().filter(|(_, drawn)| !o.used || *drawn).collect();
    work.sort_by_key(|(p, drawn)| match (*drawn, p.computes()) {
        (true, _) => 0,
        (false, true) => 1,
        (false, false) => 2,
    });
    if let Some(limit) = o.limit {
        work.truncate(limit);
    }
    let wanted: HashSet<[u8; 8]> = work.iter().flat_map(|(p, _)| p.shaders.iter().map(|s| s.1)).collect();
    println!(
        "precompile   to make: {} pipelines from {} shaders{}{}",
        n(work.len()),
        n(wanted.len()),
        if o.used { ", only those drawn with (--used)" } else { "" },
        o.limit.map_or(String::new(), |l| format!(", at most {l} (--limit)"))
    );
    let paths = crate::default_paths(o.paths)?;
    let containers = scan(&paths, Some(&wanted), threads, None);
    let without = work
        .iter()
        .filter(|(p, _)| p.shaders.iter().any(|s| !containers.contains_key(&s.1)))
        .count();
    if without > 0 {
        println!(
            "precompile   shaders not found in the title's files: {} (pipelines that cannot be made here: {})",
            n(wanted.len() - containers.len()),
            n(without)
        );
    }
    let [_, _, _, library] = d3d12x::pipeline_counts();
    println!("precompile   the pipeline library holds {} MB now", library >> 20);
    if o.plan {
        println!("precompile   plan only (--plan): nothing was compiled");
        return Ok(());
    }

    let started = Instant::now();
    let counts: Counts = [const { AtomicUsize::new(0) }; 6];
    let next = AtomicUsize::new(0);
    let alive = AtomicUsize::new(threads);
    // What each worker is at, and since when.
    let at: Vec<Mutex<Option<(usize, Instant)>>> = (0..threads).map(|_| Mutex::new(None)).collect();
    let reasons: Mutex<HashMap<&'static str, usize>> = Mutex::new(HashMap::new());
    let missing: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let done = || counts.iter().map(|c| c.load(Ordering::Relaxed)).sum::<usize>();
    let line = |what: &str| {
        let [made, loaded, other_key, library] = d3d12x::pipeline_counts();
        let done = done();
        let elapsed = started.elapsed().as_secs_f64();
        let left = if done > 0 { elapsed / done as f64 * (work.len() - done) as f64 } else { 0.0 };
        let slowest = at
            .iter()
            .filter_map(|a| *lock(a))
            .max_by_key(|(_, since)| since.elapsed())
            .filter(|(_, since)| since.elapsed() >= Duration::from_secs(20))
            .map_or(String::new(), |(i, since)| {
                format!("; at {} for {}", work[i].0.key, span(since.elapsed().as_secs_f64()))
            });
        println!(
            "{elapsed:6.0} s  {what}{} of {} pipelines: {} ready ({} compiled, {} were in the library), {} left out, \
             {} not translatable, {} refused, {} without their shader{}; library {} MB; about {} to go{slowest}",
            n(done),
            n(work.len()),
            n(counts[READY].load(Ordering::Relaxed)),
            n(made as usize),
            n(loaded as usize),
            n(counts[LEFT_OUT].load(Ordering::Relaxed)),
            n(counts[UNTRANSLATED].load(Ordering::Relaxed)),
            n(counts[REFUSED].load(Ordering::Relaxed)),
            n(counts[NO_CONTAINER].load(Ordering::Relaxed) + counts[DAMAGED].load(Ordering::Relaxed)),
            if other_key > 0 { format!(", {other_key} under another key than listed") } else { String::new() },
            library >> 20,
            span(left)
        );
        let _ = std::io::stdout().flush();
    };
    println!("precompile   compiling on {threads} threads; the library is saved every {} s", o.save_every);
    std::thread::scope(|s| {
        for at in &at {
            s.spawn(|| {
                let _alive = Alive(&alive);
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((p, _)) = work.get(i) else { break };
                    *lock(at) = Some((i, Instant::now()));
                    let outcome = d3d12x::precompile(p, &|id| containers.get(id).cloned());
                    *lock(at) = None;
                    let count = match outcome {
                        Precompiled::Ready => READY,
                        Precompiled::LeftOut(why) => {
                            *lock(&reasons).entry(why).or_default() += 1;
                            LEFT_OUT
                        }
                        Precompiled::NoContainer => {
                            let mut missing = lock(&missing);
                            if missing.len() < 20 {
                                missing.push(p.key.clone());
                            }
                            NO_CONTAINER
                        }
                        Precompiled::Untranslated => UNTRANSLATED,
                        Precompiled::Refused => REFUSED,
                        Precompiled::Damaged => DAMAGED,
                    };
                    counts[count].fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        let (mut said, mut saved) = (Instant::now(), Instant::now());
        while alive.load(Ordering::Relaxed) > 0 {
            std::thread::sleep(Duration::from_millis(250));
            if said.elapsed() >= Duration::from_secs(15) {
                line("");
                said = Instant::now();
            }
            if saved.elapsed() >= Duration::from_secs(o.save_every.max(20)) {
                let began = Instant::now();
                d3d12x::save_pipeline_cache();
                println!(
                    "{:6.0} s  library saved in {}",
                    started.elapsed().as_secs_f64(),
                    span(began.elapsed().as_secs_f64())
                );
                saved = Instant::now();
            }
        }
    });
    d3d12x::save_pipeline_cache();
    line("done: ");
    for (why, count) in lock(&reasons).iter() {
        println!("precompile   left out as in a run: {} ({why})", n(*count));
    }
    let missing = lock(&missing);
    if !missing.is_empty() {
        println!("precompile   pipelines without their shader, by key (the first {}): {}", missing.len(), missing.join(" "));
    }
    if done() < work.len() {
        bail!("{} of {} pipelines were not reached: a worker stopped early", work.len() - done(), work.len());
    }
    Ok(())
}
