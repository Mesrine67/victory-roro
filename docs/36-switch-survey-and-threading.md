# What every knob taught us, and what we learned about threading texture work

This note is for an engineer building a translator that runs Xbox One D3D12 titles on a PC. Our translator exposes hundreds of runtime switches. Many were born as experiments. A few became defaults. Most stayed off.

We surveyed them all. The question was simple: which switches make frames cheaper, which make output more correct, and which are traps? We also ran a separate study on whether texture work could be spread across threads.

This note turns both into lessons. It reports what each important switch does, what we measured, what we shipped, and what a newcomer should copy or avoid.

## How to read this note

Switch names follow one pattern. Shared switches start with `XBOX_`. Each title on our shared core also reads an alias with its own prefix. We write that alias as `<TITLE>_*`. For example, `XBOX_TEX_STREAM_LOAD` and `<TITLE>_TEX_STREAM_LOAD` are the same knob. The shared name wins when both are set.

Three evidence labels matter, because the survey was strict about them.

- **Measured** means a run with numbers, on a named class of machine.
- **Source-read** means we read the code and know what the switch does, but have no matching measurement.
- **Unverified** means neither. We treat that as "unknown cost", never as "free".

The survey's hardest rule was that a speed number only counts if the output is still exact. A switch that makes frames faster by showing stale or placeholder pixels is not an optimisation. It is a bug with a good benchmark.

Machine classes in this note:

- the RTX 4090 desktop (Windows, native D3D12 or Vulkan);
- the RTX 3080 laptop (Windows);
- the AMD desktop (an RX 9070 XT class card);
- the AMD handheld (SteamOS, Proton);
- the 5900X desktop.

Several numbers come from short, single runs. We say so where it matters.

---

# Part 1. What every knob taught us

## 1.0 Lessons about the survey itself

Before the switches, six lessons about how to evaluate switches. We learned these the hard way, and they changed more decisions than any single switch did.

**1. Defaults differ by vendor and backend.** Many switches resolve to different values depending on GPU vendor, OS, and whether the renderer is native D3D12 or Vulkan. A name like "texture read map" has four or five effective defaults. A survey that reads one default and assumes it holds everywhere will mislead you. Record the resolved value per platform.

**2. Compile-time gates hide switches.** Our shipped "Play" build compiles out many diagnostic and shortcut paths. A test-automation build keeps them. A switch can exist in one build and not the other. When you compare a benchmark arm to the shipped build, check that the arm is not running code the player never gets.

**3. Benchmark arms that skip required work overstate gains.** One of our best-looking results was measured with the exactness barrier's body skipped. The skip removes real work. So the number is an upper bound on a design that cannot ship in that form. We flag every such arm as "diagnostic only".

**4. Whole-run ratios lie.** Startup, cutscenes, scripted parking, and loading all change the ratio. We compared matched route positions (the same lap and second on the same route) and refused to compare late laps when the control run had no route markers there.

**5. Counters come in two kinds, and mixing them gives nonsense.** Some counters report a delta per present. Some report a running total. Sum the first kind. Difference the second. Also, timing scopes overlap across threads, so you must not add them into a "frame budget". They are attributions, not a partition.

**6. Instrumentation is not free, and it changes between arms.** An instrumented run (names, traces, video, CSV) can cost more than the feature under test. Keep the witnesses identical across arms, or the comparison means nothing. Also pin every candidate switch explicitly in the baseline, so a stray shell variable cannot contaminate it.

Two smaller traps are worth naming.

- **Two parsers for one switch.** A few switches were read in two places, and the two readers disagreed about what counts as "on". One accepted any positive integer. The other required exactly `1`. Use one parser per switch, and test odd values.
- **A wrong alias.** One legacy alias for a barrier switch pointed at a different switch's old name. Always set the canonical `XBOX_*` name explicitly in test scripts.

## 1.1 Memory and the title's address space

An Xbox title expects a large, mostly flat address space with its own rules. A translator must reproduce enough of that space for pointers, textures, and constant data to behave. This is the most fragile subsystem we have. It is also where most "experiments" lived.

**What exists.** There are switches for the title's virtual-address (VA) layout and for memory placement. Examples:

- `<TITLE>_CACHED_TITLE_MEMORY` keeps a cached "epoch" view of title memory so queries can be answered without asking the OS. It is on by default.
- `<TITLE>_TITLE_MEM_CACHED` is a different switch with a similar name, and it is an experiment that stays off. Confusing the two is easy. Name experiments so they cannot collide.
- `XBOX_TITLE_TLS_BOUNDARY` fixes how title thread-local storage maps onto host threads. It is on for all titles, and a value of `0` turns it off. It was promoted after correctness failures without it.
- A family of VA alias and overlap fixes (`VA_ALIAS_RESTORE`, `VA_OVERLAP_FIX`), buffer placement experiments (`BUFFER_TITLE_BACKING`, `BUFFER_SYSMEM`), reservation experiments (`XMEM_RESERVE`), and a live memory statistics mode.

**Measured effects.** We have little speed evidence here, and that is itself the lesson. The placement and lifetime experiments were each tried on one or two machines. Several regressed. Three of three laptop runs of one lifetime experiment failed outright. None showed a speed gain that survived exact output. All stayed off.

