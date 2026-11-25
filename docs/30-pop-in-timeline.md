# Texture pop-in timeline: three weeks chasing black road in a streamed-texture translator

This is a dated history of one bug in a PC translator for an Xbox One game. The game streams its textures. Sometimes the road, a wall or a patch of scenery drew pure black for a fraction of a second, or for many seconds. The team called it "pop-in" and "black road".

The notes below run from late October to mid November 2026. They follow what the team believed on each date, what they changed, what they measured, what that ruled out, and what they learned. The goal is to save an engineer building a similar translator from repeating the same detours.

A few names for the test machines, used throughout:

- **The RTX 4090 desktop**: the fast Windows machine, running the Vulkan backend (through vkd3d-proton).
- **The RTX 3080 laptop**: a slower NVIDIA machine on Windows.
- **The AMD desktop**: a Ryzen 7 with a Radeon RX 9070 XT. It ran the Windows build in October and early November and ran Linux/Proton later.
- **The 5900X desktop**: an older Ryzen with an RX 6700 XT, added as a test node later.
- **The AMD handheld**: a SteamOS handheld running the game under Proton. It is the lowest-power target.

## Background: what the translator does with streamed textures

The Xbox game creates a texture resource, then fills its mip levels from disk over the following seconds. It does this with ordinary CPU stores into memory it owns. On the console, the GPU reads that same memory directly. On a PC the translator cannot do that. It has to notice that data has arrived, copy it into a real GPU texture, and only then let shaders sample it.

The translator therefore runs an "uploader". The uploader keeps a list of textures that are waiting for data. On every Present call it visits some of them, checks whether the data has landed, and queues a copy to the GPU.

Meanwhile the game creates its shader resource views early. It sets a "most detailed mip" value that says "start at mip 0", whether or not that mip has any data yet. The translator passed that value straight through. A mip that had not landed was a zero-filled region of GPU memory. Zero is black.

That is the whole bug in one paragraph. The three weeks below are the long road to proving it, then to fixing it exactly.

Before the timeline starts, a summary of the earlier attempts matters. Between mid and late October the team had already tried, without success: a shorter re-check slice (4 ms to 2 ms per Present), asking the game for more mip groups, a premature-upload repair, larger copy scales, GPU-side untiling, OS write-watch dirty detection, an early-draw priority lane, a wake service, a queue-depth cap, a top-up class, and a top-up floor. All of these ran while the uploader was still capped at a handful of jobs per Present. A cap removal on 25 October fixed the long black period at the start of lap 1: the handheld went from 231 to 981 uploads a second, and the road was lit from about 10 seconds instead of 78 seconds. What remained was a residual 0.2 to 0.3 second flicker. That residual is the subject of this timeline.

---

## 26 October: the re-check round-robin is the suspect

**Believed.** A literal 250 ms poll had been suspected. The code read showed there was no such poll.

**Found by reading code.** A texture that is waiting on the game is re-checked only on its turn in a round-robin. The round-robin gets a 2 ms slice per Present, spread over roughly 800 to 1,700 waiting textures. On the handheld that means each texture is revisited every 0.16 to 0.31 seconds. That matches the 0.2 to 0.3 second black pops a viewer had reported.

Two more costs sat on top of this:

- Creating a view of a waiting texture did not move it to the front of the queue. It was still labelled "pending view" and waited its turn.
- A 50 ms "settle" delay (the texture must look unchanged for 50 ms before copy) cost a whole extra pass.

**Changed.** A settings-only comparison started on the AMD desktop: the default, a 4,000 microsecond slice, and a min-LOD clamp on the view. An event-driven re-check, which wakes a texture when its view or mip value changes, was built but left off.

**Measured.** A priority FIFO in the job stage had just been rejected. It moved the black episode count from 1 to 1, and made hitches over 250 ms on the laptop worse (59 to 69). The reason is simple. The job stage already finished in about 13 ms. The wait happened before a job existed.

**Ruled out.** Reordering the job queue.

**Lesson.** Measure where a texture waits before you tune the stage that processes it. Here the stage was fast. The queue in front of it was slow.

## 29 October: it is not only the road

**Believed.** Pop-in was a road problem.

**Found.** A viewer reported that it was not only the road: other textures did it too. A scan of footage from three machines agreed.

- 36 confirmed flat-black bursts: road 26, concrete barrier and sponsor panels 7, a wall behind a fence 1, a rock cliff 1, a kerb patch 1. None on sky, foliage or car bodies.
- 23 of them recovered within 2 frames.
- The same spots recurred lap after lap.
- On the handheld the rate was 1.09 bursts a minute overall, 3.8 a minute on one track. The AMD desktop showed 0.35 a minute.
- Each burst had a recognisable envelope. A sliver started at the far end of the road, swept toward the camera in about 5 frames, held for about 13 frames, then snapped back in one frame. The median length was about 0.29 seconds.
- The video encoder was ruled out: the bursts continued across an I-frame.

A live sighting on the RTX 4090 desktop added a longer case. At the start of a race, a large black road and grass lasted about 15 seconds, with trees, grandstands and cars lit.

