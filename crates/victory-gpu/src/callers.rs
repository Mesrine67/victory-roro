//! Which title code records the draws (`VICTORY_DRAW_CALLERS=1`,
//! diagnostics builds). Every draw-kind call of a command list notes the
//! return addresses above it; about every 20 seconds the stacks seen since
//! are logged, most frequent first, as `module+rva` (the title's image as
//! `title`, our own frames left out). The rvas are what a disassembler is
//! pointed at to find the title's submission loop and what decides, above
//! it, which meshes are drawn. Costs a stack walk per draw: for looking, not
//! for timing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{LazyLock, Mutex};

/// Return addresses kept per call (ours included, which take the first one
/// or two).
const DEPTH: usize = 14;
/// Stacks logged per report at most.
const SAID: usize = 150;

pub(crate) const DRAW: u8 = 0;
pub(crate) const INDEXED: u8 = 1;
pub(crate) const MESH: u8 = 2;
pub(crate) const MESH_X: u8 = 3;
pub(crate) const INDIRECT: u8 = 4;
pub(crate) const DISPATCH: u8 = 5;
const KINDS: [&str; 6] = ["draw", "indexed", "mesh", "mesh x", "indirect", "dispatch"];

fn on() -> bool {
    static ON: LazyLock<bool> =
        LazyLock::new(|| std::env::var_os("VICTORY_DRAW_CALLERS").is_some_and(|v| v == "1"));
    *ON
}

/// Calls and the sum of their amounts, by kind and stack.
type Seen = HashMap<(u8, [usize; DEPTH]), (u64, u64)>;
static SEEN: LazyLock<Mutex<Seen>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static FRAMES: AtomicU64 = AtomicU64::new(0);

/// A draw-kind call of `kind`; `amount` is what it asks for (vertices,
/// indices, mesh groups, the most commands of an indirect execution).
#[inline(never)]
pub(crate) fn note(kind: u8, amount: u32) {
    if !on() {
        return;
    }
    let mut frames = [std::ptr::null_mut::<core::ffi::c_void>(); DEPTH];
    // SAFETY: writes at most `DEPTH` addresses into `frames`.
    let n = unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::RtlCaptureStackBackTrace(
            1,
            DEPTH as u32,
            frames.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    };
    let mut key = [0usize; DEPTH];
    for (to, from) in key.iter_mut().zip(&frames[..usize::from(n)]) {
        *to = *from as usize;
    }
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let entry = seen.entry((kind, key)).or_default();
    entry.0 += 1;
    entry.1 += u64::from(amount);
}

/// A frame was presented: about every 20 seconds, logs what was noted since.
pub(crate) fn presented() {
    if !on() {
        return;
    }
    FRAMES.fetch_add(1, Relaxed);
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        match *last {
            Some(then) if now.duration_since(then).as_secs() < 20 => return,
            None => {
                *last = Some(now);
                return;
            }
            _ => *last = Some(now),
        }
    }
    let frames = FRAMES.swap(0, Relaxed).max(1) as f64;
    let seen = std::mem::take(&mut *SEEN.lock().unwrap_or_else(|e| e.into_inner()));
    let calls: u64 = seen.values().map(|v| v.0).sum();
    let mut stacks: Vec<_> = seen.into_iter().collect();
    stacks.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
    tracing::info!(
        "draw callers: {:.0} calls a frame over {frames:.0} frames, {} different stacks; the {} most frequent follow",
        calls as f64 / frames,
        stacks.len(),
        stacks.len().min(SAID)
    );
    let mut names: HashMap<usize, Option<String>> = HashMap::new();
    for ((kind, key), (n, amount)) in stacks.into_iter().take(SAID) {
        let named: Vec<String> = key
            .iter()
            .take_while(|a| **a != 0)
            .filter_map(|a| names.entry(*a).or_insert_with(|| name(*a)).clone())
            .collect();
        tracing::info!(
            "draw callers: {:.2} a frame, {} (amount {:.0} each) from {}",
            n as f64 / frames,
            KINDS[usize::from(kind)],
            amount as f64 / n as f64,
            named.join(" < ")
        );
    }
}

/// `module+rva` of a return address; `title+rva` in the title's image; none
/// for our own code.
fn name(address: usize) -> Option<String> {
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        GetModuleFileNameW, GetModuleHandleExW, GetModuleHandleW,
    };
    let mut module = std::ptr::null_mut();
    // SAFETY: looks up the module containing `address`; no reference taken.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            address as *const u16,
            &mut module,
        )
    };
    if found == 0 {
        return Some(format!("{address:#x}"));
    }
    // SAFETY: the process's own image.
    if module == unsafe { GetModuleHandleW(std::ptr::null()) } {
        return None;
    }
    let mut path = [0u16; 260];
    // SAFETY: `module` is a loaded module; the buffer length is passed.
    let len = unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) };
    let path = String::from_utf16_lossy(&path[..len as usize]);
    let file = path.rsplit('\\').next().unwrap_or(&path);
    let module_name = if file.to_ascii_lowercase().ends_with(".exe") { "title" } else { file };
    Some(format!("{module_name}+{:#x}", address - module as usize))
}