One memory switch did earn its place, because it saves memory rather than time. `XBOX_BUFFER_TIGHT_ALIGN` allocates eligible small buffers (64 KiB or less, row-major) at their real size instead of the console's coarse alignment. It only applies on Vulkan, and only when the driver reports the needed feature. On the 5900X desktop with Vulkan, private memory fell from about 15,343 MiB to 10,247 MiB (a 33 percent cut). Working set fell from about 11,807 MiB to 6,958 MiB. Frame time did not move. On native D3D12 the switch does nothing.

**Default and why.** Tight alignment is on, behind a support check. Everything else in the family is off.

**General lessons.**

- Memory-placement tricks are cheap to propose and expensive to prove. Stale data and alias races show up as rare flicker, not crashes. Require a long, exact run before promoting any of them.
- Put a support gate on anything that depends on a driver feature, and make the unsupported path identical to "switch off".
- Give every memory experiment an explicit refusal rule. Ours refuse to start when combined with an incompatible experiment, rather than running in a half-valid state.
- Memory savings and speed gains are different claims. Report them separately. We never claimed a frame-time win for tight alignment.

## 1.2 Texture upload, streaming, and exact staging

This subsystem consumed most of our attention. It also produced the largest single cost in our frame, and the most instructive switches.

### The core problem

The console's GPU reads textures from memory the title writes at will. A PC GPU cannot. The translator must notice when title memory holds new texture data, copy it into a GPU-visible resource, and publish it before a draw samples it. If the translator publishes late, the draw shows old or missing data. If it publishes everything every frame, the CPU cost is huge.

We call the "publish before the draw uses it" requirement the **exact publication barrier**. The switch family is `XBOX_TEX_DRAW_EXACT`.

### `XBOX_TEX_DRAW_EXACT` and its detection modes

**What it does.** Before a draw that reaches a texture, the barrier makes sure the current bytes are staged and published. It needs dirty evidence: which pages changed since last time. On native Windows it uses a trap (page protection that faults on write). Under Wine and Proton it uses a write-watch query instead. Our reading of the operating system's documentation confirmed a known race between reading and resetting write-watch state when another thread writes in between. That supports keeping the barrier's evidence step, not skipping it.

**Measured effect.** The barrier is the largest cost we have. On the RTX 3080 laptop with Vulkan, at matched lap positions early in a lap, the control run (barrier off) took about 38.6 ms per present. The run with the barrier on took about 86.7 ms per present. The wrapper-side "outside" time rose from about 20.8 ms to about 56.5 ms per frame.

That gap is an association in one pair of runs. The barrier-on run also logged first-reach traces and names, and the draw workload differed. So we do not claim the barrier costs 48 ms. We claim it is large and worth attacking.

Mean per-present barrier costs, from the barrier-on run, were roughly:

| Cost | Per present |
|---|---|
| Stall waiting on stage/prepare | 3.5 ms |
| Hashing | 2.6 ms |
| Dirty-evidence harvest | 0.75 ms |
| Drain of trap events | 0.8 ms |
| Page protect/unprotect | 0.9 ms |
| Stage | 0.9 ms |
| Prepare | 0.7 ms |

These overlap, so do not add them.

The counters tell a sharper story. In one present, the bookkeeping visited about 431,000 "attention" slots, and only about 1.2 of them had real work. Descriptor events ran in the thousands per present. Bundle calls ran at about 3,600 per present, and every one was a miss.

**Default and why.** Off in the shipped build, pending its own correctness gate. In the exact candidate it stays on. Turning it off is not a speed fix. It removes the thing that makes output correct.

**Lesson.** Measure the barrier by counting work visits against real work. If a million visits find one job, the bookkeeping structure is the cost, not the copies.

### `XBOX_TEX_DRAW_EXACT_FLAT`

**What it does.** Replaces map-based descriptor bookkeeping with flat arrays. Slots and sets are indexed directly. The duties stay the same: same work, same "unknown" handling, same view semantics. Only the data structure changes.

**Measured effect.** On the laptop with Vulkan, matched laps 1 to 3 showed throughput of 0.669 of the control with the old bookkeeping, and 0.965 of the control with flat bookkeeping. Over the run, the flat arm completed 14,892 presents against 11,165. That is about 44 percent more matched throughput, or roughly 31 percent less time per present.

**Why we do not trust the headline number.** Those arms skipped the barrier body. So the result shows only that the bookkeeping itself got cheaper. A full-barrier gameplay run was still unverified when the survey closed. A CPU-only integration test did pass with the barrier model active: 100,000 events per mode, the same digest in all four combinations of trap or watch with flat on or off.

**Default and why.** It was the top candidate for a Play default, conditional on a full-barrier gate. Keep a zero override for rollback.

**Lesson.** The best first optimisation of an exactness barrier is usually the data structure, not the algorithm. Keep a shadow mode that runs both structures and counts mismatches. Our shadow mode showed zero mismatches, zero filter misses, and zero use-after-retire. But a shadow arm that also skips harvest cannot qualify exactness. Do not let a proof arm double as a performance arm.

### The shortcut switches

`XBOX_TEX_DRAW_EXACT_SKIP_BODY`, `SKIP_HARVEST`, and `SIDE_OFF` each bypass part of the barrier. They exist to decompose cost. They let you ask "how much of the frame is the barrier body?" without the barrier running. They produce stale bytes by design.

**Lesson.** Keep decomposition switches. They are the best profiler you can build. But compile them out of the shipped build, and never accept their runs as exactness evidence.

### Cheap-looking CPU switches near the barrier

We tested or proposed several small switches. All stayed off for lack of a clear win.