**Coverage read.** Streamed views are 2D views of tiled title textures. There were 146,526 waiting texture generations, all single-slice, 80% BC4 and the rest BC6H, BC7, BC5 and BC1. A min-LOD clamp already existed, but it only clamped partly uploaded textures. It skipped textures with nothing uploaded at all ("resident-none"). Those were 54% of lowered views, and exactly the black ones. It also never raised a clamp, and never refreshed copied descriptors. That explained why the clamp had still popped.

**Latency estimate.** The time from a view going live to its first mip landing scaled with frame rate times the 2 ms-per-Present budget. Estimates were 6.5 to 13 seconds on the laptop on one track, 2 to 7 seconds on the RTX 4090 desktop, and 0.7 to 1.0 second on the AMD desktop.

**Lesson.** A road-only classifier would never have fixed this. Any fix has to cover every streamed view. And the right metric is per-texture landing time, which the logs could not yet give.

## 30 October: many things ruled out in one day

This was the densest day of the history. Almost every row below is a negative result.

**Stage 1 demand-first ordering.** It was not measurable in the headless A/B, because the camera views diverged between arms. In a live viewing on the RTX 4090 desktop the viewer said the issue had not been fixed. The whole road was black at race start, with about 2,200 textures pending, never draining, at only 14 to 29 MiB/s of upload.

**Throughput split.** With 0 to 9 live jobs and 1,300 to 2,750 textures pending, the workers were idle. The upload pipeline was not saturated. Something in front of the pipeline held the textures back.

**Event-driven re-check, first game runs.** Small effect. The AMD proxy ended 1.4 to 2.2% under the off baseline. Not enough.

**Drain-only budget arm.** 2.0% under baseline. Small.

**Read map and cheap visits.** An external cross-title measurement showed that a per-page VirtualQuery scan cost about 250 microseconds per call. A readability map built from the loader's own title-memory bookkeeping removed that cost (69 s down to 0.12 s per run in that other title). It was ported with a cheaper walk. On AMD machines the gate was not met.

**Min-LOD clamp on landed fence.** Inconclusive, then marginal. A viewer who watched it on the handheld judged any gain marginal at best. Counters explained why. Most of the black samples were textures with nothing resident, where the clamp cannot act (76,320 none-skipped against 10,867 clamped copies).

**A tint test on the road shader.** This was a clever step. The road shader was tinted a flat colour on the handheld. The viewer reported the road was solid purple with no flicker. So the road geometry and its draw were present every frame. Nothing drew over it. The black had to come from what the shader reads.

**Magenta marker.** A missing-data marker was added: any texture with nothing landed is filled with magenta instead of left zero. The viewer played the game with it. Magenta showed in 69 of 73 marks: trees, fence wire, kerbs, road lines, decals. After a scene cut, a garage menu was about 50 to 61% magenta for about 3 seconds. Lateness was real and everywhere. But the live-job count was near zero and 0 admission defers occurred, while the readiness gate logged 25,000 to 35,000 "unreadable" and 4,000 to 9,000 "zero-hold" visits every 10 seconds. The gate that decides "has data arrived?" was the limiter, not copy bandwidth.

**A design review.** A reviewer found a real bug. The clamp used a "resident" flag that is set at submit time, not the landed fence. So it published one fence early. The review also said that scheduling changes alone could not reach a 30% black-time cut.

**Lead analysis.** Unuploaded mips read zero because committed resources are zeroed, draws never wait, and there is no placeholder. In controlled headless runs, the RTX 4090 desktop was only a little worse than the 6700 XT (8.4 against 7.7 seconds of black per lap), and its uploads were not slower. It simply made fewer frames in the burst (29.7 against 42.7 fps), so its per-Present re-check fell behind. A pipeline-state swap was ruled out (0 pipeline creations in the black windows).

**Lessons.**
1. A cheap visual test (tint the draw) can settle a whole class of questions. Here it proved that the draw exists, so the black is an input, not an overdraw.
2. A marker that paints "not yet loaded" turns an invisible state into a visible one. Use it early.
3. Check whether a worker pool is actually saturated before you add workers.

## 31 October to 1 November: the read map is promoted, then withdrawn

**Believed.** The read map plus cheap visits cut the cost of noticing that data had landed.

**Measured.** On the handheld, three paired runs showed the mean p95 of demand-to-landed falling from 9.45 to 7.68 seconds (-18.8%). Hitches over 250 ms went from 3 to 5. On the AMD desktop the p95 did not beat run-to-run spread.

**Changed.** The decision was to switch it on anyway. Both switches became default ON on Proton and on NVIDIA, and stayed OFF on Windows AMD, where the gate had failed.

This decision looks reasonable but it was made on a thin result. Four days later it caused a retraction.

## 5 November: the read map did not fix the black road

**Retraction.** The earlier note "0 seconds of black road per lap" had to be withdrawn. A viewer watched an RTX 4090 desktop run with the read map on and said it had not fixed the black road. The viewer marked 23 frames. The hard-edged black areas were visible.

