# 11. Observability: the instruments we built

Every instrument below is an **observer**. Each one's header states the same contract:
- it changes no decision, call, order, copy, fence or wait;
- it uses no lock, allocation or TLS on hot paths;
- it preserves `GetLastError`;
- when disabled, it costs one relaxed atomic load.

A profiler that perturbs the hitch it measures is useless. Ours did, several times, before we enforced this.

| Instrument (switch, as an example name) | What it measures | Cost and lessons |
|---|---|---|
| **Frame split** (`XBOX_EXACT_FRAME_SPLIT`) | Cuts each presenter frame into exclusive parts: title run, hooked waits, pipeline creation, trap faults, and the barrier's harvest, drain, enumerate, classify, wait, stage, submit and "rest". Attributes each slow frame's excess over the 16.7/33.4/50 ms lines to parts. | Exclusive parts that sum exactly matter more than precision per part. An unbracketed remainder is reported, never hidden. |
| **Stall split** (`XBOX_EXACT_STALL_SPLIT`) | Presenter CPU against blocked time, lock-acquire waits on each named loader lock, barrier need classes (used, stale, no job, stolen, missing), joins. Binary side file plus summary rows. | About 0.05 ms per frame on the desktop, 0.3-0.5 ms of CPU on the handheld. Its CPU split relies on `GetThreadTimes`, which ticks at 10 ms under Wine, so use it for totals only. |
| **Hook timing** (`XBOX_TEX_DRAW_EXACT_HOOK_TIMING`, test builds only) | Samples 1 in 64 calls of each draw-path hook. | No measurable cost. Showed that no single hook is the hitch (section 7.6). |
| **Create split** | Times every hooked create/alloc entry point as whole hook, native part and wrapper part, outermost call per thread only. | 0.34 → 0.39 ms per frame of presenter CPU. Found that our wrappers, not the kernel, own most VM hook time (section 2.6). |
| **Lock slow / lock census** | Per lock, per call site, per thread role: hold and wait times. | **Lesson:** the first version used a per-event ring (256 entries, with the presenter scanning 128 slots × 256 events per frame). It overwrote history and *doubled* slow frames (166 against 86-91 a minute). It also cost the handheld 6 fps. The second version keeps fixed per-thread counters only: added lock-pair cost 65.7 → 25.0 ns. |
| **Wait split** (`XBOX_EXACT_WAIT_SPLIT`) | An activity word for the presenting thread, plus a per-thread record of every hooked title wait, sleep, yield, I/O completion wait and VM call; a 1 kHz sampler thread reads the activity word. | The activity word and timed waits are valid. **Its in-process `/proc` reads are not valid under Wine** (section 9.8). The sampler costs about 11 % of one core. |
| **Linux-side sampler** | An external script that reads `/proc/<pid>/task/<tid>/{stat,schedstat,wchan,syscall}` for every thread at about 1 kHz and joins the result to the loader's frame marks. | About 8 % of one core. Became the standard for CPU-against-blocked splits on Linux. |
| **Lock owner** (`XBOX_EXACT_LOCK_OWNER`) | On a contended title critical section: the owning thread, and whether it is inside one of our hooks. | Built to settle the burst lock question (section 9.7). |
| **Lead histograms** (`XBOX_TEX_DRAW_EXACT_MEASURE`) | For each need, how long before it the bytes became final and the job was queued, by cause. | This one ended the "ordering" theory (section 5.5). |
| **Thread CPU table** | Helper threads register themselves; per-thread CPU every 5 s. | Cheap, and catches runaway workers. |
| **Memory meta rows** | Private bytes, working set, peak, DXGI local usage and budget, plus the cost of the query itself. | Replaces ad-hoc queries; never adds a second reader. |
| **Object ownership census** (`XBOX_MEMORY_OWNERSHIP`) | Live native objects by kind (committed, placed, heap, descriptor, pipeline, upload, title-committed, title-imported). | Uses a COM private-data sentinel as a token, so no vtable hooks are needed. |
| **Switch census** | At startup, the effective value and source of every switch: default, profile or environment. | Removes "which build/config was this?" from every investigation. |
| **Trap attribution** (observation only) | Walks the faulting thread's call chain, outside any lock, to say which module family wrote trapped pages. | Useful once per title to learn its writer classes. |
| **Debug fills** (`XBOX_TEX_UNRESIDENT_FILL`) | Fills unuploaded data with magenta checkers at registration time. | Not timing-neutral. Cannot cover render targets, depth or UAVs. Undercounts if scored by pixel percentage. |
| **GPU timestamp layer** | Section 12.1. | |

**Habits that made these useful.**
- Every run prints a **health block**: trap counters, breaches, lease refusals, device-loss checks and the switch census. Performance numbers from an unhealthy run are discarded.
- Rows are **machine-readable** and versioned (`schema=1`). Reader scripts are kept with the data.
- Diagnostics that cost real time are **test-build only**. Play builds compile them out, so they cannot leak into players' frame time.

---
[Back to the index](../README.md)