- `XBOX_LEAN_CPU` bundles several reductions: caching a diagnostic setting, reducing exhausted logging, and moving command and bundle containers. On the laptop, an A-B-B-A comparison reversed direction between pairs. The variation exceeded the gain. With it on, hitches over 50 ms rose in both pairs.
- `XBOX_CPU_BARRIER_STACK` keeps the temporary barrier list on the stack. `XBOX_CPU_BARRIER_COUNT_CACHE` caches an immutable subresource count for one transition. `XBOX_CPU_BARRIER_LOG_GATE` avoids diagnostic work when output is exhausted. None had an isolated measurement. The old bundle did not separate them.
- `XBOX_ZERO_WORK_DRAW_SKIP` skips a draw with zero count or zero instances. Safe only if side effects and queries are preserved.
- `XBOX_TEX_SCAN_FAST` swaps the zero scan for an outlined version with the same predicate.

**Lessons.** First, a switch named "lean" is a claim, not a result. Bundles hide which part helps. Test one change at a time against an unchanged baseline, then confirm a combined winner. Second, `XBOX_TEX_SCAN_FAST` changed nothing in a later test, because the scan loop was not the cost (see Part 2). A faster loop does not help when the time is spent in page faults, not instructions.

### Texture-read switches that did earn defaults

**`XBOX_TEX_READ_MAP` (with `<TITLE>_TEX_CHEAP_VISITS`).** The loader keeps a map of which title ranges are readable. Revisits consult the map instead of asking the OS. The effective default is vendor-gated. It is on under Wine and Proton, on for native NVIDIA Windows, and off for native AMD Windows.

Measured: on the AMD handheld, texture latency at the 95th percentile fell 19 percent. On the RTX 4090 desktop, race-minute p95 fell 43 percent. On the AMD desktop on Windows, the gain was not clear: average 9.94 s with the map on against 8.15 s off, so the gate stayed closed there. A different title on the core gained 37 to 59 percent on the 4090. That is a different title, so it is not our gain.

**Lesson.** A switch that helps three machine classes and does not help a fourth needs a vendor gate, not a global default. Write the gate into the profile and document the reason.

**`XBOX_TEX_SETTLE_WAKE`.** Instead of polling every pending texture on a fixed ring, wake a texture when its settle deadline expires. Mode 1 wakes first landings. It is on by default under Vulkan, with a 64-visit cap and a 1 ms budget.

Measured on the AMD desktop: demand-to-GPU-fence latency p50 fell from about 0.63 to 0.73 s down to 0.09 to 0.10 s. The p95 fell from about 2.8 to 2.9 s down to 0.30 to 0.51 s. The share of textures waiting over one second fell from 42 percent to 2 or 3 percent.

A proposed mode 2 also wakes top-up steps and backs off held textures. An AMD desktop test gave a top-up p50 of 0.59 to 0.69 s against 0.74 to 0.86 s for mode 1, and fewer long waits. It cost about 1.25 ms more per frame, and it did not help first landings. We left it off. Do not assume the promotion of mode 1 proves mode 2.

**Lesson.** Event-driven wake-ups beat fast polling. The ring's revisit gap was the delay, not the copy. We cover this fully in Part 2.

**`<TITLE>_TEX_STREAM_LOAD`.** Reads title memory with SSE4.1 streaming loads during texture preparation. Title memory is write-combined, so ordinary reads are slow. The switch is on by default behind a CPU feature check. It is byte-preserving.

**`XBOX_WORKER_COPY_STREAM_LOAD`.** A different switch. It intercepts the title's own worker copies from write-combined ranges. On another title on the 4090 it gave +59 percent average fps and +37 percent 1 percent lows. On the AMD handheld, capped near 59.4 fps, it gave no measurable benefit. It needs an explicit interval for the title's copy callers, and turning on the master alone refuses to start. We could not transplant the caller interval from one title to another. It stays off.

**Lesson.** Interception switches depend on caller identity, which is title-specific. A tracking layer also adds maps and locks. Make a title-specific input mandatory and make "master on, input missing" an error.

### Async preparation and admission

`<TITLE>_TEX_ASYNC` and `<TITLE>_TEX_BOUNDED` prepare textures on worker threads with bounded retry scheduling. The worker count defaults to half the logical processors, capped at six (a range of one to eight). In-flight caps are 2,048 MiB and 1,024 jobs. A snapshot limit of 2,048 MiB applies, and a setting of zero forces the synchronous fallback. Drain defaults are eight jobs and 2,000 microseconds.

**Default and why.** Async and bounded are on. We kept the numeric defaults and refused to lower the caps for a speed claim. Lowering capacity pushes work back to synchronous paths and creates stalls.

**Lesson.** Async staging must still publish the current content. A worker pool does not make stale data acceptable. Our 30 October throughput split found zero to nine live jobs and the conversion step at about 1 percent of capacity. Workers were not the limit.

### Upload limits that failed

`<TITLE>_TEX_WINDOW_BYTES`, `_WINDOW_US`, `_WINDOW_SUBMITS`, and the `_SELECT_*` budget family cap how much upload work runs per window. They default to unlimited or off. Removing an earlier cap fixed a case where dark surfaces persisted through a whole lap.

**Lesson, and it is a strong one.** Deferring required data is never a performance fix. If a limiter delays content the draw needs, the output is wrong. We rejected the entire "speed by delay" family.

### Placeholders and quality shortcuts