The scorer was the problem. It read only two flank patches of the road, needed at least half of each dark, and sampled sparsely. It scored 0.0 seconds on a run the viewer had watched fail. It could not serve as an acceptance measure.

**A crude rescore with a wider band.** Share of samples with near-black over at least 30% of the band:

| Machine | Map off | Map on |
|---|---|---|
| AMD desktop | 27.0% | 1.4% |
| Laptop | 25.9% | 0.1% |
| RTX 4090 desktop | 11.8% | 0.8% |

So the map cut the largest black areas by about 98% on two GPUs. Smaller areas remained, and 17 of the 23 viewer marks sat in the same lap-second stretch. A scheduler variant added no gain and cost 1.7 to 1.9 GB of extra VRAM. The black repeated at the same place every lap and start.

**Magenta marker, watched run.** A viewer drove ten minutes with 139 F12 marks. The same patches appeared on all three laps: 10 of 21 magenta patch groups appeared on every lap and held 67% of the magenta time. Magenta was mostly thin strips along the foot of far walls and kerbs, median 0.2 to 0.3 seconds, recurring ones 0.6 to 0.7 seconds, longest 4.5 seconds. Magenta time per lap was 15.7, 17.8 and 20.7 seconds, slightly worse each lap. Patches changed from the marker pattern to the real texture. They never changed from magenta to black.

But the plain black wedges the viewer had reported did not return in that run, as black, magenta or checker. The marker also could not paint render targets, depth textures or UAV textures, so lighting inputs stayed untested. The build also differed from the build where wedges had been seen.

**Texture budget is not the limit.** On the laptop a texture budget of 2,048 MiB against 3,500 MiB gave texture use peaks of 1,299 and 1,300 MiB and the same fps. A viewer saw no difference between the runs.

**Lessons.**
1. A scorer must be validated against what a human saw before it is trusted. Compare it against marked frames.
2. A screen-visible bug can survive a metric that says zero. When a person says it is still broken, believe the person and repair the metric.

## 6 November: the cause of lateness is found, and the wake fix

This is the first day the delay was explained, not just correlated.

**Lifecycle telemetry.** Per-texture demand-to-enqueue-to-fence timing was added. On the AMD desktop, with about 36,000 textures:

- Demand to fence: p50 0.72 s, p90 2.6 s.
- Enqueue to fence only: p50 20 ms.
- The wait sat in two blocking reasons: "seen-settle" (p50 0.63 s) and "pending-view" (p50 0.8 s). Both are "wait for the next round-robin visit".
- Not the game's data (head pages were committed at demand for most textures), not copy bandwidth, not the budget.

**The fix: a time-based wake** (`XBOX_TEX_SETTLE_WAKE`). A texture in those two states is revisited at its own deadline, not at its next turn in the round-robin. The settle length stayed the same, the budget stayed bounded, and no extra state stayed resident.

AMD desktop results, six pairs of full runs:
- demand-to-fence p50 0.63 to 0.73 s down to 0.09 to 0.10 s;
- p95 2.8 to 2.9 s down to 0.3 to 0.5 s;
- textures waiting over 1 s: 42% down to 2 to 3%;
- fps, VRAM and hitches unchanged.

On the RTX 4090 desktop in a slow display-off mode (about 11 fps) the p50 dropped from 0.7 to 1.0 s to 0.3 to 0.5 s. Magenta frames above 0.05% of the screen fell from 15 to 24 of about 88 to 0 to 11 of about 86 (a later recount said the strict colour test undercounted by about 80 to 115 times).

**An important side discovery: the display-off slow mode.** If the display was off when the game launched, the game ran at about 11 fps from the first second. It showed twice as many draws and a lot more black. For several nights, test runs were silently in this mode. A viewer noticed because the runs were not matching. A later fix kept the display awake and wiggled it back on.

**A forced test at normal speed.** Nine starts on the RTX 4090 desktop with the display on, at 46 fps, varied the re-check slice (400 or 900 microseconds) and the wake (0 or 1).

| Arm | Whole-road black, s per lap |
|---|---|
| 400 µs, wake off | 6.3 to 7.9 |
| 400 µs, wake on | 0 to 0.30 |
| 900 µs, wake off | about 0.3 to 1.6 |
| 900 µs, wake on | 0 |
| 2,000 µs (default), either | 0 in the scored window |

So starving the re-check slice causes whole-road black at normal speed, and the wake removes it. The second result is subtle. The wake does not simply add visits. Control arms with more total visits stayed black. The better wording is: "textures already in use are serviced late, and a low rate is one way to get there."

**A viewer test at the highest slice.** The viewer drove four runs with the wake on and a slice from 900 to 4,000 microseconds. The viewer reported that much of the pop-in and black road was gone, but quick flickers remained, and at 4,000 the same textures still seemed slow. Whole-road black still appeared in two runs: one lap 2 frame at 0.786 of the band dark, and another at 0.567. The cost of a higher slice was real: 1.4 ms per frame at 900, 2.5 at 2,000, and 4.3 at 4,000.

