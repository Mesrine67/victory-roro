//! The list of every pipeline the title creates, kept across runs beside
//! the pipeline library (`created-pipelines.bin`): what `victory
//! precompile` makes host pipelines from before a run draws with them.
//!
//! A run makes a host pipeline on its first draw (`make`), so a place the
//! title has not shown yet still costs a compile when it comes, and a pass
//! that runs once is lost if its compile is late. The title creates nearly
//! all of its pipelines while it loads (some 60,000 in 20 seconds), long
//! before it draws with them: one run lists them, and the precompile step
//! makes them all.
//!
//! An entry holds what a host pipeline is made from and no more: each
//! shader's stage and hash (the containers are found again in the title's
//! own files) and, for a pipeline from a stream, the state subobjects as
//! the title gave them. Those go through `GraphicsState::read` again, so a
//! replayed pipeline has the key and the library name the run's has. The
//! input layout is the one subobject with pointers: it is kept flat.
//!
//! The file is only added to: a tag, a length, the record, a check. A run
//! is often ended from outside, so records are written every two seconds,
//! and a torn last record is cut off by the next run.

use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use windows::Win32::Graphics::Direct3D12::{D3D12_INPUT_CLASSIFICATION, D3D12_INPUT_ELEMENT_DESC};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;
use windows::core::PCSTR;

use super::Shader;
use super::host::GraphicsState;

const FILE: &str = "created-pipelines.bin";
const MAGIC: &[u8; 8] = b"VICPIPE1";
/// Record tags: a graphics state, and a pipeline naming one.
const STATE: u8 = 1;
const PIPELINE: u8 = 2;
/// The input layout's subobject type, kept flat.
const INPUT_LAYOUT: u32 = 12;

/// Stage names by their number in the file.
const STAGES: [&str; 8] = [
    "vertex",
    "pixel",
    "domain",
    "hull",
    "geometry",
    "compute",
    "amplification",
    "mesh",
];

/// Whether a subobject of `kind` is kept as the title gave it: the ones
/// `GraphicsState::read` takes in that hold no pointers.
fn kept(kind: u32) -> bool {
    matches!(kind, 8 | 9 | 10 | 11 | 13 | 14 | 15 | 16 | 17 | 21)
}

/// A stream's graphics state as it goes into the list.
#[derive(Default)]
pub(crate) struct Recorded(Vec<u8>);

impl Recorded {
    /// Takes in one subobject of the stream, whole.
    pub(crate) fn subobject(&mut self, kind: u32, bytes: &[u8]) {
        if kept(kind) {
            self.0.extend_from_slice(&kind.to_le_bytes());
            self.0.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            self.0.extend_from_slice(bytes);
        }
    }

    /// The state with the input layout `g` read from the same stream.
    pub(crate) fn finish(mut self, g: &GraphicsState) -> Vec<u8> {
        let mut flat = Vec::new();
        let mut count = 0u32;
        for (name, e) in g.title_inputs() {
            count += 1;
            let name = name.to_bytes();
            flat.extend_from_slice(&(name.len() as u32).to_le_bytes());
            flat.extend_from_slice(name);
            for v in [
                e.SemanticIndex,
                e.Format.0 as u32,
                e.InputSlot,
                e.AlignedByteOffset,
                e.InputSlotClass.0 as u32,
                e.InstanceDataStepRate,
            ] {
                flat.extend_from_slice(&v.to_le_bytes());
            }
        }
        if count > 0 {
            self.0.extend_from_slice(&INPUT_LAYOUT.to_le_bytes());
            self.0.extend_from_slice(&(flat.len() as u32 + 4).to_le_bytes());
            self.0.extend_from_slice(&count.to_le_bytes());
            self.0.extend_from_slice(&flat);
        }
        self.0
    }
}

/// Reads little-endian fields off the front of a slice.
struct Fields<'a>(&'a [u8]);

impl<'a> Fields<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }

    fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_le_bytes)
    }

    fn u8(&mut self) -> Option<u8> {
        self.array::<1>().map(|b| b[0])
    }
}