`XBOX_TEX_UNRESIDENT_FILL` paints unlanded textures with a visible marker. `<TITLE>_TEX_SRV_MINLOD` changes sampling. Resolution, render-scale, scenery-distance, and level-of-detail switches change the output.

All fail an exact-output contract. We use the fill marker only as a diagnostic probe. We keep scenery distance and similar settings identical across A-B arms, because a lower setting hides cost and removes visible detail.

### Naming and first-use

`XBOX_TEX_NAME` and the name-trace switches label content for first-reach evidence. The first-use lane (`<TITLE>_TEX_FIRST_LANE` and a dozen `FIRST_*` children) is an older design for scheduling first visits. Many children inherit the master, so enabling one half of the family is dangerous. Setting a child to its "off" value did not disable bookkeeping while the exact barrier was on, because the barrier needs the same tracker.

**Lessons.**

- Inheritance trees of switches are hard to reason about. Prefer one master with few explicit children.
- Document which switches are implied by others, and test "child off, master on".
- Naming helps diagnosis and costs CPU. Turn it off in shipped builds and in any arm you time.

## 1.3 Descriptors and bundles

The console uses descriptor tables and bundles heavily. A translator has to mirror descriptor state, resolve copies, and replay bundles on the host.

**`XBOX_BUNDLE_SWEEP` (with `_RATE` and `_POOL`).** Reclaims dead bundles: those overwritten or unmapped. It sweeps at a bounded rate (64 per present) with a pool size of 64. It is gated to Vulkan.

Measured on the AMD desktop with Vulkan: memory fell 1,686 MiB (14.9 percent) in one race and 1,672 MiB (12.6 percent) in another, with equal frame rate. A long laptop run of 840 s showed no loss and no misses.

**Default and why.** On under Vulkan. It depends on fence and age checks, and release on native D3D12 was not proven safe.

**Lesson.** Reclaim by age and fence, never by guesswork. A memory win is a real win even when fps is flat. Say so, and say it is not a frame-time claim.

**NVIDIA descriptor policy.** A family of switches (`<TITLE>_NV_DESC_MODE`, `UAV_GUARD`, `SRV_DIM_GUARD`, `NV_GUARD_SPLIT`, `DESC_FIX`) governs how descriptors are handled on NVIDIA. The resolved auto policy is:

- UAV protection on, for NVIDIA on Vulkan;
- SRV-dimension guard off in auto, because it omitted world draws;
- descriptor fix off, because a broad descriptor rewrite regressed.

Without the UAV guard, a whole piece of world geometry went missing. With the policy in place, three of three laptop runs passed.

**Lesson.** A guard that fixes one vendor's bug can cause another failure when applied broadly. Make guards narrow and vendor-specific, and write down which half of the split you adopted.

**Descriptor and slot tracking.** Switches like `XBOX_DESCRIPTOR_SLOT_VALUE` keep a slot-to-resource lineage, including copies. They are passive, use try-lock, and drop events on contention. Good for diagnosis. They cost memory and cannot price callbacks from counts alone. We saw millions of descriptor events per minute and no timing attribution for them.

**Lesson.** Count and time are different. Run a separate instrumented timing arm if you need to price hooks.

## 1.4 Pipelines and shader caches

**`XBOX_PSO_PRECOMPILE` and `XBOX_PSO_RECORD`.** A recorder journals the pipeline states a title actually uses. A replayer compiles them in advance, on worker threads, at the next launch. Recipes stay exact. The driver identity, resume points, and crash guard are all kept. Thread count defaults to half the logical processors, capped at eight.

Measured (cold loads, not steady frame time):

- AMD handheld: 159 s down to 113 s (29 percent less), three runs.
- RTX 3080 laptop: 233 s down to 138.4 s (41 percent less).
- AMD desktop, an earlier build: 58 percent less.
- Warm start: neutral.

**Default and why.** On under Vulkan. Native D3D12 on NVIDIA stays off. In the shipped package, an INI setting had accidentally forced prebuild off, which also silenced the recorder. The survey called this out and recommended restoring `auto` in the Play package only. The first launch has an empty journal and gets no benefit.

**Lessons.**

- Journal and replay is the best pipeline-stall fix we found. It turns first-use compilation into load time.
- Package files can override your code defaults silently. Diff the shipped INI against the profile.
- Keep an explicit off override for matched steady-state testing.

**`XBOX_SHADER_MEMORY_CACHE` and `XBOX_PSO_MEMORY_CACHE`.** In-process caches of shader and pipeline bytes. They are on everywhere, and only an exact `0` turns them off. We could not measure an isolated gain. They are cheap insurance, and distinct from disk caches.

**`XBOX_PSO_CACHE` (driver blob cache).** The disk cache of driver-compiled blobs. The gate is closed. We saw no benefit on any tested machine, with Vulkan or D3D12. Under Proton, a historical classification put its disk writes at about 56 percent of pipeline stall time. A separate Proton switch defaults off.

**Lesson.** A disk cache can cost more than the work it saves. Do not combine driver blob caching with journal prebuild. They solve different problems, and the blob writes can add stalls.

**Shader correctness switches.** `XBOX_SH_REDUCE`, depth-decompress, wheel and alpha-to-coverage fixes, and a shader opcode guard family. These are correctness policies. They are on where promoted. We never turned any of them off for speed. The shader guards also hash-admit specific kernels. That makes them title-specific by nature.

**Lesson.** Keep correctness shaders out of your performance experiments. Their cost is real but already in the baseline.