**Also on 6 November.** A study asked if the work could be multithreaded. The answer: partly, but not first, and not a cure. One background thread for the ring and wake was feasible. Several polling threads would not help, because each query serialises on one mutex (and on Proton, on the Wine process-wide virtual memory lock). A "road thread" could not be built in general, because the loader cannot name road shaders without an opt-in path.

**Cached title memory.** A second study found why the readability scan was slow on Windows. Streamed title memory was write-combined. The zero scan read it 8 bytes at a time at 55 MB/s, against about 9,700 MB/s on normal memory. A switch to map the memory as cached took the scan from 52 to about 4,800 MB/s, the pending-list rotation from 1.0 to 0.11 seconds, and the Present thread by 1 to 2 ms a frame. But it gave no visible black gain. A viewer watched it on the RTX 4090 desktop and saw no difference. It is a CPU saving, not a visible fix.

**Lessons.**
1. Time-stamp every stage per object before you theorise. The 0.63 second wait was invisible in aggregate counters.
2. Prove the cause with an intervention (starve the slice), then prove the cure with a second intervention (the wake). Correlations across mixed machines had been misleading.
3. Hardware state such as "display off at launch" can change behaviour by a factor of ten. Record it with every run.

## 6 to 7 November: more reviews and the merge

**Wake mode 2.** A variant also wakes a texture's next top-up step. Two valid starts against three: top-up p50 0.74 to 0.86 s down to 0.59 to 0.69 s, p95 2.4 to 4.9 s down to 1.6 to 1.8 s. First landing unchanged. Cost +1.25 ms per frame.

**A rebuilt half-written-texture check.** One risk with a wake is that the copy may run while the game is still writing the mip, so the GPU copy is torn. A span recheck at 250 ms and 2 seconds caught 98 to 100% of injected tears. On real runs, it found a candidate class (a BC4 1024 mip chain) in both fix and control arms, so there was no sign the fix made it worse. But "harmless" was not proven.

**Naming.** Stable per-texture ids and per-landing lateness were added. On the laptop, 137 ids were late at the same lap-second on all three laps. The worst first landings looked like baked lighting and shadow data (BC4 masks, BC6H lightmaps). The worst top-ups were large colour and normal maps for trees and walls. Top-ups were 91% of summed lateness, but the figure counts from the first view that listed the mip, so it includes the game's own loading time.

**The merge, 7 November.** The go-ahead was to merge and switch on whatever we were confident in. The lead judged only the wake ready. It became default ON for the game on Vulkan after a check on the AMD desktop (stock AMD settings, three laps each):

- Whole-road black, seconds per lap: control 6.5, 6.3, 5.3; with the fix 0, 0, 0.27 and 0.30, 0.25, 0.
- Fps 43.6 to 43.7, control 43.7. No device loss, no new crash.

Not yet fixed by the merge: top-ups (91% of summed lateness), big textures that still took over 1 second to their last fence (19% laptop, 35 to 42% AMD desktop), short whole-road flashes, device loss, distant building shadows.

**Lesson.** The merge was right for the measured defect and wrong to call it done. A viewer, not a scorer, found what was left.

## 7 to 8 November: a harder benchmark track, and ideas that did not work

**A new benchmark.** The viewer said a different track should be the benchmark for where black road and late textures occur. An automated route was built with scripted menu presses. A fresh gotcha: a saved menu state pointed at a different track, and a slow start shifted the wall-clock presses so that the run drove the wrong track. The rule that came out of it: check the time of the first screenshot. Over 300 seconds means the wrong track, so discard.

**Every texture is recreated each lap.** Of first landings in laps 2 and 3, 80,268 were ids that had landed in an earlier lap and 7,373 were new. The same textures were re-landed at the same lap-second every lap, which suggested "evicted and reloaded".

**Recreated-texture reuse (a cache).** The idea: remember the GPU copy of a texture and seed a recreated one with it.

- Step 1, observation only. A full SHA-256 over a complete untiled snapshot could not exist at first landing. 0 of 8 apron textures got a digest before first use, because the first landing is a partial mip chain. Across the whole run, 69% of generations eventually got a digest but only 0.5% before their first view. Verdict: a safe identity does not exist in time.
- Step 2, a speculative GPU seed. It seeded about 375 to 396 generations per run, but 51 to 55 were mismatches (the quarantine caught wrong content), and the key table saturated at 8,192 entries. Landings over 1 second were 1.9 to 2.0% against 0.13%. It was turned off. A later "verified" variant made 0 seeds from 27,579 candidates.
- A bigger budget (128 MiB, 512, 1,024) did not move the number of late first landings per lap. At most 1.5k of 117k first landings were seeded.

**A coarse-first prototype.** It split only fully readable first chains, behind safety gates. It split 1 texture and fell back 129 times, so ON equalled OFF. Not merged.

**Where the delay really was.** A first-landing stage timeline gave the answer:

- Median 85 to 120 ms: the 50 ms settle plus lateness past that deadline (quantised by the Present interval).
- Worker prepare 4 to 7 ms; submit to fence 2 to 3 ms.
- A long tail, about 11% of textures at 3 to 8 seconds, all in one 20,000-texture load wave. It was the per-Present drain cap (8 jobs, 2 ms). The in-flight job count pinned at about 1,000 with pending up to 10,000.

**An existing environment variable fixed it.** Raising the drain to 64 jobs per Present, two starts on the AMD desktop:

| | Default (3 starts) | Drain 64 (2 starts) |
|---|---|---|
| Landings over 1 s | 10.7 to 11.1% | 0.2%, 0.1% |
| No-landed-mip texture-seconds | 64k to 66k | 14.1k, 14.1k |
| Landing p99 | 6.5 to 6.9 s | 0.79 s, 0.65 s |
| Small apron textures, median | 1.7 to 1.9 s | 42 to 45 ms |
| fps, hitches over 250 ms | 45.5, 0 | 45.5 to 45.6, 0 |
| Peak pending | 10.6k to 11.2k | 2.0k |

This was the most efficient change of the month: no code. It ran on one AMD GPU and one scene, so it was not final. A longer proof was still needed before changing the default.

**Lessons.**
1. A cache of recreated textures needs an identity that exists before first use. Here the data arrives in pieces, so it does not.
2. A hard-coded compile-time table (8,192 slots, dead slots never reused) can quietly turn a design into a no-op. Check the reject reasons.
3. Always compute the stage timeline. The median and the tail had different causes.

## 9 to 10 November: first-use guarantees, then exact black frames and a closed door

**First-use preparation** (a submit-time guarantee that every texture a draw can reach is prepared before it runs). Version 1 prepared 0 textures: 40.2 million consumer ranges were unresolved. The game reads streamed textures mostly through bundles, then through unbounded descriptor tables (bindless-style). Version 2 added bundle inheritance. Still 0 prepared, 41.65 million unresolved, 40.6 million of them "table-unbounded". Version 4 used a conservative heap-reachable rule. It prepared 1,843 textures (811 MB) but could not keep up, with hitches over 50 ms rising from 885 to 2,327, and over 250 ms from 7 to 25. Conclusion: with bindless tables an exact draw-level set is impossible, and the heap-wide over-approximation is too expensive. A submit-time preparation is not viable as the zero-flicker route. The lane was stopped.

**A series of instrumented runs.** Sync-upload mode (every admitted upload lands before the next submission) did not remove the cutscene black. So late landing of admitted uploads was ruled out for that scene. Render-target lineage dumps showed that the black was real render-target content: a repeatable five-boundary sequence wrote exactly RGB zero into two colour targets over the road crop. A later episode ring caught two black episodes, and traced the black upstream: the multisampled scene target was already black at the road before later passes. The draw that writes the road into the scene target wrote zero, or did not run.

**A RenderDoc route closed.** Capture at the Vulkan level wrote 7 captures. All failed to replay with the same opaque-capture-address error. The device was lost during capture 2 of 2 times. The route was closed.

**Lessons.**
1. Heavy capture tools can change the failure. The capture runs lost the device and collapsed to 0.2 to 0.5 fps. Do not treat them as the game's timing.
2. Logging reached its limit when the game used bindless tables and indirect commands. Identify what the logger cannot see before investing in a longer run.

## 11 November: independent corrections and the cause is found

**Corrections from an independent review of the earlier work.** Several earlier claims did not hold:

- The RenderDoc captures had captured the pre-race menu, not the black frames.
- "Zero lineage loss" referred to a stream counter only.
- A one-pixel exact-zero test misses near-black and partial black.
- A display setting had changed from 1080p to 1440p between runs, unrecorded, so frame counts did not compare.
- The upload-sync switch could drop a title submission on timeout, so some arms were not clean.
- The leading hypothesis stayed: a view is live before its mip is on the GPU. In 7 of 7 runs the black ended within two presents of the end of a 26 to 27 frame upload burst.

**The marker A/B (the proof).** Same pinned build, same arm, same route on the laptop. One change: the missing-data marker on. The control road probe was exact zero on 25 of 25 frames. With the marker on, 0 exact-zero pixels. The road became a magenta checker on the frames where the control had been black.

So: **the black road is the road draw sampling texture data that is not on the GPU yet.** A second reviewer confirmed it independently.

But the wall stayed black, and it was untested. The marker had painted only 116 of 1,951 textures created at the cut. A silent 128 MiB pin cap in the marker code had refused the rest. "No magenta" there proved nothing.

**Lesson.** A marker is only as good as its coverage counter. Print refusals. A silent cap cost a day.

## 12 November: which textures, and the exact design

**What the road waits for.** At the cut the game created 1,929 textures in one present, about 318 MB of uploads over 26 presents. The road waited on 20 of them: 2 colour, 2 normal, 2 material maps, and 14 BC4 tyre-mark overlays (27.8 MB of mips above 128 pixels). The wall waited on 3 concrete BC7 maps. A raised marker budget (4,096 MiB, 1,954 registered, 0 refusals) then showed the wall solid magenta where the control was black. The wall also samples data not on the GPU yet.