/// The graphics state a recorded stream gives; `None` if the record is not
/// one of ours (a damaged file).
pub(crate) fn replay(state: &[u8]) -> Option<GraphicsState> {
    let mut g = GraphicsState::default();
    let mut f = Fields(state);
    while !f.0.is_empty() {
        let kind = f.u32()?;
        let len = f.u32()? as usize;
        let mut bytes = Fields(f.take(len)?);
        if kind == INPUT_LAYOUT {
            let count = bytes.u32()?;
            let mut elements = Vec::new();
            for _ in 0..count {
                let len = bytes.u32()? as usize;
                let name = CString::new(bytes.take(len)?).ok()?;
                let mut v = [0u32; 6];
                for v in &mut v {
                    *v = bytes.u32()?;
                }
                elements.push((
                    name,
                    D3D12_INPUT_ELEMENT_DESC {
                        SemanticName: PCSTR::null(),
                        SemanticIndex: v[0],
                        Format: DXGI_FORMAT(v[1] as i32),
                        InputSlot: v[2],
                        AlignedByteOffset: v[3],
                        InputSlotClass: D3D12_INPUT_CLASSIFICATION(v[4] as i32),
                        InstanceDataStepRate: v[5],
                    },
                ));
            }
            g.set_inputs(elements);
        } else {
            let (size, _) = super::subobject(kind)?;
            if !kept(kind) || bytes.0.len() < size {
                return None;
            }
            // SAFETY: a whole subobject of this kind, as the stream held it.
            unsafe { g.read(kind, bytes.0.as_ptr()) };
        }
    }
    Some(g)
}

/// A pipeline the title created in some run.
#[derive(Clone)]
pub struct Created {
    /// What names it across runs (`make::Pending`'s key).
    pub key: String,
    /// Each stage and the first 8 bytes of its container's BLAKE3.
    pub shaders: Vec<(&'static str, [u8; 8])>,
    /// Its graphics state, if it came from a stream ([`replay`]).
    pub(crate) state: Option<Arc<Vec<u8>>>,
}

impl Created {
    /// A compute pipeline as the plain compute description creates it (no
    /// stream, so no state) from the shader with this hash: for trying
    /// `victory precompile` on chosen shaders without a run's list.
    pub fn compute(hash: [u8; 8]) -> Created {
        // As `make::Pending::new` keys it.
        let mut key = blake3::Hasher::new();
        key.update(b"compute");
        key.update(to_hex(&hash).as_bytes());
        Created {
            key: key.finalize().to_hex()[..32].to_owned(),
            shaders: vec![("compute", hash)],
            state: None,
        }
    }

    /// Whether it is a compute pipeline.
    pub fn computes(&self) -> bool {
        matches!(self.shaders.as_slice(), [(stage, _)] if *stage == "compute")
    }