## 1.5 Presentation and pacing

**Pacing switches.** Frame-rate cap, vsync, vblank rate, menu frame rate, fast loading, a frame-latency wait, and a maximum-frames-in-flight cap.

- `<TITLE>_LATENCY_WAIT` auto is on only when the system has its own DXGI library and a Microsoft D3D12 core, which means native NVIDIA on D3D12. It is off on the bundled Vulkan stack and on Proton. The bundled stack lacks a native waitable swap chain, and we measured no gain from forcing it.
- `<TITLE>_MAX_FRAMES_IN_FLIGHT` is zero (disabled) by default. A cap from 1 to 16 is allowed. Non-digits and larger values fall back to zero.
- The packaged build uses a 60 fps cap for play, with vsync, and fast loading on. The shipped INI also supports 120.

**Lessons.**

- A frame cap hides cost. A capped handheld at 59.4 fps cannot show an upload speed-up. Measure uncapped, or measure CPU time per frame, then restore the cap.
- Never lower the frame rate to claim cheaper work. Keep pacing identical across arms.
- Forced latency waiting needs the real waitable object. Do not fake it on a stack that lacks one.
- A paced-present probe (a fixed delay between presents) is a good way to prove that a slowdown is real work and not a pacing artefact. We injected 200, 1,000, 5,000, and 15,000 microsecond submit delays to calibrate what a given loss of time looks like.

**Renderer choice.** `XBOX_RENDERER_STRICT` is on: if the bundled Vulkan stack fails, the translator stops instead of silently falling back to native D3D12. `=0` allows the fallback. Silent fallbacks make benchmark data meaningless, because you do not know which backend produced it. Fail loudly.

**HDR and output.** A group of HDR switches control peak limit, metadata, range tagging, and a restart-only scRGB output mode. They are correctness and output features. They are not frame-time levers.

## 1.6 CPU, threads, and scheduling

**Affinity and sleep.** `<TITLE>_AFFINITY`, `XBOX_AFFINITY_VIRTUAL`, `<TITLE>_AFFINITY_NATIVE`, `<TITLE>_SLEEP_PRECISE`, `<TITLE>_FMOD_UNPIN`, and `<TITLE>_POLL_TO_WAIT`.

Measured: on a different title on the 4090, precise sleep gave no gain and doubling affinity was ruled out. Our own topology effect was unverified. All stay off, and the existing affinity policy stands.

**Lesson.** Scheduling tweaks interact with audio timing, input, and portability. Without a measured win, leave the OS scheduler alone. On a power-limited handheld, every extra busy core takes power from the game.

**Texture workers.** Covered above. They use a below-normal priority, a thread name, and a CPU preference function. Priority matters: a thread at idle priority was starved by the game and made textures later.

**Deferred release.** The `XBOX_DEFER_RELEASE` family delays freeing GPU objects until it is safe. It is off by default. A related experiment failed three of three laptop runs. If deferred release is on, several lifetime and DMA experiments make the loader refuse to start, because their combination is invalid.

**Lesson.** Express invalid combinations as startup refusals. Do not let two lifetime strategies run at once.

**A CPU switch that helped logging.** `XBOX_BOUNDED_LOG_SHUTDOWN` is on. It restores a bounded, asynchronous logger shutdown, in place of a synchronous one that could hang. This is a robustness switch, not a speed switch. The default logger is asynchronous and bounded, with a crash sidecar. A synchronous mode exists for debugging.

## 1.7 Audio and I/O (brief)

- `XBOX_AUDIO_FIFO_BULK` copies the audio FIFO in bulk. It is off. A visual match is not enough to accept it. Audio order, underruns, and latency all need checking.
- Audio mode switches select software audio (the default), off, or a silent stub. A silent output mode serves automated runs.
- Storage identity switches pin a fixed title identity unless a master is set. Safety and identity semantics are not traded for speed.

## 1.8 Diagnostics and capture

Diagnostics are the largest group by count. The survey's inventory found about 600 names. Most were drawing debug filters, trace selectors, capture paths, readbacks, probes, and observers.

Rules that held up:

- Every diagnostic is off in the shipped build. Many are compiled out, not just off.
- Costs are harness-dependent. Screenshot capture alone created hitches. A video recorder encodes on the GPU by default, so capture needs an override on other vendors.
- Do not change capture between A-B arms. Remove observers only from the delivered build.
- A trace that keeps 4,096 lines per second and one million lines per run loses data. A first pass of ours claimed 19 percent of textures never landed. Retire summaries showed that was lost log lines. Know your logger's drop policy before you draw conclusions.
- A diagnostics switch that shares a log with a hot path can be the bottleneck. One run's loader log reached about 329 MB.
- Test-automation inputs (scripted presses, route recovery, park and shot scripting) exist only in automation builds. A shipped build carries no scripted-press code.

## 1.9 Cross-cutting lessons

1. **Promote on exact output, then on speed.** Every rejected switch failed the first test.
2. **Put the gate where the evidence is.** Use vendor, backend, and OS gates in the profile. State why.
3. **Prefer event-driven over faster polling.** The wake beat a faster ring.
4. **A memory saving is a win; label it.** Two of our three "ON" switches in this area were memory wins, not fps wins.
5. **Keep a rollback for every default.** An explicit zero override is cheap.
6. **No blind all-on bundles.** Combining unqualified alternatives produces competing freshness policies, and old content can be reused as if it were current.
7. **Count unknowns.** "Unverified" is a legitimate state. Never read it as "zero cost".

