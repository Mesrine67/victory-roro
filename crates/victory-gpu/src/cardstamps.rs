//! Diagnostics (`VICTORY_CARD_STAMPS=1`): how long the passes of ours over
//! copies in the card's memory take on the card itself, by the card's own
//! timestamps. The per-command timing (`VICTORY_GPU_TIMING`) names the
//! title's commands only; what it leaves over was "the layer's own lists and
//! the waits themselves", some 60 ms of the card's 150 busy ms a frame in the
//! room, and nothing said which of ours it was: the bytes sent up before a
//! submission, the hash passes after them, the compare after the lists, or
//! the fetch of the pieces that differ.
//!
//! A list of ours writes a run of timestamps into one query heap shared by
//! all of them (each list its own slots) and resolves them into one
//! read-back buffer; the frame line sums the intervals by kind once the
//! lists have surely run. A compare's run can name the last timestamp of
//! the same submission's "sent up" list: the time between the two is the
//! title's own lists of that submission.
//!
//! Timing changes the work a little: with the switch on, the lists end in a
//! wait for every earlier shader write, so that the last timestamp is taken
//! after the passes have run and not when the card has merely read them.

use crate::host::Barred as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;

/// What an interval between two timestamps of a run was spent on.
pub(crate) const NONE: u8 = 0;
/// Bytes of title buffers' copies going from PC memory to the card's.
pub(crate) const SENT_UP: u8 = 1;
/// The hash passes over the pieces just sent up.
pub(crate) const REHASH: u8 = 2;
/// The compare of the declared ranges after the submission's lists.
pub(crate) const COMPARE: u8 = 3;
/// The pieces that differ coming down to PC memory.
pub(crate) const FETCH: u8 = 4;
/// The submission's own lists (the title's), between our two lists.
pub(crate) const TITLE: u8 = 5;
pub(crate) const KINDS: usize = 6;

/// Timestamps kept: at some 600 a frame, several of the frame line's
/// intervals of five seconds.
const SLOTS: u32 = 1 << 17;

struct Stamps {
    heap: ID3D12QueryHeap,
    back: ID3D12Resource,
    /// The read-back buffer's mapping.
    data: usize,
}

// SAFETY: D3D12 objects are free-threaded; the mapping is read only.
unsafe impl Send for Stamps {}
// SAFETY: as above.
unsafe impl Sync for Stamps {}

static STAMPS: OnceLock<Option<Stamps>> = OnceLock::new();
static NEXT: AtomicU32 = AtomicU32::new(0);

/// A run whose list went to the card: its first slot, the kind of the
/// interval that ends at each slot (the first: [`NONE`], or [`TITLE`] with
/// the slot it is counted from), and when it was recorded.
struct Sent {
    first: u32,
    kinds: Vec<u8>,
    link: Option<u32>,
    when: std::time::Instant,
}

static SENT: Mutex<Vec<Sent>> = Mutex::new(Vec::new());

/// Whether the passes are timed (`VICTORY_CARD_STAMPS=1` in a diagnostics
/// build; off by default).
pub(crate) fn on() -> bool {
    #[cfg(feature = "diagnostics")]
    {
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("VICTORY_CARD_STAMPS").is_some_and(|v| v == "1"))
    }
    #[cfg(not(feature = "diagnostics"))]
    {
        false
    }
}

fn stamps(device: &ID3D12Device) -> Option<&'static Stamps> {
    STAMPS
        .get_or_init(|| {
            let mut heap: Option<ID3D12QueryHeap> = None;
            let mut back: Option<ID3D12Resource> = None;
            let readback = D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_READBACK, ..Default::default() };
            let desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Width: u64::from(SLOTS) * 8,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                ..Default::default()
            };
            let mut data = std::ptr::null_mut();
            // SAFETY: plain object creation; a read-back buffer stays mapped.
            unsafe {
                device
                    .CreateQueryHeap(
                        &D3D12_QUERY_HEAP_DESC { Type: D3D12_QUERY_HEAP_TYPE_TIMESTAMP, Count: SLOTS, NodeMask: 0 },
                        &mut heap,
                    )
                    .ok()?;
                device
                    .CreateCommittedResource(
                        &readback,
                        D3D12_HEAP_FLAG_NONE,
                        &desc,
                        D3D12_RESOURCE_STATE_COPY_DEST,
                        None,
                        &mut back,
                    )
                    .ok()?;
                back.as_ref()?.Map(0, None, Some(&mut data)).ok()?;
            }
            tracing::info!(slots = SLOTS, "the card's timestamps around our passes over copies in its memory: on (VICTORY_CARD_STAMPS=1)");
            Some(Stamps {
                heap: heap?,
                back: back?,
                data: data as usize,
            })
        })
        .as_ref()
}

