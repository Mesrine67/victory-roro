//! The title's upscaler, stood in for.
//!
//! As the smaller console the title renders at a low resolution and has
//! AMD's upscaler (`amd_fidelityfx_upscaler_xbox.dll`, FSR 4) make the
//! full-size picture. Its passes are console-compiled compute programs in
//! a container part the translator does not read (`ACCC`, magic `FSPX`):
//! none of them runs here, the output stays as it was made and the picture
//! is black (run `ss_try4`).
//!
//! The title's loader library looks the upscaler's `ffxDispatch` up by
//! name; the runtime hands it [`dispatch`] instead ([`hook`]). An upscale
//! dispatch is then one pass of ours that draws the colour input over the
//! output with linear filtering (`crate::list::upscale`); the upscaler's
//! own passes are not recorded at all. Every other dispatch, and an
//! upscale we cannot make sense of, goes to the upscaler as before.
//!
//! `VICTORY_UPSCALE=title`: nothing is hooked.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

/// The upscaler's own `ffxDispatch`.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// Dispatches seen; upscales drawn by us; upscales handed on.
static CALLS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

/// `ffxDispatch(context, description)`.
type Dispatch = extern "system" fn(*mut c_void, *const u8) -> u32;

/// The description type of an upscale dispatch (`ffx_upscale.h`).
const UPSCALE: u64 = 0x0001_0001;

/// What the title's lookup of the upscaler's `ffxDispatch` gets: ours, which
/// keeps `original` for what it hands on.
pub fn hook(original: usize) -> usize {
    if std::env::var_os("VICTORY_UPSCALE").is_some_and(|v| v == "title") {
        return original;
    }
    ORIGINAL.store(original, Relaxed);
    tracing::info!(
        original = format_args!("{original:#x}"),
        "the title's upscaler: its dispatch goes through ours (VICTORY_UPSCALE=title: left alone)"
    );
    dispatch as *const () as usize
}

fn hand_on(context: *mut c_void, description: *const u8) -> u32 {
    match ORIGINAL.load(Relaxed) {
        // (an error code of the interface: no backend)
        0 => 1,
        // SAFETY: the address the upscaler exports its dispatch at.
        f => {
            let f = unsafe { std::mem::transmute::<usize, Dispatch>(f) };
            f(context, description)
        }
    }
}

extern "system" fn dispatch(context: *mut c_void, description: *const u8) -> u32 {
    if description.is_null() {
        return hand_on(context, description);
    }
    // SAFETY: the title passes a description, which begins with its type.
    let kind = unsafe { description.cast::<u64>().read_unaligned() };
    let call = CALLS[0].fetch_add(1, Relaxed);
    if kind != UPSCALE {
        if call < 8 {
            tracing::info!(kind = format_args!("{kind:#x}"), "the title's upscaler: a dispatch that is no upscale, handed on");
        }
        return hand_on(context, description);
    }
    // An upscale description (`ffxDispatchDescUpscale`): header 16 bytes,
    // the list, seven resources of 48 bytes (the pointer first), then the
    // jitter and motion scale, the render size and the upscale size.
    const LIST: usize = 16;
    const COLOUR: usize = 24;
    const OUTPUT: usize = 24 + 6 * 48;
    const RENDER: usize = OUTPUT + 48 + 16;
    const LEN: usize = RENDER + 16;
    // SAFETY: an upscale description is longer than this.
    let bytes = unsafe { std::slice::from_raw_parts(description, LEN) };
    let word = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let pointer = |at: usize| (u64::from(word(at)) | u64::from(word(at + 4)) << 32) as usize;
    let (list, colour, output) = (pointer(LIST), pointer(COLOUR), pointer(OUTPUT));
    let render = [word(RENDER), word(RENDER + 4)];
    let size = [word(RENDER + 8), word(RENDER + 12)];
    if call < 2 {
        let words: Vec<String> = (0..LEN / 4).map(|i| format!("{:x}", word(4 * i))).collect();
        tracing::info!(
            list = format_args!("{list:#x}"),
            colour = format_args!("{colour:#x}"),
            output = format_args!("{output:#x}"),
            ?render,
            ?size,
            words = words.join(" "),
            "the title's upscaler: an upscale dispatch"
        );
    }
    let sane = |d: [u32; 2]| (1..=16384).contains(&d[0]) && (1..=16384).contains(&d[1]);
    let drawn = list != 0
        && colour != 0
        && output != 0
        && sane(render)
        && sane(size)
        && crate::list::upscale(list as crate::object::This, colour as crate::object::This, output as crate::object::This, render, size);
    if drawn {
        CALLS[1].fetch_add(1, Relaxed);
        // (the interface's "ok")
        return 0;
    }
    if CALLS[2].fetch_add(1, Relaxed) < 4 {
        tracing::warn!(?render, ?size, "the title's upscaler: an upscale we could not draw ourselves, handed on");
    }
    hand_on(context, description)
}