---

# Part 2. Threading texture work

## 2.1 What we studied

We asked: can the late-texture work be multithreaded, and would that fix what a player sees?

The symptom was late textures: surfaces that draw dark or with a placeholder for a few frames after the title first uses them. We studied the path from "texture wanted" to "texture visible". We used existing logs and code reading only. We ran no new GPU work during the study. A short test of two proposed steps on the AMD desktop followed.

The answer was: **partly yes, but threads are not the first fix, and they cannot remove the flicker alone.**

## 2.2 The path from wanted to visible

| Step | Thread | Measured |
|---|---|---|
| Birth, view creation, first gate | Title threads | Birth to demand p50 near 0 ms. The view goes live here, before any data lands |
| Re-check ring | Present thread | 2.4 ms per frame at a 2 ms slice |
| Wake visits | Present thread | 0.08 ms per frame, about 9.5 microseconds per visit |
| Memory query | Whichever thread visits | Cheap with the map; costly with OS fallback |
| Zero scan | Visiting thread | 256 KiB per visit |
| Settle | Time gate | 50 ms floor on partial first views and top-ups |
| Snapshot and conversion | Six below-normal workers | Enqueue to prepared p50 13 ms |
| Snapshot copy of title memory | Drain callers, not workers | Part of submit time |
| Submit copy | Same callers, under two locks | Prepared to submit p50 4 ms |
| GPU copy to fence | GPU | Submit to fence p50 3 ms; enqueue to fence p50 20 ms |

On the RTX 4090 desktop at a 2,000-visit setting while racing, the loader's own frame cost was about 5 ms of a 21 ms frame (46 fps). About 2.4 ms was the ring. About 2.4 to 2.9 ms was the drain and submit.

Workers were not the limit. Existing threads include the title's threads, six texture workers, a bundle-scan thread, a pacer, a video writer when recording, and (on Proton) one submission thread per D3D12 queue inside the translation layer.

## 2.3 Where the wait really goes

We split texture wait time by state, charged to the state set by the last visit. "Discovery" is the gap between the last "nothing yet" visit and the visit that found data. It bounds what instant noticing could save.

| Arm, episode type | Wait p50 / p95 | Over 1 s |
|---|---|---|
| 2,000 wake, first landing | 0.083 / 0.47 s | 1.0% |
| 2,000 wake, top-up | 0.50 / 1.14 s | 8.3% |
| 900 wake, top-up | 1.17 / 2.89 s | 55.4% |
| 2,000 no wake, first landing | 0.54 / 1.76 s | 27.6% |

What we read from this:

- **Top-ups follow the ring.** The p50 was 1.17 s at 900 visits per second and 0.50 s at 2,000. Ring rotation was 1.35 s and 0.69 s. A rate of 4,000 gave about 0.29 s rotation.
- **First landings are already fast** with the wake on.
- **The title is part of the slow tail.** With the wake on, the title's own lateness was 5 to 23 percent of first-landing wait, and 35 to 41 percent of top-up wait. In episodes over one second it was 46 to 55 percent.
- **Half of textures never get their top mip.** About 52 percent of retired textures never had their top pages committed. That is the title's streaming choice. No thread changes it.
- **About two thirds of landings over 0.3 s were loader-led and one third title-led.**

An optimistic ceiling (instant noticing, exactly 50 ms settle, no ring gap, pipeline halved) put top-up p50 near 0.01 s and p95 at 0.33 to 0.48 s. A thread does not reach that ceiling. It needs a wake, not a faster loop.

## 2.4 Why flicker survives any thread count

- **The zero window is structural.** The view is written with nothing landed. There is no placeholder. A partial first view must wait 50 ms of settle plus about 20 ms of pipeline. That is three to four frames at 46 fps, at any thread count.
- **Rate alone did not help.** Whole-surface blackness survived doubling the re-check rate to about 5,900 visits per second.
- **Only not sampling unlanded mips removes the window.** A minimum-LOD clamp that covers "resident-none" textures is the only idea that can remove the last frames of black. It needs descriptor-copy tracking, and its earlier versions were marginal.

## 2.5 Designs we rejected, and why

### Rejected: parallel polling

We considered running the re-check ring on several threads.

It gives no gain. One owner swaps the pending list. Each visit takes a global texture lock several times. Each memory query takes the title-memory mutation lock, so queries are serial. Under Wine, the virtual-memory query takes one process-wide mutex, so parallel polling serialises there too.

More threads would add contention, not throughput.

### Rejected: per-class threads (road first, walls elsewhere)

The idea was a thread for terrain surfaces and others for barriers and buildings.

It cannot be built in general. The loader sees only format, size, mip count, and address per texture. Those do not separate classes. Five format-and-size classes made up 81 percent of pending textures. One address served different formats at different times. Only 653 address keys recurred across all three laps, so a per-texture identity cannot be learned by address.

There is one real class signal: a draw-time binding path that resolves which textures a draw samples. But it is hard-wired to three known pixel shaders and is off by default. Each new class would need its own shader decode.

We also found no public design that uses per-class threads. Public priority signals were "bound this frame" or "last used".

Road-first priority had already been tried twice. One try left a lap's surfaces dark, but it ran on a capped uploader, so it does not rule the idea out.

