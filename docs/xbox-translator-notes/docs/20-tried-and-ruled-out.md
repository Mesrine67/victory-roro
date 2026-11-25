# Tried and ruled out: the experiment ledger

This is the condensed version of the "tried / ruled out" tables we kept for each problem. Every row was an A/B run against a control, with two passes or more where marked. The columns are:
- **what we changed**;
- **the result**, with numbers where we had them;
- **what it rules out**. This is the column that saved the most time later.

Use it to skip our dead ends, or to retry one deliberately with a reason why your case differs. The switch names are ours. They are listed in [Appendix A](A-switches.md).

Machines are described as in the main text: an AMD desktop (Linux/Proton), an AMD handheld at 15 W (SteamOS/Proton), an RTX 4090 desktop (Windows), an RTX 3080 laptop (Windows) and a Ryzen 9 5900X desktop with an RX 6700 XT.

---

## 1. Black pop-in / late textures (before the exact path)
Background: [06-black-pop-in-history.md](06-black-pop-in-history.md). Most of these ran when textures were noticed by polling. They show what does *not* fix "data noticed late".

| Attempt | Result | Rules out |
|---|---|---|
| Shorter re-check slice per present (4 → 2 ms) | Polling cost went down; no pop-in measurement. | Nothing. |
| Ask the title for more texture data (higher detail setting) | Same black surfaces and backlog; slower. | More title demand as a cure. |
| Re-check partially uploaded tails | 65 repairs; opening road still black. | Premature-upload repair as the cure. |
| Scale snapshot copy size x1-x32; CPU copy cap 1 vs 8 ms | Backlog drains earlier at x8; still black. | Copy size/caps alone. |
| GPU detiling instead of CPU | Driver CPU 49.96 s → 1.12 s per run; still black. | CPU detile cost as the sole cause. |
| OS write-watch dirty detection plus a GPU texture pool | Road 96-100 % black on laps 2-3; pool exhausted at 80 s. | Dirty detection alone; that pool design. |
| "First lane" priority for early draws | Still black; one device loss on the laptop. | Lane priority as a cure. |
| First lane plus fair queues | Ready-to-landed p95 29.0 s → 2.34 s; 1 % lows -46 %; wall panel still black. | Latency alone as a cure. |
| Queue fixes so nothing starves | 0 live jobs in the pipeline, yet 19,750 of 94,696 viewed texture generations never got mip 0. | Queue starvation. |
| Wake service and admission tuning | Enqueue p95 266 ms, service 512 ms; still black. | Wake/admission tuning as a cure. |
| Asynchronous copy-engine signalling; queue-depth caps | Signalling fix failed; the cap never bound. | Frame depth and async signalling. |
| Road-class top-up | Lap 1 road still 92.5 % black. | Class-specific top-up. |
| Top-up floor (keep a minimum mip) | Opponents' cars near-black; stays off. | Floors. |
| **Remove uploader caps** | **Worked for lap 1**: 981 vs 231 uploads/s; road lit from 0:10 vs 1:18. Short pops remained. | Upload caps as the lap-1 cause (fixed). |
| Demand pull: inline copy before the first draw (64 MiB budget) | Hitches over 250 ms 2 → 115; 6-8 fps. | Unbounded inline work on the draw path. |
| Priority FIFO | Black episodes 1 → 1; laptop hitches over 250 ms 59 → 69. | Reordering the job stage. |
| Re-check slice 4 ms | Revisits x1.82; opening pass faster; fewer pops not proven. | Nothing (supports the mechanism). |
| Clamp the view's most-detailed mip to landed data | Black pops of 241 ms or more survive; clamp acted on about 12-17 % of decisions, because most black samples had *nothing* resident. | Partial-only clamps. |
| Cancel obsolete texture jobs | Fixture passes; no game effect. | Nothing about pop-in. |
| Warm vs cold pipeline cache | Same black in both arms. | Cold shader compile as the sole cause. |
| Demand-first ordering | Not measurable; hitches over 50 ms rose. | Another ordering-only variant. |
| More detile workers | Pending 1,300-2,750 with only 0-9 live jobs; workers idle. | Untile/worker saturation as the limit. |
| Larger drain per present (32 jobs, 6 ms) | Small effect (about 2 % on a proxy). Later, 64 jobs per drain: landings over 1 s 10.7 % → 0.1-0.2 %, peak pending 10.6k → 2.0k, no fps cost. | The drain cap as the *main* limit. |
| Event-driven re-check on change | Small effect (about 1-2 % on a proxy); 0 hitches over 250 ms. | Wake-on-change as *the* fix. |
| Readability map instead of `VirtualQuery` (sibling title) | Scan cost 69 s → 0.12 s per run; texture visits 0.1 M → 3.6 M; fps unchanged. | Range-scoped invalidation; epoch churn. |
| Cheaper readability walk | Small effect; `VirtualQuery` calls 1.29 per visit. | The per-page walk as the main limit. |
| Cached (not write-combined) title memory | Scan 52 → 4,800 MB/s; top-ups -28 %; on the 4090, hitches about 27 → about 1 per minute. Black unchanged to the eye. | Slow memory reads as the pop-in cause (but a real CPU win: kept). |
| **Settle wake** (deadline wake for waiting textures) | **Black road per lap 6.5/6.3/5.3 s → 0/0/0.27 s**; demand-to-fence p50 0.63 s → 0.10 s; cost about 0.5 ms/frame and 40-60 MiB VRAM. Short flashes remained. | Re-check starvation (fixed). |
| Recreated-texture reuse (speculative GPU seed) | 8,192-slot table fills; most lookups miss; landings over 1 s rose 0.13 % → 2 %. | Speculative reuse. |
| Coarse-first prototype | Split 1 texture, fell back 129 times. | Coarse-first. |
| Submit-time first-use preparation v1-v4 | v3 found unbounded descriptor tables (40.6 M unresolved); v4 had 1.10 M violations and 2.6x the hitches over 50 ms. | Submit-time preparation from descriptor creation. |
| Upload sync (every upload lands before the next title submission) | Scene-cut black not removed. | Late landing of *admitted* uploads. |
| Placeholder / neutral fill | No faithful value exists. | Placeholders (by design). |
| **Pixel ownership tint** of the road draw | Road draw solid every frame. | Missing or culled draws. |
| **Magenta fill** of unuploaded data | Magenta in 69 of 73 marked moments. | Everything except "data not on the GPU yet". |