    /// Whether it has a mesh shader.
    pub fn meshes(&self) -> bool {
        self.shaders.iter().any(|(stage, _)| *stage == "mesh")
    }
}

/// What a file of the list holds, and how far it is whole.
#[derive(Default)]
struct Listed {
    states: HashMap<[u8; 16], Arc<Vec<u8>>>,
    pipelines: Vec<Created>,
    keys: HashSet<[u8; 16]>,
    /// The bytes up to the end of the last whole record.
    whole: usize,
}

fn check(tag: u8, record: &[u8]) -> [u8; 4] {
    let mut h = blake3::Hasher::new();
    h.update(&[tag]);
    h.update(record);
    let hash = h.finalize();
    let b = hash.as_bytes();
    [b[0], b[1], b[2], b[3]]
}

/// Frames `record` for the file.
fn frame(tag: u8, record: &[u8], out: &mut Vec<u8>) {
    out.push(tag);
    out.extend_from_slice(&(record.len() as u32).to_le_bytes());
    out.extend_from_slice(record);
    out.extend_from_slice(&check(tag, record));
}

fn hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != 2 * N || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Reads a file of the list; `None` if it is not one. A record that is
/// torn or does not check ends the reading: what came before it stands.
fn listed(bytes: &[u8]) -> Option<Listed> {
    let mut f = Fields(bytes);
    if f.take(MAGIC.len())? != MAGIC {
        return None;
    }
    let mut l = Listed {
        whole: MAGIC.len(),
        ..Default::default()
    };
    loop {
        let record = (|| {
            let tag = f.u8()?;
            let len = f.u32()? as usize;
            let record = f.take(len)?;
            (f.array::<4>()? == check(tag, record)).then_some((tag, record))
        })();
        let Some((tag, record)) = record else { break };
        let mut r = Fields(record);
        let taken = (|| {
            match tag {
                STATE => {
                    let id = r.array::<16>()?;
                    l.states.insert(id, Arc::new(r.0.to_vec()));
                }
                PIPELINE => {
                    let key = r.array::<16>()?;
                    let state = match r.u8()? {
                        0 => None,
                        _ => Some(l.states.get(&r.array::<16>()?)?.clone()),
                    };
                    let mut shaders = Vec::new();
                    for _ in 0..r.u8()? {
                        let stage = *STAGES.get(usize::from(r.u8()?))?;
                        shaders.push((stage, r.array::<8>()?));
                    }
                    if l.keys.insert(key) {
                        l.pipelines.push(Created {
                            key: to_hex(&key),
                            shaders,
                            state,
                        });
                    }
                }
                // A record of a later build: passed over.
                _ => {}
            }
            Some(())
        })();
        if taken.is_none() {
            break;
        }
        l.whole = bytes.len() - f.0.len();
    }
    Some(l)
}

/// The pipelines listed in `dir`, in the order the title first created
/// them; none without a list.
pub(crate) fn created(dir: &Path) -> Vec<Created> {
    let Ok(bytes) = std::fs::read(dir.join(FILE)) else {
        return Vec::new();
    };
    match listed(&bytes) {
        Some(l) => {
            if l.whole < bytes.len() {
                tracing::warn!(
                    whole = l.whole,
                    bytes = bytes.len(),
                    "the list of created pipelines ends in a torn record; read up to it"
                );
            }
            l.pipelines
        }
        None => {
            tracing::warn!("the list of created pipelines is not one; read as empty");
            Vec::new()
        }
    }
}

/// The list being added to.
struct List {
    path: PathBuf,
    pipelines: HashSet<[u8; 16]>,
    states: HashSet<[u8; 16]>,
    /// Records not in the file yet.
    unwritten: Vec<u8>,
    /// Pipelines added by this run.
    added: u64,
    /// A write failed: nothing more is kept.
    failed: bool,
}

impl List {
    /// Opens the list in `dir`, making or mending the file.
    fn open(dir: &Path) -> std::io::Result<List> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(FILE);
        let bytes = std::fs::read(&path).unwrap_or_default();
        let old = listed(&bytes);
        let mut list = List {
            path,
            pipelines: HashSet::new(),
            states: HashSet::new(),
            unwritten: Vec::new(),
            added: 0,
            failed: false,
        };
        match old {
            Some(l) => {
                if l.whole < bytes.len() {
                    // A run ended in the middle of a write.
                    let file = std::fs::OpenOptions::new().write(true).open(&list.path)?;
                    file.set_len(l.whole as u64)?;
                    tracing::warn!(
                        cut = bytes.len() - l.whole,
                        "the list of created pipelines ended in a torn record; cut off"
                    );
                }
                list.states = l.states.into_keys().collect();
                list.pipelines = l.keys;
            }
            None => {
                if !bytes.is_empty() {
                    tracing::warn!("the list of created pipelines is not one; started again");
                }
                std::fs::write(&list.path, MAGIC)?;
            }
        }
        Ok(list)
    }

    /// Adds a pipeline if it is not listed; whether it was new.
    fn add(&mut self, key: [u8; 16], shaders: &[(u8, [u8; 8])], state: Option<&[u8]>) -> bool {
        if self.failed || !self.pipelines.insert(key) {
            return false;
        }
        let mut record = key.to_vec();
        match state {
            Some(state) => {
                let hash = blake3::hash(state);
                let id: [u8; 16] = hash.as_bytes()[..16].try_into().expect("16 of 32 bytes");
                if self.states.insert(id) {
                    let mut s = id.to_vec();
                    s.extend_from_slice(state);
                    frame(STATE, &s, &mut self.unwritten);
                }
                record.push(1);
                record.extend_from_slice(&id);
            }
            None => record.push(0),
        }
        record.push(shaders.len() as u8);
        for (stage, hash) in shaders {
            record.push(*stage);
            record.extend_from_slice(hash);
        }
        frame(PIPELINE, &record, &mut self.unwritten);
        self.added += 1;
        true
    }