**What to do instead.** Serve by what the title is waiting on, not by what the object is. Reason, not class, drives the wake already, and the wake worked. If visibility signals are ever needed, the cheapest form is a per-frame "referenced by a known shader" set for a few shaders.

### Rejected: a copy queue for this problem

Submitting texture copies on a dedicated copy queue is legal, but not useful here. The submit-to-fence pipeline was only 0.3 to 10 percent of wait. It would need state handoffs between queues. It would also touch queue behaviour tied to a device-loss issue still open on NVIDIA. We could not verify that the copy queue is a real DMA engine on both vendors.

### Rejected: parking textures by blocking reason

A different title on the core tried a scheduler that parks textures by reason. It worked very well there. Expensive visits fell from 237,000 to 7,700. Demand-to-landed p50 fell from seconds to 5 to 14 ms. It also crashed in three of three sessions with an access violation. A version for our title cost 1.7 to 1.9 GB more VRAM because it kept parked jobs and state.

Do not assume parking is safe or cheap. We required a review before anyone builds it.

### Held back: moving the snapshot copy to workers

A code comment says never to defer raw address reads to workers. The reason was not written down. An older commit message says copies moved to workers, yet the code copies on drain callers now. We said: find out why before moving it.

## 2.6 What we accepted

All default off, in the shared core, extending existing switches where one existed. Each step stops if its test shows nothing.

1. **Count the visit cost.** A diagnostic switch (`XBOX_TEX_VISIT_COST`) counts map hits, OS fallbacks, and time per outcome. It measures. It fixes nothing.
2. **Extend the wake.** Mode 2 of `XBOX_TEX_SETTLE_WAKE` wakes a top-up's next step, and backs held textures off at 50, 100, 200, and 400 ms. The target was a top-up p50 under 0.2 s and fewer than 4 percent over one second, with under 0.2 ms more prep time.
3. **Remove the Windows visit cost** if step 1 blames OS fallbacks, and find out why the map defaults off on AMD Windows.
4. **Wake "unavailable" textures on title commits** (`XBOX_TEX_COMMIT_WAKE`). Ring visits should fall about 80 percent. This needs a review first, because of the parking crash above.
5. **One service thread** for ring and wake (`XBOX_TEX_SERVICE_THREAD`), capped duty, below normal priority, off by default on Proton. Do this only if steps 2 to 4 leave a need.

A single service thread is feasible because the upload path already runs on several threads at once. A texture is claimed with a busy flag under the global lock, so one more caller is the same kind of caller. It would take about 2.4 ms a frame off the present thread, or 4.5 ms at the 4,000 setting.

Rules for any such thread:

- **Cap the duty.** The title-memory lock is held during fallback queries and across native allocation calls. A thread running flat out would block title allocations. Today's 2.4 ms of 21 ms is about an 11 percent duty.
- **Move only the two service calls,** not the whole retry function. That function also runs sweeps, reports, and polls.
- **Copy the worker setup pattern:** CPU preference, thread naming, priority.
- **Give it an owning reference.** The wake needed a use-after-free fix. Any new route needs the same.
- **Do not trust settle as completeness.** A thread visiting earlier does not make data more complete.

## 2.7 What the short test found

We tested steps 1 and 2 on the AMD desktop (Vulkan, 600 s starts, the map and cheap visits on). Results:

| Arm | First landing p95 | Top-up p50 / p95 | Top-ups over 1 s | Prep ms/frame | fps |
|---|---|---|---|---|---|
| Mode 1 (three starts) | 0.97 to 1.60 s | 0.74 to 0.86 / 2.4 to 4.9 s | 34 to 39% | 3.5 | 44.2 to 44.6 |
| Mode 2 (two starts) | 0.66 to 1.01 s | 0.59 to 0.69 / 1.6 to 1.8 s | 20 to 27% | 4.8 | 43.8 to 44.0 |
| Mode 1 plus fast scan | 1.24 to 1.42 s | 0.80 to 0.84 / 2.0 to 2.4 s | 33 to 34% | 3.5 | 44.2 to 44.3 |

Mode 2 gave a real top-up gain for about 1.25 ms per frame, with no sign of more tearing. It did not touch first landings. We did not promote it. The cost was high on that machine, and the RTX 4090 had not seen it. No device loss occurred in 16 starts. One start in each series ran partly in a 24 fps phase, and we excluded it.

The visit-cost counters corrected our first suspect. With the map on, OS fallbacks were about one in 10,000 lookups. With the map off, a Windows memory query cost about 1.4 microseconds, not the 250 microseconds we had measured on another title. But the "unavailable" state triggered about 53 such queries per visit, so a visit cost 74 to 118 microseconds against 5 microseconds with the map on.

The "hold" visits cost about 280 microseconds each. That cost was the zero scan: about 70 to 90 microseconds per call, scanning 3.5 to 4.6 KB per call. The scan ran on the CPU at a steady 3.4 GHz. The fast-scan switch changed nothing, so the loop was not the cost.

One result stayed unexplained. An idle 256 KiB scan on that machine took 27 to 62 microseconds. The same scan inside the game was about 1,000 times slower per byte. About five page faults per scan could explain part of it.

## 2.8 Windows and Proton differences

**CPU speed is not the limit.** The AMD handheld's visit cost was 7 to 8 microseconds. On the RTX 4090 desktop it was 32 to 40 microseconds. The handheld did about 5.6 times as many visits per 2 ms. A weaker chip beat a faster one on this loop. The difference tracks Windows against Proton.