## 2. Making the exact path fast
| Attempt | Result | Rules out / status |
|---|---|---|
| Write-watch harvest per submission | 76 M `GetWriteWatch` calls per 600 s run; 41-56k calls and about 63 ms per present on the laptop. | Write-watch as the primary detector at this scale. |
| Page traps with a fixed event ring | 127,391 overflows (each forced a full re-check); 1-5 fps; world vanished on a straight. | Overflow-means-recheck-all. |
| Traps with O(changes) drain plus batched harvest | Barrier 203 → 7 ms, drain 182 → 0.3 ms, harvest 65 → 0.3 ms per present; about 2 → 16 fps. | Kept. |
| Per-span inline copies at level load | 84,058 spans (65 MB) one by one: a 12 s frame. | Per-span overhead; batch per texture and fan out. |
| Batching, pooled upload memory, fan-out | Level-load window 435 ms, largest present 216 ms (was 11.7 s). | Kept. |
| Unbounded upload batches | 4,470 and 53,405 members per submit; hangs and a device loss. | Uncapped batches (cap at 64). |
| Test-build-only per-span log row under the barrier locks | 85,504 rows in one present: a 12-13 s freeze. | Logging under locks. |
| Whole-heap reach walk | 5,235 µs per submission. | |
| Per-heap reach table | 324 µs. | |
| Per-texture unknown-slot index | 14.2 µs mean (limit 30). | Kept. |
| Tree-map descriptor bookkeeping under one mutex | 4.2-4.5 ms per present on the title thread. | |
| Flat descriptor bookkeeping | 0.47 ms; side-effect-only cost 0.67 → 0.97 of control; shadow compare 0 mismatches. | Kept. |
| Bundle reach by cached ranges | 13.5 M cache misses, 5 hits per run. | Caching per bundle buffer. |
| In-place bundle reach (replay metadata setters only) | -1.5 to -3.5 ms per frame; 14.18 M differential checks, 0 mismatches. | Kept. |
| Size-zero free read as "all memory" | 8,648-13,051 whole-registry holds per run; submissions waited 138-243 s. | Fixed: resolve the real range. |
| Readability cache | 27,830 stale entries in a long run (a birth race). | Fixed; 0 since. |
| Snapshot read without a mapping lease | "Snapshot read" breaches in every run. | Fixed by striped leases. |
| System SHA-256 under Wine | About 300 ms CPU per second. | Own SHA-NI implementation, bit-identical, self-tested. |
| Scratch `VirtualAlloc`/`Free` per job under Wine | About 2.5 ms CPU per frame on two workers. | Snapshot pool. |
| Console tiling library detile | 65 µs per 21 KiB call (3.2 ms/MiB) on the handheld. | Learned run tables, verified per class (about 10x). |
| Tight small-buffer alignment | Private memory -33 % (15.3 → 10.2 GiB). | Kept. |
| Bundle table never pruned | 62,475 entries, 55 % dead; sweep -13-15 % private bytes. | Kept (sweep). |
| Arm every mip separately | 24,453 re-arms cost 132 ms at a track load. | Arm merge (kept). |
| Synthetic busy-wait per submission (exact off) | 15 or 5 ms froze the title; 1 ms -42 % presents; 0.2 ms -14 %. | Sets the budget. |