    /// Appends what is not in the file yet.
    fn write(&mut self) {
        if self.unwritten.is_empty() || self.failed {
            return;
        }
        let written = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.path)
            .and_then(|mut file| file.write_all(&self.unwritten));
        match written {
            Ok(()) => self.unwritten.clear(),
            Err(e) => {
                self.failed = true;
                self.unwritten = Vec::new();
                tracing::warn!(error = %e, "the list of created pipelines is not kept: it could not be written");
            }
        }
    }
}

/// `None` until the first pipeline; `Some(None)` if the list is off or
/// cannot be opened.
static LIST: Mutex<Option<Option<List>>> = Mutex::new(None);

/// `VICTORY_PIPELINE_LIST=0` keeps no list.
fn open(dir: &Path) -> Option<List> {
    if std::env::var_os("VICTORY_PIPELINE_LIST").is_some_and(|v| v == "0") {
        tracing::info!("no list of created pipelines is kept (VICTORY_PIPELINE_LIST=0)");
        return None;
    }
    match List::open(dir) {
        Ok(list) => {
            tracing::info!(
                pipelines = list.pipelines.len(),
                states = list.states.len(),
                file = %list.path.display(),
                "created pipelines listed by earlier runs (for `victory precompile`)"
            );
            // Runs are ended from outside: what is new is written every
            // two seconds, not at the end.
            let spawned = std::thread::Builder::new().name("victory-pipeline-list".into()).spawn(|| {
                let mut last = (0u64, std::time::Instant::now());
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    let added = write();
                    if added != last.0 && last.1.elapsed() >= std::time::Duration::from_secs(30) {
                        tracing::info!(added, "created pipelines new to the list so far");
                        last = (added, std::time::Instant::now());
                    }
                }
            });
            if let Err(e) = spawned {
                tracing::warn!(error = %e, "the list of created pipelines is written only at the end of the run");
            }
            Some(list)
        }
        Err(e) => {
            tracing::warn!(error = %e, "no list of created pipelines");
            None
        }
    }
}

/// The title created a pipeline with `key` from `shaders` and, if from a
/// stream, the state `state` ([`Recorded`]).
pub(crate) fn record(key: &str, shaders: &[Arc<Shader>], state: Option<&[u8]>) {
    // Without a pipeline cache (tests, a run without one) nothing is kept.
    let Some(dir) = super::library::dir() else { return };
    let mut list = LIST.lock().unwrap_or_else(|e| e.into_inner());
    let Some(list) = list.get_or_insert_with(|| open(dir)) else {
        return;
    };
    let Some(key) = hex::<16>(key) else { return };
    let named: Option<Vec<(u8, [u8; 8])>> = shaders
        .iter()
        .map(|s| Some((STAGES.iter().position(|n| *n == s.stage)? as u8, hex::<8>(&s.hash)?)))
        .collect();
    let Some(named) = named else { return };
    if named.len() > usize::from(u8::MAX) {
        return;
    }
    list.add(key, &named, state);
}