| Machine | Visits/s | Microseconds per visit | Rotation |
|---|---:|---:|---|
| AMD handheld, 55 fps | 13,700 to 14,300 | 7 to 8 | 0.09 to 0.10 s |
| RTX 4090 desktop, 2,000 | 2,300 to 2,900 | 32 to 40 | 0.44 to 0.80 s |
| RTX 3080 laptop | 1,800 to 2,800 | 36 to 51 | 0.62 to 0.89 s |
| AMD desktop, map off | 341 to 1,072 | 108 to 259 | 1.6 to 5.2 s |
| AMD desktop, map forced on | 1,142 to 1,653 | 56 to 71 | 1.0 to 1.4 s |

Public sources helped little, and we say so. We searched for per-call Windows against Linux costs for page faults, read-only faults on never-written memory, and address-space queries. Findings:

- Wine's address-space query walks its own tree under one process-wide lock. That serialises parallel polling on Proton. We read this in source.
- Linux maps a shared zero page on a read fault of never-written anonymous memory. We read this in source.
- Windows behaviour for a read of a never-touched committed page is **not verified**. Our best guess is that it allocates a real zeroed page, which costs more. We found no primary source.
- We found no public per-call numbers for address-space queries, and no emulator report of a slower memory scan on Windows.
- One mailing-list report claimed Windows takes orders of magnitude more page faults than Linux on the same work. It gave no cause.
- Security features such as memory integrity can add cost to faults. Public numbers were end-to-end game fps only, a few percent.

So the likely cause of a slow hold visit is demand-zero read faults. Up to 64 pages of 4 KiB each map a real zeroed page, perhaps 0.7 to 1 microsecond each. That is a hypothesis. To test it, count page faults per visit, try a smaller probe, read through a known-touched bitmap, or query working-set state to skip unresident pages.

**Policy on small machines.**

- Handhelds should poll less, not more. A rotation target could free about 1.5 ms a frame on the AMD handheld. An always-busy extra thread there is wrong, because at 15 W every busy core takes power from the game.
- Keep any service thread off by default on Proton.
- On a desktop that is also gaming, a below-normal thread at capped duty costs about a quarter of one thread out of sixteen. Priority matters, because an underserved thread makes textures later.

**AMD Windows gains most and is least served.** The default there ran 341 to 555 visits per second in eight of nine runs. Why the map defaults off there was never checked. We said to answer that first.

---

# Part 3. Knobs we would build in from day one

If we started again, this is the list. We would build each as a named switch with a documented default and a rollback.

**Exactness and the barrier**

1. **A master exactness switch with a detection mode** (page traps by default on every OS; write-watch as a fallback and as an independent checker in test builds. Early on we defaulted to write-watch under Wine; the trap later proved cheaper there too). Ship it on once it passes. Do not tie correctness to a benchmark flag.
2. **Flat bookkeeping from the start,** with a shadow mode that runs both structures and counts mismatches. Indexed slots beat map bookkeeping when most visits find no work.
3. **Decomposition switches that skip barrier parts,** compiled out of the shipped build, used only to profile.

**Texture streaming**

4. **An event-driven wake** for settled and top-up textures. Do not build a faster polling ring first.
5. **A readable-range map,** gated by vendor and OS, with a per-platform default recorded in the profile.
6. **A visit-cost counter** that separates map hits, OS fallbacks, scan time, and lock time. Build it before you optimise.
7. **Streaming loads** for write-combined reads, behind a CPU feature check.
8. **Worker pool knobs:** count, in-flight bytes, in-flight jobs, snapshot size, drain budget. Defaults: half the cores, capped at six.
9. **A hard rule against deferring required data,** with no "window" or "budget" limiter that can delay current content.

**Pipelines**

10. **Pipeline journal and replay** on by default for Vulkan, with an explicit off, a resume point, a driver-identity check, and a crash guard. It was the biggest load-time win.
11. **In-process shader and pipeline byte caches.** Cheap.
12. **No driver blob disk cache by default.** Test it before enabling.

**Memory**

13. **Tight buffer alignment** behind a driver-support gate. It cut private memory by a third on one stack.
14. **Dead-bundle sweep** with a bounded rate and a fence-and-age check.
15. **Refusal rules** for combinations of lifetime and placement experiments.

**Descriptors and vendor policy**

16. **A vendor-specific descriptor guard policy** with a documented split. Narrow guards, not broad rewrites.

**Presentation**

17. **Strict renderer selection** that fails loudly instead of falling back.
18. **A latency wait** that turns on only when the stack has a real waitable swap chain.
19. **Frame cap, vsync, and menu rate as separate settings,** kept identical across benchmark arms.

**Threads**

20. **Below-normal priority and a duty cap** for any service thread. Default off on handhelds.

**Diagnostics and method**

21. **A resolved-switch census at startup:** print every effective value and its source (environment, INI, profile, default), with invalid input marked. This stops a whole class of "which default was it?" arguments.
22. **One parser per switch,** and tests for odd values.
23. **A survey kit** that builds A-B arms from a fixed baseline: same route, matched laps, identical witnesses, one change per arm, and a pass-through for the baseline of every candidate switch.
24. **A lossy-log warning:** count dropped trace lines and print the count in every analysis.

If you only copy three things, copy these:

- Make exactness a gate you pass first, and speed a gate you pass second.
- Wake work on events instead of polling faster.
- Measure the cost of a visit before you design a thread.

---
[Back to the index](../README.md)