**How data arrives.** The large mips reach title memory by CPU stores, not by completed file reads. The uploader had seen the road maps readable and non-zero 815 ms before the road showed, but copied them 20 presents later. The delay was the uploader's creation-order queue, with a fixed amount of work per Present, not the copy itself (a fixing batch took about 24 ms).

**A separate wheel pop.** A single-frame wrong pose on a front wheel and spring, seen in 3 of 9 cutscene runs, was shown to be wrong in game data before drawing. It was a different bug (a suspected thread-timing issue), not the black-road mechanism.

**The exact-fix design** (agreed after a design review and a second reviewer):

- Console equivalence at submission. Before the game's command lists run, every reachable committed span of a CPU-produced texture must have a GPU copy made from bytes not marked dirty since read.
- Dirty marks come from commit, protect and free hooks, from file reads, and (required, because the data arrives by CPU stores) from write-watch.
- An audit thread counts changes without a mark.
- Never wait on a GPU fence. Never drop a submission. No placeholder, clamp or timeout.

This is an "exact" design: it does not approximate, hide or blur. It keeps the console's ordering guarantee.

**First runs of the exact switch (`XBOX_TEX_DRAW_EXACT`) failed.** In the first track load, the barrier stalled the game. Presents fell to 2 to 9 fps, about 2,100 to 2,250 presents in 600 seconds against about 17,000 with the fix off. The write-watch harvest made 76 million GetWriteWatch calls (about 1,370 per submission). A control run showed the black wall even with the fix off at uncapped speed, so the black was not an artefact of a frame cap.

**Step 0 found two causes of the stall:**

1. **Time alone stops presents.** With the exact switch off, a busy-wait of 15 ms or 5 ms per primary-direct submission froze the title at the first track load. 1 ms survived at -42% presents. 0.2 ms cost -14%. The barrier budget is far under 1 ms.
2. **The side effects alone also stall it.** With the switch on and the barrier body skipped, the game near-froze from 72 to 449 seconds at the race load. Submissions waited 201 seconds for one lock.

A lock census found the culprit in the afternoon: a "mark all" walk held one lock 8,648 times for 455 seconds on the laptop. Title frees passed size 0, and the code read 0 as "all memory". The game frees thousands of blocks at a cutscene load. The fix resolves each free to its real range, and runs callbacks outside the lock. A rerun showed 21,968 presents in 600 seconds against 22,666 for the control (-3%), and submission lock wait fell to 0.12 seconds in total (from 243 seconds).

**Lessons.**
1. The first-order mistake is to treat a fix's cost as the barrier's cost. Its side effects (locks, hooks, helper threads) can be larger.
2. A single sentinel value (size 0) that means "everything" in one function and "nothing" in the caller will eventually walk the whole heap. Resolve it at the boundary.
3. When a stall exists, ask first if time alone causes it, with a pure busy-wait. This cost one day and saved several.

## 12 to 13 November: staged rebuild, a failed speed gate, an exactness stop

**The staged rebuild.** The design review produced a build sheet with stages. First, cheaper side effects (this matched the old answers exactly, and reached 0.95 of control on the RTX 4090 desktop and 0.64 on the laptop). Second, cheap reach.

**The cheap-reach stage failed its gate** (mean 5.2 ms against a 30 microsecond limit). The attention walk visited about 24,500 unknown slots per submission. A reviewer re-derived it: the cost was fan-in, not reach. One whole-heap walk per submission over about 21,600 slots that name only about 28 reason and texture pairs. A per-texture slot index fixed that: 324 down to 14.2 microseconds mean.

A second reviewer blocked two proposals as written. A bundle template could silently under-include when a caller inherited a table set inside a bundle. A "no title backing" rename would miss three menu textures that are filled every frame by buffer copies. Both stayed out.

**Where the side-effect cost lives.** Barrier skipped, laptop, presents against control: 0.54. The descriptor path off, 0.68. All five hooks off, 0.85, so about 15% lay outside the hooks. The title thread paid 4.3 ms per Present on one descriptor lock, and 1.46 ms on bundle reach.

**An exactness stop, 13 November.** A long run found 27,830 "unreadable-cache-stale" breaches. A span cached as not readable while its memory was readable can skip its copy, which is the black road again. An earlier run had the same fault (8,514), missed because only arms with the audit off had been checked, and those cannot detect it. The root cause was a birth race: the cache stored readability before the state was inserted, so a commit in between was never seen. The fix took the count to 0.

**A trace proof of first reach.** For the 32 road, wall and overlay textures, 290 first reaches were traced. At every one, the barrier either copied the texture in that same barrier (63) or every readable span already had a landed copy (227). The proof was at the log level. Not shown on screen: the load frame took about 13 seconds in the slow build.

**Lessons.**
1. When you review an A/B, check that the arms you checked can detect the failure. An arm with the audit off cannot see an audit breach.
2. A second review caught defects in the first reviewer's fix every time. Budget for it.