/// Writes what is new to the list's file; the pipelines this run added.
pub(crate) fn write() -> u64 {
    let mut list = LIST.lock().unwrap_or_else(|e| e.into_inner());
    match list.as_mut() {
        Some(Some(list)) => {
            list.write();
            list.added
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> [u8; 16] {
        [n; 16]
    }

    #[test]
    fn the_list_keeps_each_pipeline_once_across_runs() {
        let dir = tempfile::tempdir().unwrap();
        let state = [7u8; 40];
        {
            let mut list = List::open(dir.path()).unwrap();
            assert!(list.add(key(1), &[(7, [1; 8]), (1, [2; 8])], Some(&state)));
            assert!(!list.add(key(1), &[(7, [1; 8]), (1, [2; 8])], Some(&state)));
            assert!(list.add(key(2), &[(5, [3; 8])], None));
            list.write();
        }
        // The next run knows them, and adds only what is new.
        let mut list = List::open(dir.path()).unwrap();
        assert!(!list.add(key(2), &[(5, [3; 8])], None));
        assert!(list.add(key(3), &[(0, [4; 8]), (1, [2; 8])], Some(&state)));
        list.write();
        let all = created(dir.path());
        let keys: Vec<&str> = all.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, [to_hex(&key(1)), to_hex(&key(2)), to_hex(&key(3))]);
        assert_eq!(all[0].shaders, [("mesh", [1; 8]), ("pixel", [2; 8])]);
        assert!(all[0].meshes() && !all[0].computes());
        assert!(all[1].computes() && all[1].state.is_none());
        // One state, shared.
        assert!(Arc::ptr_eq(all[0].state.as_ref().unwrap(), all[2].state.as_ref().unwrap()));
        assert_eq!(all[0].state.as_ref().unwrap()[..], state);
    }

    #[test]
    fn a_torn_record_is_cut_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut list = List::open(dir.path()).unwrap();
        list.add(key(1), &[(5, [1; 8])], None);
        list.add(key(2), &[(5, [2; 8])], None);
        list.write();
        let path = dir.path().join(FILE);
        let whole = std::fs::read(&path).unwrap();
        // The run ended five bytes into its last write.
        std::fs::write(&path, &whole[..whole.len() - 5]).unwrap();
        assert_eq!(created(dir.path()).len(), 1);
        let mut list = List::open(dir.path()).unwrap();
        assert!(list.add(key(2), &[(5, [2; 8])], None));
        list.write();
        assert_eq!(std::fs::read(&path).unwrap(), whole);
        // A file of something else is started again.
        std::fs::write(&path, b"something else entirely").unwrap();
        assert!(created(dir.path()).is_empty());
        let mut list = List::open(dir.path()).unwrap();
        assert!(list.add(key(1), &[(5, [1; 8])], None));
        list.write();
        assert_eq!(created(dir.path()).len(), 1);
    }

    /// A stream's state read by the run and the same state replayed from
    /// the list describe themselves alike: the pipeline's key and its name
    /// in the library are made of that text.
    #[test]
    fn a_replayed_state_is_the_one_the_run_read() {
        let names = [c"POSITION", c"TEXCOORD"];
        let elements = [
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR(names[0].as_ptr().cast()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT(6),
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION(0),
                InstanceDataStepRate: 0,
            },
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR(names[1].as_ptr().cast()),
                SemanticIndex: 3,
                Format: DXGI_FORMAT(16),
                InputSlot: 2,
                AlignedByteOffset: u32::MAX,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION(1),
                InstanceDataStepRate: 4,
            },
        ];
        // A stream as the title builds it, with values that are no defaults.
        let layout: [(usize, u32); 9] = [
            (0x00, 8),
            (0x150, 9),
            (0x158, 10),
            (0x188, 21),
            (0x1c8, 17),
            (0x1d8, 12),
            (0x1f0, 14),
            (0x1f8, 15),
            (0x220, 16),
        ];
        let mut stream = vec![0u8; 0x228];
        for (i, b) in stream.iter_mut().enumerate() {
            *b = (i % 3) as u8;
        }
        let mut put = |at: usize, v: u64| stream[at..at + 8].copy_from_slice(&v.to_le_bytes());
        for (at, kind) in layout {
            put(at, u64::from(kind) | 1 << 32);
        }
        put(0x1e0, elements.as_ptr() as u64);
        put(0x1e8, elements.len() as u64);
        put(0x1f8, 15 | 28 << 32);
        put(0x218, 2 << 32);
        put(0x220, 16 | 40 << 32);
        let mut read = GraphicsState::default();
        let mut recorded = Recorded::default();
        for (at, kind) in layout {
            let (size, _) = super::super::subobject(kind).unwrap();
            // SAFETY: a whole subobject of this kind lies at `at`.
            unsafe { read.read(kind, stream.as_ptr().add(at)) };
            recorded.subobject(kind, &stream[at..at + size]);
        }
        let recorded = recorded.finish(&read);
        // The title's stream and its names are gone by the time of a replay.
        drop(stream);
        let replayed = replay(&recorded).expect("a state of ours");
        assert_eq!(replayed.describe(), read.describe());
        assert!(read.describe().contains("TEXCOORD"));
        assert_ne!(read.describe(), GraphicsState::default().describe());
        // A stream without state subobjects replays as the defaults.
        let none = Recorded::default().finish(&GraphicsState::default());
        assert_eq!(replay(&none).unwrap().describe(), GraphicsState::default().describe());
        // A damaged record is refused, not read past its end.
        assert!(replay(&recorded[..recorded.len() - 3]).is_none());
    }
}
