//! The title's own `VirtualAlloc` and `VirtualFree` (it imports them from
//! `api-ms-win-core-memory`, beside `xmem.dll`): the host's, counted, and
//! with the page protections the console adds for speed taken out.
//!
//! On the console most title memory is write-combined: fast to stream
//! into, slow to read. `XMemVirtualAlloc` drops those bits already
//! (`xmem.rs`, `HOST_PROTECTION`); these two calls went straight to
//! Windows, unseen, so nothing said what the title asks of them.
//! `VICTORY_TITLE_CACHED=0` passes the protection on as the title gave it.
//!
//! Nothing here walks anything: a free of a whole allocation (size 0) is
//! the host's own call on that one allocation, and the GPU side keeps no
//! table that a free has to sweep. The counts are there to show it: how
//! many calls, how long Windows took, how many failed (memory opened in
//! place cannot be freed while its heap stands, `victory_gpu`'s `inplace`).
#![allow(non_snake_case)]
// Title code calls these through its import table, passing pointers the real
// API would trust.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_DECOMMIT, PAGE_READWRITE};
use windows_sys::core::BOOL;

use crate::{HleModule, hle_export};

const PAGE_NOCACHE: u32 = 0x200;
const PAGE_WRITECOMBINE: u32 = 0x400;
/// What makes reads of the memory slow.
const SLOW: u32 = PAGE_NOCACHE | PAGE_WRITECOMBINE;
/// The host's access values (one of them is set in a valid protection).
const ACCESS: u32 = 0xff;

/// Whether write-combining and uncached are taken out of what the title
/// asks (`VICTORY_TITLE_CACHED`, on unless 0).
fn cached() -> bool {
    static ON: LazyLock<bool> =
        LazyLock::new(|| std::env::var_os("VICTORY_TITLE_CACHED").is_none_or(|v| v != "0"));
    *ON
}

/// The protection handed to the host for the title's `protect`.
fn cleaned(protect: u32) -> u32 {
    if !cached() || protect & SLOW == 0 {
        return protect;
    }
    match protect & !SLOW {
        // Nothing but the speed bits: read-write, as the console gives.
        p if p & ACCESS == 0 => p | PAGE_READWRITE,
        p => p,
    }
}

// Since the last line: allocations, bytes they committed, microseconds in
// the host, those asking for slow pages and their bytes; frees, of them
// whole allocations (size 0), decommits, failed ones, microseconds.
const ALLOCS: usize = 0;
const COMMITTED: usize = 1;
const ALLOC_US: usize = 2;
const SLOW_ASKED: usize = 3;
const SLOW_BYTES: usize = 4;
const FREES: usize = 5;
const WHOLE: usize = 6;
const DECOMMITS: usize = 7;
const FAILED: usize = 8;
const FREE_US: usize = 9;
static COUNTS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];

/// Every (kind, protection) the title asked for since the start: calls and
/// bytes. A handful of entries; the lock is held for one lookup.
static ASKED: LazyLock<Mutex<HashMap<(u32, u32), (u64, u64)>>> = LazyLock::new(Default::default);

static STARTED: LazyLock<Instant> = LazyLock::new(Instant::now);

pub extern "system" fn VirtualAlloc(
    address: *const c_void,
    size: usize,
    kind: u32,
    protect: u32,
) -> *mut c_void {
    let started = Instant::now();
    // SAFETY: the title's own arguments, the protection cleaned.
    let at = unsafe {
        windows_sys::Win32::System::Memory::VirtualAlloc(address, size, kind, cleaned(protect))
    };
    // Counters only from here on: the title reads the host's last error.
    COUNTS[ALLOC_US].fetch_add(started.elapsed().as_micros() as u64, Relaxed);
    COUNTS[ALLOCS].fetch_add(1, Relaxed);
    if kind & MEM_COMMIT != 0 {
        COUNTS[COMMITTED].fetch_add(size as u64, Relaxed);
    }
    if protect & SLOW != 0 {
        COUNTS[SLOW_ASKED].fetch_add(1, Relaxed);
        COUNTS[SLOW_BYTES].fetch_add(size as u64, Relaxed);
    }
    {
        let mut asked = ASKED.lock().unwrap_or_else(|e| e.into_inner());
        let entry = asked.entry((kind, protect)).or_default();
        entry.0 += 1;
        entry.1 += size as u64;
    }
    say();
    at
}

pub extern "system" fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> BOOL {
    let started = Instant::now();
    // SAFETY: the title's own arguments. Size 0 names the whole allocation
    // at `address`, which the host resolves itself.
    let freed = unsafe { windows_sys::Win32::System::Memory::VirtualFree(address, size, kind) };
    COUNTS[FREE_US].fetch_add(started.elapsed().as_micros() as u64, Relaxed);
    COUNTS[FREES].fetch_add(1, Relaxed);
    if size == 0 {
        COUNTS[WHOLE].fetch_add(1, Relaxed);
    }
    if kind & MEM_DECOMMIT != 0 {
        COUNTS[DECOMMITS].fetch_add(1, Relaxed);
    }
    if freed == 0 {
        COUNTS[FAILED].fetch_add(1, Relaxed);
    }
    say();
    freed
}