## 13 November: the world vanishes on the straight

**Viewer report.** The start grid was fixed. But on the long straight the world vanished and came back about 10 seconds later.

**Cause.** The fix ran at 1 to 5 fps. A trap event ring overflowed 127,391 times, each overflow forcing a re-check of every texture. The game's own frame-clocked streaming fell 7 to 15 seconds behind. A slow-proof run later confirmed it: with the fix off but the game paced to about 2.5 fps, the same palms, overpass and roller coaster vanish. Speed is part of correctness for this kind of fix, because the title streams on a frame clock.

**Fixed.** The trap drain runs at O(changes), harvest batching, a bounded rule, a wall-clock base uploader. Per Present, in the race: barrier stall 7.0 ms (was 203), drain 0.3 ms (was 182), harvest 0.3 ms (was 65). Frame rate went from about 2 to about 16 fps on the laptop.

**Later the same day.** The race-load frame still took 12 seconds, because the barrier copied 84,058 spans (65 MB) one by one: stage 5.1 s, prepare 4.0 s, submit 2.7 s. A later fix found that the freeze was a per-span log row (85,504 rows in one present, written under locks) that existed in test builds only. After the fix the race-load window was 435 ms and the largest single present was 216 ms.

**Lessons.**
1. A throughput fix can look correct in a test and still starve the title's own streaming. Test at the speed a player will see.
2. A test-build-only logger can dominate a stage. Compare a play-like build, not just a test build.

## 14 November: bookkeeping cost, a replay bundle, and the Proton trap

**Flat bookkeeping.** On the laptop the fix cost about 1.44 times the median frame time and 1.70 times the p95 at first. A "flat" mode of bookkeeping cut the cost. With the fix on and flat off: 44.3 ms median frame. With flat on: 32.2 ms. Fix off: 22.4 ms. The mode became a Play default after a full-barrier gate.

**A sweep of ten switches.** None except bundle replay gave a win (a lean CPU path, barrier logging, a zero-work draw skip, a fast scan all gave noise). Bundle replay brought settled frames from 25.7 ms (fix on) to 23.6 ms (fix off was 21.0). A differential check ran 14.18 million bundle and 5.99 million direct-draw comparisons with 0 mismatches.

**Watch mode is unplayable on Proton.** The write-watch harvest costs 41,000 to 56,000 GetWriteWatch calls and about 63 ms per present on the laptop. The mitigation is the first-write trap, which write-protects trusted spans and treats a fault as a dirty mark. But the trap is a Windows mechanism. On the handheld under Proton the trap path was untested, with an estimated 1 to 6 ms per present.

**Remaining defect.** Audit arms crashed on an unguarded SHA-256 of title memory. A "snapshot-read" breach persisted: a guarded copy read of title memory faulted while the range looked readable. 2,502 rows in three runs, 2,492 of them on the speculative head-start worker and 10 on the barrier thread. No commit failures. A later change held the game's backing memory mapped while a snapshot copies, and the fixture race went from thousands of faults to 0.

**Lesson.** A mechanism that is cheap on one OS can be unusable on another. Plan the Proton path before you declare the design done.

## 15 to 16 November: eyes-on, speed rounds, and the play defaults

**First eyes-on "correct" on NVIDIA.** On 15 November a viewer watched one RTX 4090 desktop run at 1440p with the fix on in Play mode and reported that, apart from performance, it looked correct. One short segment, one look. Race median fps 71.9 (p10 49.9). The first 15 seconds ran at 44 to 58 fps with p99 frames of 80 to 140 ms.

**A fix cost, honestly stated.** The laptop paid about 45% at the race start (20.8 against 36.9 fps) and 30% in the race (28.9 against 41.4). The RTX 4090 desktop paid about 19% and 13%. The laptop was GPU-bound at the race start with the fix on, and a stall probe (a CPU-only 550 microsecond stall per submission, fix off) reproduced the cost, which showed that CPU stalls starving the direct queue were the "GPU" cost.

**Several speed rounds, mostly negative.**

- A resolve-point memo: exact (2.44 million hits checked, 0 mismatches), but a 20 to 27% hit rate and no gain.
- An upload pool, footprint cache and SSE2 scan: exact, no real gain (the pool filled at 128 MiB and 40 to 45% of jobs fell back).
- A dense slot table and a descriptor-copy tracking change: exact. Together they lifted the handheld from about 31.6 to 37.8 fps in the race.
- Upload batching: unsafe. Four of four runs hung about 30 seconds into loading on the 5900X desktop, with batches up to 4,470 members in one ExecuteCommandLists (53,405 on the handheld, which device-lost once).
- A reported "debugger makes trap mode much worse" result was a confound. A remote desktop session on the RTX 4090 desktop had slowed every run from 88 to 57 fps.

**A hash change, a snapshot pool and an environment cache.** On the handheld, the stack went from 30.8 fps to about 44 (faster hashing), 45 (snapshot pool), then 50 (an environment-variable cache). The cache was a real Proton win of about 10% (4.5 fps): Windows rows were neutral. That fits environment lookups being slow under Wine. All differentials reported 0 mismatches.