## 3. Slow frames (hitches) with the exact path on
Real slow frames are present intervals over 33.4 ms with harness capture frames excluded. Two passes per arm unless noted.

| Attempt | Result | Rules out |
|---|---|---|
| Cheaper trap exclusion lookup (exact page-aligned key instead of a window walk) | About 700k walks skipped per run, 0 missed; no frame-time change. | Exclusion walk cost as the hitch cause. |
| Hot/cold head-start queues | Still 37-50k hot-stolen needs per run; no change. | Head-start queue ordering. |
| Both together | No change. | |
| Eager head-start staging alone | Marginally best, inside noise. | |
| Retire disarms only the texture's own tokens | Mark staging 1.9 → 0.08 ms per slow frame; equal-after-disarm re-checks about 130k → under 100; stolen needs halved; slow frames unchanged. | Disarm-caused stealing as the hitch cause (kept: exact, removes false work). |
| Preview staging before first view | Race 1 worse (112 vs 89 per minute), race 2 slightly better. | Not enabled. |
| Stall split timers | About 0.05 ms per frame on the desktop, 0.3-0.5 ms CPU on the handheld. | Diagnostic. |
| Hook timing (5 draw-path hooks) | No measurable cost; hooks explain 15-25 % of the slow-frame excess. | Any single hook as the cause. |
| Release fast path plus lock-free negative filter | Presenter texture-lock acquisitions 1,443 → 303 per frame; **slow frames 98.2 → 85.5 per minute on the desktop, 161.3 → 129.9 (-19 %) on the handheld**. | Kept. |
| Head-start worker at normal priority | No-op under Proton (all threads nice 0). | Priority levers under Proton. |
| Second head-start worker alone | Stolen needs -85 %; slow frames level on the desktop, worse on the handheld (zero-need 11.6 → 28 per minute). | Steal count as the hitch driver. |
| **Second worker plus eager staging** | **Desktop real slow frames -18 to -23 %** (confirmed in a second pair); handheld at 15 W 106 → 98 and 88 → 70, frames over 50 ms 15.3 → 12.8 and 15.8 → 11.6 per minute; no fps cost. | Kept (play default). |
| Cheaper exclusion lookup vs the burst lock | Presenter's contended-lock wait unchanged; other threads' `VirtualFree` hook only 5-20 % shorter. | Not decisive for the burst lock. |
| In-process `/proc` wait sampler | Read wineserver's thread under Wine. | In-process `/proc` sampling under Wine. |
| Per-event lock-history ring | Overwrote history; doubled slow frames; -6 fps on the handheld. | Ring-based lock tracing (use fixed per-thread counters). |
| Sub-allocating committed resources (measured first, not built) | vkd3d's own create time was only 0.5 ms per slow frame. | Not worth building. |

## 4. Device loss (NVIDIA, Vulkan path)
| Attempt | Result | Rules out |
|---|---|---|
| vkd3d-proton deferred resource destruction | Lost at 314 s. | Driver-side deferral as a fix. |
| Our deferred release behind all-queue fence completion | 3 of 3 lost on the laptop; a 60 s hold made it worse (3 of 6). | Fence-based holds for a stale-descriptor fault. |
| Retention patch | 0 of 20 vs 0 of 3: unproven; +265 MiB VRAM, +1.8 GiB RAM. | Nothing yet. |
| Typed null descriptors in empty slots | Lost at 374 s despite 1,086 substitutions. | Empty-slot nulls as a fix. |
| Record-time descriptor table copies (NVIDIA guard) | Suppresses a class of faults; violates root signature 1.0 volatility; one sub-guard dropped real draws. | As a cure. Refill at submit instead. |
| Eager staging off | 5 of 8 losses had it on and 0 of 17 clean runs; a paired 6x6 A/B was 1 of 6 each. | Eager staging as a cause (a small-n correlation). |
| GPU timestamp layer on | One traced loss, untraced twins clean. | Trust no device-loss result from a traced run. |
| RenderDoc Vulkan capture | 2 of 2 capture runs lost the device. | Captures as neutral observers. |

---
[Back to the index](../README.md)