/// A run of timestamps being recorded into one list of ours.
pub(crate) struct Run {
    stamps: &'static Stamps,
    first: u32,
    most: u32,
    kinds: Vec<u8>,
    link: Option<u32>,
}

impl Run {
    /// Takes slots for at most `most` timestamps of one list. `None` with
    /// the switch off, or without the heap.
    pub(crate) fn begin(device: &ID3D12Device, most: u32) -> Option<Run> {
        if !on() || most == 0 || most > SLOTS {
            return None;
        }
        let stamps = stamps(device)?;
        let first = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |at| {
                Some(if at + most > SLOTS { most } else { at + most })
            })
            .map_or(0, |at| if at + most > SLOTS { 0 } else { at });
        Some(Run {
            stamps,
            first,
            most,
            kinds: Vec::with_capacity(most as usize),
            link: None,
        })
    }

    /// The run's first timestamp ends an interval of [`TITLE`] that began at
    /// slot `from` (the last timestamp of the list sent before the
    /// submission's own lists).
    pub(crate) fn after(&mut self, from: Option<u32>) {
        self.link = from;
    }

    /// A timestamp here: the interval since the one before was `kind`.
    pub(crate) fn mark(&mut self, list: &ID3D12GraphicsCommandList, kind: u8) {
        let at = self.kinds.len() as u32;
        if at >= self.most {
            return;
        }
        // SAFETY: an open list of ours and our heap; the slot is this run's.
        unsafe { list.EndQuery(&self.stamps.heap, D3D12_QUERY_TYPE_TIMESTAMP, self.first + at) };
        self.kinds.push(kind);
    }

    /// The list is about to be closed: its timestamps go to the read-back
    /// buffer when it has run. Returns the slot of the last one.
    pub(crate) fn end(self, list: &ID3D12GraphicsCommandList) -> Option<u32> {
        let count = self.kinds.len() as u32;
        if count == 0 {
            return None;
        }
        // SAFETY: an open list of ours; the slots were written above, in
        // this list; the buffer holds `SLOTS` of them.
        unsafe {
            list.ResolveQueryData(
                &self.stamps.heap,
                D3D12_QUERY_TYPE_TIMESTAMP,
                self.first,
                count,
                &self.stamps.back,
                u64::from(self.first) * 8,
            );
        }
        let last = self.first + count - 1;
        SENT.lock().unwrap_or_else(|e| e.into_inner()).push(Sent {
            first: self.first,
            kinds: self.kinds,
            link: self.link,
            when: std::time::Instant::now(),
        });
        Some(last)
    }
}

/// A wait for every earlier shader write, for the end of a timed list.
pub(crate) fn wait_for_writes(list: &ID3D12GraphicsCommandList) {
    // SAFETY: an open list; a barrier that names no resource.
    unsafe {
        list.Barred(&[D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                UAV: std::mem::ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER {
                    pResource: std::mem::ManuallyDrop::new(None),
                }),
            },
        }]);
    }
}

/// For the frame line: the runs recorded `settled` or longer ago (their
/// lists have run), summed by kind as (intervals, ticks of the card's
/// clock), and how many runs that was. Intervals whose timestamps read
/// zero or run backwards are left out.
pub(crate) fn take(settled: std::time::Duration) -> ([(u64, u64); KINDS], u64) {
    let mut sums = [(0u64, 0u64); KINDS];
    let Some(stamps) = STAMPS.get().and_then(Option::as_ref) else {
        return (sums, 0);
    };
    let due: Vec<Sent> = {
        let mut sent = SENT.lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        let (due, later): (Vec<Sent>, Vec<Sent>) = sent.drain(..).partition(|s| now.duration_since(s.when) >= settled);
        *sent = later;
        due
    };
    // SAFETY: the mapping of the read-back buffer, `SLOTS` timestamps.
    let read = |slot: u32| unsafe { (stamps.data as *const u64).add(slot as usize).read_volatile() };
    let runs = due.len() as u64;
    for run in due {
        let mut before = run.link.map(read);
        for (i, &kind) in run.kinds.iter().enumerate() {
            let at = read(run.first + i as u32);
            if let Some(from) = before
                && kind != NONE
                && from != 0
                && at >= from
            {
                let sum = &mut sums[usize::from(kind).min(KINDS - 1)];
                sum.0 += 1;
                sum.1 += at - from;
            }
            before = Some(at);
        }
    }
    (sums, runs)
}