/// The line of what the title asked, every ten seconds in which it asked
/// anything.
fn say() {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = STARTED.elapsed().as_millis() as u64;
    let last = LAST.load(Relaxed);
    if now < last + 10_000 || LAST.compare_exchange(last, now, Relaxed, Relaxed).is_err() {
        return;
    }
    // SAFETY: plain thread state; writing the log may change it, and the
    // title reads it after the call this runs in.
    let error = unsafe { GetLastError() };
    let n: [u64; 10] = std::array::from_fn(|i| COUNTS[i].swap(0, Relaxed));
    let mut asked: Vec<((u32, u32), (u64, u64))> = {
        let asked = ASKED.lock().unwrap_or_else(|e| e.into_inner());
        asked.iter().map(|(k, v)| (*k, *v)).collect()
    };
    asked.sort_unstable_by_key(|(_, (_, bytes))| std::cmp::Reverse(*bytes));
    let kinds: Vec<String> = asked
        .iter()
        .take(8)
        .map(|((kind, protect), (calls, bytes))| {
            format!("kind {kind:#x} protection {protect:#x}: {calls} calls {} MB", bytes >> 20)
        })
        .collect();
    tracing::info!(
        "the title's own VirtualAlloc and VirtualFree in the last {:.1} s: {} allocations, {} MB committed, {:.2} ms in Windows, \
         {} of them asking for write-combined or uncached pages ({} MB, {}); {} frees ({} of whole allocations by size 0, \
         {} decommits, {} failed), {:.2} ms in Windows; asked since the start, {} kinds: {}",
        (now - last) as f64 / 1000.0,
        n[ALLOCS],
        n[COMMITTED] >> 20,
        n[ALLOC_US] as f64 / 1000.0,
        n[SLOW_ASKED],
        n[SLOW_BYTES] >> 20,
        if cached() { "given ordinary cached pages" } else { "passed on as asked, VICTORY_TITLE_CACHED=0" },
        n[FREES],
        n[WHOLE],
        n[DECOMMITS],
        n[FAILED],
        n[FREE_US] as f64 / 1000.0,
        asked.len(),
        kinds.join("; ")
    );
    // SAFETY: as above.
    unsafe { SetLastError(error) };
}

pub(crate) fn module() -> HleModule {
    HleModule {
        name: "vmem",
        dlls: &["api-ms-win-core-memory-*", "kernel32.dll", "kernelbase.dll"],
        exports: vec![hle_export!(VirtualAlloc), hle_export!(VirtualFree)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{
        MEM_RELEASE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_NOACCESS, VirtualQuery,
    };

    fn protection(at: *const c_void) -> u32 {
        // SAFETY: plain data filled in by the query.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out pointer of the right size.
        assert_ne!(unsafe { VirtualQuery(at, &mut info, size_of_val(&info)) }, 0);
        info.Protect
    }

    #[test]
    fn slow_pages_become_ordinary_ones() {
        assert_eq!(cleaned(PAGE_READWRITE | PAGE_WRITECOMBINE), PAGE_READWRITE);
        assert_eq!(cleaned(PAGE_READWRITE | PAGE_NOCACHE), PAGE_READWRITE);
        assert_eq!(cleaned(PAGE_WRITECOMBINE), PAGE_READWRITE);
        assert_eq!(cleaned(PAGE_NOACCESS), PAGE_NOACCESS);
        assert_eq!(cleaned(PAGE_READWRITE), PAGE_READWRITE);

        let kind = MEM_RESERVE | MEM_COMMIT;
        let p = VirtualAlloc(std::ptr::null(), 1 << 16, kind, PAGE_READWRITE | PAGE_WRITECOMBINE);
        assert!(!p.is_null());
        assert_eq!(protection(p), PAGE_READWRITE);
        // SAFETY: committed read-write.
        unsafe { p.cast::<u8>().write(1) };
        // Size 0: the whole allocation, and only that one.
        let q = VirtualAlloc(std::ptr::null(), 1 << 16, kind, PAGE_READWRITE);
        assert_ne!(VirtualFree(p, 0, MEM_RELEASE), 0);
        assert_eq!(protection(q), PAGE_READWRITE);
        assert_ne!(VirtualFree(q, 0, MEM_RELEASE), 0);
        // A second free fails, and the host's error is the title's to read.
        assert_eq!(VirtualFree(q, 0, MEM_RELEASE), 0);
        // SAFETY: plain thread state.
        assert_ne!(unsafe { GetLastError() }, 0);
    }
}