**A heads-up on a reported correlation.** Every device loss in the fix-on rows still on disk for the RTX 4090 desktop had an eager head-start switch on (5 of 8 against 0 of 17). A later paired test gave 1 of 6 each way, and earlier losses at the same fault address predate the switch. Correlation, not cause. The switch left the Play defaults anyway.

**Hitch fix.** A revision-aware head-start request (`XBOX_TEX_DRAW_EXACT_HS_REFRESH`) became ON in the viewer's setups: race hitches over 50 ms per minute 72.7 and 78.5 against 96.3, 110.4 and 98.2 with it off. A full audit on the 5900X desktop checked 1,099,247 times, with 0 changed without a mark.

**Where it stood on 16 November.** Black road and late textures were fixed exactly, in the sense that no draw samples a texture before its data is on the GPU, with a trap and an audit proving it. The fix still cost frames, most on the slower laptop, and a few exactness classes (snapshot reads on a dying texture, bundle variants) were still open. The handheld, the lowest-power target, ran at about 50 fps with the Play stack. The device-loss bug on the RTX 4090 desktop remained a separate open item.

---

## Lessons

These are the points an engineer building a similar translator can use directly.

**On finding the cause**

1. **Prove the mechanism with an intervention, not a correlation.** Four things moved the answer: a tint on the draw (the geometry is there), a magenta fill (the texture is missing), starving the re-check slice (lateness causes black), and a time-only busy-wait (the barrier cost alone stops presents). Each was cheap and each ended a week of argument.
2. **Make "data not loaded yet" visible.** A marker that paints unloaded textures turns a one-pixel mystery into a picture. Print the marker's own refusal counters, or you will mistake a cap for a result.
3. **Timestamp each stage per object.** Aggregate counters hid a 0.63 second wait. Per-texture demand-to-enqueue-to-fence rows found it at once.
4. **Know what your logger cannot see.** Bindless tables, bundles, compute and indirect commands defeated every draw-level logger. Several rounds were spent before the team accepted that.

**On measuring**

5. **Validate a scorer against a human.** A scorer that reported 0 seconds on a run a viewer had watched fail was withdrawn. Keep a set of human-marked frames and check every new metric against it. A single-pixel test misses near-black and partial black.
6. **Record the environment with every run.** A display that was off at launch, a changed resolution, a remote desktop session, a full disk, and a menu press landing on the wrong track each invalidated runs silently. Check the time of the first screenshot, the frame rate in the first seconds, and the lap clock.
7. **Heavy capture changes the failure.** RenderDoc runs lost the device and ran at under 1 fps. Treat their timing as invalid.
8. **Write down which arms can detect the failure.** An arm with the audit off cannot report an audit breach. Do not count it as clean.
9. **Test at the speed a player sees.** A fix that is correct at 2 fps can make the game's own frame-clocked streaming fall 7 to 15 seconds behind.

**On design**

10. **A cache of recreated textures needs an identity that exists before first use.** Streamed data arrives in pieces. A whole-texture hash cannot exist at first landing.
11. **Exact beats approximate when a person is watching.** Clamps, placeholders, fills, timeouts and drop-on-timeout all failed to remove the symptom or created new ones. The exact rule is simple: no draw samples a texture before its data is on the GPU.
12. **With bindless descriptors, a draw-level set cannot be known.** Either over-approximate over the whole heap (too costly) or track writes and keep every reachable texture current (a write trap plus an audit).
13. **The side effects of a fix can cost more than the fix.** A mark-all walk under a lock, a per-span log row, descriptor locks and per-draw range evaluation were each larger than the barrier. Run a lock census early.
14. **A sentinel such as "size 0 means everything" will eventually walk the whole heap.** Resolve it at the boundary.
15. **An existing knob can beat new code.** Raising the drain from 8 to 64 jobs per Present cut landings over 1 second from about 11% to 0.2% with no code change. Look at the caps first.
16. **A cap that only looks like a design.** A compile-time table of 8,192 slots with no slot reuse turned a reuse cache into a no-op. Read the reject reasons.

**On process**

17. **Pair every design review with a second reviewer whose job is to break it.** A second reviewer found a wrong K, a wrong function, a refuted rule, a silent under-inclusion in a bundle template, and a stale-cache hole in the audit. Not one review ran clean.
18. **Do not call a measurement a fix.** The read map cut the largest black areas by about 98% and was promoted. Four days later a viewer watched it and said it had not fixed the road. Promote on what a person sees, and keep the scorer as a supporting number.
19. **Plan the slow platform on day one.** Write-watch harvest was unplayable under Proton. A mechanism that is cheap on one OS can cost milliseconds on another. The environment-variable cache was a 10% win on Proton and a no-op on Windows.
20. **Report the limits of every result.** Most of the good notes in this history end with "not covered": one viewer, one GPU, one track, one run per arm, a sampling gap that misses short flashes. Those lines kept later teams from over-reading the numbers.

---
[Back to the index](../README.md)
