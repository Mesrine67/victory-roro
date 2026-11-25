# Chasing frame hitches in an exact texture-staging path: a chronological account

This note records a long measurement campaign. We built a PC translator for an Xbox One game. At each `ExecuteCommandLists` call, the translator stages texture bytes "exactly". That means the bytes the GPU sees are bit-identical to what the original console path would have produced. No placeholders, no lower mips, no skipped or deferred copies, no added fences.

Exact staging costs CPU time. Frame times showed a tail of "slow frames", meaning frames over 33.4 ms, even though the average was near 17 ms. We spent roughly a dozen iterations trying to find out why, and whether any exact-safe change could remove them. This document walks through each step in order. Each step gives the hypothesis, what we built, how we measured, the numbers, the verdict, what a second reviewer challenged, and the lesson.

The short version: we built many correct, exact, well-tested changes. Most of them moved nothing. Only one combination (a second head-start worker plus eager queueing) gave a real gain, and it was modest. The main value of the campaign was learning how to measure this problem, and how often our first reading of the data was wrong.

## Setting and vocabulary

- **The barrier.** At each submission the translator checks which textures the command lists will read. It proves each one is up to date and stages the bytes if not. This runs on the presenting thread.
- **Head-start (HS) worker.** A helper thread that stages textures early, so the barrier finds them ready. When the barrier needs a texture and the worker has not finished it, the barrier takes it over and stages it inline. We call that a "steal".
- **Trap registry.** The loader write-protects tracked memory pages so it learns when the game writes to texture memory. About 111,000 spans lived in one ordered registry.
- **Test machines.** The AMD desktop (Linux/Proton) ran most rows, uncapped, with vsync on. The AMD handheld at 15 W (SteamOS/Proton) is the real play device. The RTX 4090 desktop appears only in older evidence.
- **Race laps.** Each test row drove two races. Only about 40 % of each row was race laps. The rest was menus and replays, with a different slow-frame profile.
- **Instrument discipline.** Every observer switch was default-off and "exact-safe". Each build carried a proof that, with the switch off, the code equalled the previous build byte for byte after removing the marked insertions.

## 1. Present-hook split

**Hypothesis.** Early rows showed a large share of slow-frame time in the loader's present hook. We suspected something in that hook (frame cap, compositor, logging, capture) was the hitch source.

**What we built.** A counters-only observer, `XBOX_EXACT_PRESENT_SPLIT`. It divided the present-hook bucket into 15 named parts, such as frame cap, latency wait, compositor, texture maintenance, retire, probes and reports, capture, video queue and swapchain. A remainder part caught the rest. Parts summed exactly to the parent bucket. The observer used an independent clock mark so it could never change the legacy timers.

**How we measured.** First offline only: 72 fixture checks per build, a cost microbenchmark (6.3 to 8.4 us added per frame unsampled), and a reader that printed per-part wall and CPU. Game rows came later.

**Numbers and verdict.** The first real rows showed the remainder part holding all of the time. The reader noticed that the signed adjustment equalled the parent bucket exactly, with every named part at zero. That is a synthetic reconciliation, not a measurement. In one row it was 6.56 s over 16,701 frames.

The cause was our own bug. The loader built a second, unused submission object on a secondary path. Its constructor started an exclusion counter, but its finish call was conditional and never ran. The counter leaked, every later frame stayed "excluded", and the reconciliation invented an all-remainder result. A small balancing guard fixed it.

**What review caught.** Before the freeze, review found the first draft would have advanced the legacy timers through the added scopes. The independent mark replaced it. Later review fixed two stop-time overlaps by emitting an explicit "incomplete" marker instead of waiting.

**Lesson.** A reader that checks "named parts sum to the parent" can still pass on garbage. The check that compared adjustment against bucket caught it. Build the check that asks "is the decomposition actually measuring anything?" before trusting any breakdown.

## 2. Lap-by-lap slowdown (a side analysis)

**Hypothesis.** The player reported that a long session got slower lap by lap on the handheld at 25 W. We assumed the barrier's fixed work grew with tracked texture memory.

**How we measured.** A log-only analysis of one nine-lap session, with lap windows cut by a fixed 138.85 s period and driving windows only.

**Numbers.** Lap 1 was cold at 13.7 ms. Laps 2 to 6 were flat at 13.0 to 13.3 ms. Then it climbed: lap 7 plus 0.45 ms, lap 8 plus 1.2 ms against laps 5 and 6 on the same stretch. Whole-lap means went from 13.28 ms to 14.15 ms, about plus 0.87 ms per frame. Barrier stage time moved only from 0.329 to 0.342 ms. Texture counts and budget gauges were flat after lap 3. Present-thread CPU supplied 0.64 of the 0.87 ms. Process CPU was flat.

**Verdict.** The barrier did not explain the trend. The best-fitting explanations were fast-core clock or heat soak, or driver object churn on the present thread. The log had no clock, power, memory or live-object data, so neither could be confirmed.

**Lesson.** When a log cannot distinguish two explanations, say so, and list the cheap counters that would. We added thread cycles, per-core frequency and live-object counts to the wish list instead of guessing.

## 3. Lock census

**Hypothesis.** Slow frames come from the presenting thread waiting on shared loader locks held by worker threads.

**What we built, in three passes.**

1. A stage-split observer divided the barrier's "stage-rest" residual into lookup, snapshot allocation, snapshot copy, trust, lock acquisition and rest. It also timed four selected locks per role.
2. The first lock observer kept a 256-entry event ring per thread. It dropped data almost immediately. The four locks released about 5,400 to 5,700 times per frame, so the ring overflowed every frame. We replaced it with fixed per-thread slots holding per-lock hold counters. A later rework that used the invariant TSC cut the cost from 84 ns to 25 ns per presenter lock pair.
3. A per-site and per-role table for the lifetime lock. It recorded each acquire site, each thread role, and who held the lock when the presenter began to wait.

**How we measured.** Rows on both machines with the lock observer on. We read the top (site, role) pairs of presenter wait in slow frames.

**Numbers.** On the desktop the presenter waited 1.04 ms per frame on the lifetime lock overall, and 3.44 ms in frames over 33.4 ms. On the handheld, the slow-frame wait was 5.66 ms (race 1) and 3.81 ms (race 2). The top holder pair was a resource release by a game thread: 50 % of the wait on the desktop and 54 to 58 % on the handheld. Pins from the head-start worker came next at 15 %. The census itself disturbed the result: the first version cost the handheld about 6 fps.

**Verdict.** The lifetime lock was a real, large contributor to presenter wait. Release calls dominated the holds.

**Review and what survived.** A design review, using these rows plus a code read, produced a ranked list of changes. A second reviewer then checked every claim with its own scripts. It found:

- The first reviewer's per-row figures mixed in the 60 % of each row that was not race laps. Only the race-lap baseline was valid: 18.0 ms per frame on the desktop and 20.1 ms on the handheld.
- On the desktop, the barrier chain (summed over all its parts) was as large as game-thread time in slow frames: plus 10.0 ms against plus 11.7 ms. Earlier tables had counted each staging part alone, so each looked small.
- The harness's own route sampler produced 27 to 32 % of the >33.4 ms frames and 71 to 74 % of the >50 ms frames inside race laps. This would later become the most important correction (see "Capture-frame correction").

**Lesson.** Always quote savings on the population you care about. Always check what the harness itself adds to the tail.

## 4. Release fast path

**Hypothesis.** Release calls on resources that no pin can ever reach do not need the lifetime lock. Skipping it should shorten the presenter's wait.

**What we built (first attempt).** `XBOX_RELEASE_FASTPATH` skipped the lifetime hold for title buffers, using a creation-time flag set before the resource became visible to other threads. `XBOX_PIN_DESC_OUTSIDE` moved a description query out of the pin's critical section. A reader fix also stopped false "overlap" warnings between windows that shared a cut.

**How we measured.** Five rows per machine: two off, two on, one with counters.

**Numbers (first attempt).** Nothing moved. Desktop >33.4 ms per minute: 96.7 to 92.5 (race 1) and 93.9 to 93.5 (race 2), against an off-to-off spread of 5.6 and 15.5. Handheld: 155.3 to 155.2 and 124.5 to 125.8. The fast-path counter read zero on every row.

**Why.** Our gate required a "first-use" feature to be off. But the function that answers that question is true whenever exact drawing is on, which is always so in the play configuration. The path never ran. The rows measured only the description change.

**What we built (second attempt).** We re-derived the gate with a reachability proof written before the code. We also added three more things:

- A negative filter, `XBOX_RELEASE_TILED_FILTER`: a table of about a million saturating one-byte cells indexed by a pointer hash. It let a release of a never-tracked resource skip the texture lock without reading the map. In the failing direction it falls back to the locked path. Automation builds re-read every 64th "absent" answer under the lock and raise a breach if one was wrong.
- A single lock hold for an exact release instead of three.
- A class split of release holds, so we could see which resources dominated.

**Numbers (second attempt).** The mechanism worked. 1.46 million fast releases fired. The filter answered "absent" 138 million times with zero misses. Presenter texture-lock acquisitions fell from about 1,443 to 303 per frame, and its wait from 0.28 to 0.12 ms. Desktop race 1 fell from 98.2 to 85.5 slow frames per minute (minus 13 %), against pair spreads of 2.9 and 1.4. Race 2 showed no clear change (minus 1.6 against a spread of 18.5). The handheld dropped from 161.3 to 129.9 per minute in race 1 (minus 19 %, with both on rows beating both off rows), and minus 12 % in race 2.

The class split exposed a surprise. Title buffers were 0.0 % of lifetime-lock release holds. 76 % were resources the loader had never registered, and 21 % were tiled textures. The lifetime-lock wait itself did not shrink on the desktop (3.24 ms per slow frame).

**Verdict.** A small real gain, probably from the texture-lock part, not the lifetime-lock part. These switches became the fixed baseline for every later row.

**What the second reviewer challenged.** The first reviewer had proposed the filter with a claimed 80 to 85 % cut in texture-lock acquisitions. The second reviewer corrected that to 60 % for the filter alone and 75 % with the single-hold change. It also found a lock acquisition we had missed in the forget path, and an insert-or-replace path the filter needed to count. It rejected one of the first reviewer's mechanisms: a head-start "pass pin" reuse that would have saved one pin out of six. It also required that the fast path never cover exact releases, and we enforced that at run time.

**Lessons.**
- Prove the switch fires before you spend rows on it. A counter that reads zero is an honest result, but a gate check at build time would have saved a night of rows.
- A bad gate can look like a null result from a good idea.

## 5. Hook create split

**Hypothesis.** Players described a hitch "every second or so". We guessed it came from bursts of title resource creation and memory calls that the loader wraps.

**What we built.** `XBOX_EXACT_CREATE_SPLIT`, an observer that timed hooked create and allocate calls per frame (VirtualAlloc, VirtualFree, placed resources, committed resources, command lists) with a native-versus-wrapper split. It wrote a per-frame side file. Cost was 3.3 ns per call off and 38 ns per call on.

**How we measured.** Desktop rows, uncapped, with the earlier switches on. A separate lock-census row split bare syscall time from loader work.

**Numbers.** Slow frames per minute were 86 to 95 while racing. Two off rows got stuck on lap 0 or 1 and showed 34 per minute. A parked car did not hitch, so slow frames came with racing activity. In race 1 the game thread spent about 24.7 ms per slow frame in hooked create and allocate calls, against 2.2 ms on average, which is 11 times. Per slow frame:

| call | ms | native part | loader wrapper |
|---|---:|---:|---:|
| VirtualAlloc | 6.6 | 5.0 | 1.5 |
| VirtualFree | 5.6 | 1.6 | 4.0 |
| placed resources | 2.8 | 0.6 | 2.2 |
| committed resources | 1.7 | 0.5 | 1.2 |
| command lists | 1.7 | 1.5 | 0.2 |

The presenter created almost nothing (0.001 ms per frame). Native time inside committed-resource creation was only 0.5 ms, so sub-allocating from pre-made heaps would have gained almost nothing. The syscall census gave 0.12 ms per frame for VirtualAlloc and 0.11 for VirtualFree. A periodic component of about 1.4 s showed in race 2 in all three observer rows, and in race 1 in one of three.

**Verdict.** Slow frames carried a lot of loader bookkeeping inside the allocation hooks. Causation was not proven: creation time correlated 0.72 with excess over 16.7 ms but only 0.30 with excess over 33.4 ms. The lock census doubled the slow-frame rate (166 per minute against 86 to 91), so we used it for lock numbers only.

**What the review process did.** We built no fix on this data, because the rule was "no guessed fix". Two reviews followed. A code review enumerated the trap registry's costs exactly. One VirtualFree under exact mode took the trap lock five times and walked the 111,000-span registry three times. The review also made a precision point that mattered later: the walk is O(log N + W), where W is the number of entries in a window behind the free range, not O(N). A past large registration also never shrinks that window.

**Lesson.** An observer that perturbs the hitch rate by 2x (the census) can still give valid ratios. It cannot give valid rates. Say which is which in the table header.

## 6. Trap exclusion lookup and hot/cold queues

**Hypothesis.** Two ideas from the previous steps:
- Fix A: an exact exact-key lookup could replace the trap-exclusion walk on allocation and free paths.
- Fix B: a hot queue for viewed textures and a cold queue for the rest, with a barrier that jumps queued entries, would let the head-start worker stage what the barrier needs first.

**What we built.** `XBOX_TEX_TRAP_EXCLUSION_FAST` (Fix A), `XBOX_TEX_STAGE_EARLY` (Fix B) and a diagnostic `XBOX_EXACT_STALL_SPLIT` that split each frame into title run, blocked, barrier and out-of-barrier blocked time. Fix A was exact because it only skipped walks that would find nothing. In Automation builds it rechecked one in 64 skips and raised a breach on any miss. Fix B kept a never-join rule: a pass that has been jumped is abandoned and the barrier stages inline. That means more staging, never an added wait.

**How we measured.** Two passes per arm on the desktop and the handheld: off, Fix A, Fix B, both, and the existing `XBOX_TEX_DRAW_EXACT_HS_EAGER=1`. Every row needed a strict lap check, because stuck cars produced several invalid rows.

**Numbers.** Desktop, >33.4 per minute, race 1 and race 2: off 85.9 and 92.4, Fix A 90.1 and 90.4, Fix B 85.1 and 92.3, both 93.1 and 94.7, eager 83.1 and 86.5. Pass-to-pass spread was 0 to 13. The handheld agreed: every difference was inside a spread of up to 25 per minute.

Counters proved the code ran. Fix A skipped about 705,000 to 716,000 walks with zero misses. Fix B still showed 49,000 to 51,000 hot entries stolen per run. Timers on the desktop: 370 slow frames with fixes on versus 361 off, frame 39.2 versus 39.1 ms, blocked 12.8 versus 12.8 ms, barrier 13.2 versus 13.9 ms.

**Verdict.** Neither fix moved any frame-time number. The loader does less work, but the hitch does not change.

**Review dynamics.** Fix B went through three independent reviews, requiring 11, then 5, then 3 changes. Examples: reference lifetimes of queued entries, bounded queues, and a rule that a delayed cold pass must not be treated as jumped. The first reviewer's premise for Fix B was overstated. Worker speed, not queue order, was the main limit, and Fix B was roughly a subset of the existing eager switch. The reviewers predicted at most a 1 to 2 ms gain per slow frame out of 9 to 17 ms of inline staging. The rows agreed.

The earlier code review of the trap work had recommended the smallest justified change: a shortcut only for provably empty repeated scans. It also warned that we needed matched moving off and on rows, since the stuck-car rows could not show a tail improvement.

**Lesson.** When review says "the premise is overstated", believe it and still measure, but do not spend three review cycles perfecting a change whose ceiling was predicted to be small. Estimate the ceiling first.

## 7. Retire-own

**Hypothesis.** A design review, working from the rows above, claimed the head-start worker was not the bottleneck at all: it was idle 87 to 88 % of the time in race laps. It said the real limit was lead time, close to zero. In its reading, the worker may only stage a texture after its first view, and the barrier follows almost at once. A frame with two textures already had 81 to 82 % of spans stolen. So it proposed a "stage-ahead" design that stages before first view.

**What a second reviewer found.** The second reviewer reran the evidence and refuted the central reading.
- The two-take frames were not first views. 85 to 95 % of stolen spans in them had already landed once before (the "mark" class). They were disarm-only re-checks. When a texture retires, the loader used to disarm its page neighbours' trap tokens too. That made the neighbour untrusted without any mark and without any head-start request. The barrier's own trap drain then created the request microseconds before its take. Those entries were stolen by construction. No queue order, worker count or pre-view staging could change that.
- A switch for exactly this already existed: `XBOX_TEX_DRAW_EXACT_RETIRE_OWN=1`. It makes retire disarm only its own tokens. It had passed its correctness gate days earlier but had never been scored for slow frames. Every earlier row ran with it unset.
- "Throughput is not the limit" held only on average. In burst frames with 17 or more takes, one worker is saturated.
- The stage-ahead design was exact-safe only with four required changes, and was not worth building before two no-build rows.

**What we built.** Nothing new. We ran the existing switch. The earlier correctness gate (desktop, audit on) had shown the mechanism: own tokens 738,832, foreign tokens 0, inline "equal after disarm only" down from 10,764 (control) to 12, and zero audit failures across about 3.1 million checks.

**How we measured.** Four arms in two passes on the desktop: off, retire-own, `XBOX_TEX_DRAW_EXACT_HS_PREVIEW=1`, and both.

**Numbers.**

| arm | >33.4 per min, race 1 and 2 | birth inline ms per slow frame |
|---|---|---|
| off | 89 and 97 | 5.6 and 9.0 |
| retire-own | 83 and 96 | 4.9 and 9.2 |
| preview | 112 and 86 | 7.5 and 6.9 |
| both | 100 and 86 | 8.5 and 7.2 |

With retire-own, mark staging fell from 1.9 to 0.08 ms per slow frame and stolen needs about halved. Hitches did not move. Preview made race 1 worse in both passes. A "lead time" row showed 33,913 of about 37,500 stolen needs in race 1 had a lead of 16 ms or more. The work had been queued in time, and the single worker had not got through it.

**Verdict.** Retire-own does what it claims and gives no measurable gain. Preview is worse. Do not enable preview.

**Lessons.**
- A coherent story with strong numbers can still read the wrong mechanism. The second reviewer's check against the code (who creates the request, and when) beat the aggregate statistics.
- Look for existing switches before designing a new one.
- The correct sequence was: run the no-build rows first, then decide whether to build.

## 8. Hook timing

**Hypothesis.** Perhaps a handful of the loader's hooked calls (release flip, in-place bundle build, flat copies) explain the extra game-thread time in slow frames.

**What we built.** `XBOX_TEX_DRAW_EXACT_HOOK_TIMING`, a 1-in-64 sampled timer on five hook families (Automation only). It showed no measurable cost: 55.8 against 55.6 fps and 89.2 against 89.0 slow frames per minute.

**Numbers.** Slow-frame excess over normal frames, per frame, race 1 and 2: release flip plus 0.98 and plus 0.48 ms (calls per frame rose from 839 to 2,760, about 3x), in-place bundle plus 0.85 and 0.25, submit flat-copy plus 0.42 and 0.26, title flat-copy plus 0.34 and 0.30, draw reach under 0.07. The timed hooks explained about 1.5 to 3 ms, or 15 to 25 %, of the extra 9 to 12 ms of game-thread run time in slow frames. The named locks covered about 3 of the 12 to 13 ms blocked. The remaining 9 to 10 ms of blocked time, and barrier time of 12 to 18 ms per slow frame, had no timer.

**Verdict.** No single loader hook is the hitch. The extra time sits in the barrier and in waits we could not see.

**Lesson.** A negative result from a timer family is progress only if you write down the share it did not explain. "15 to 25 % explained" told us exactly where the next instrument had to go.

## 9. Second worker

**Hypothesis.** The head-start worker is behind. A second worker, or a higher-priority one, will cut stealing and so cut slow frames.

**What we built.** `XBOX_TEX_HS_WORKER_BOOST`: value 1 raises the worker to normal priority; value 2 starts a second worker that runs the same pass code. A pass started by worker 2 is never joined. The barrier abandons it and stages inline, so the presenter never waits longer than today. Worker 2 runs no expiry scan and no trap drain, and waits on a separate condition variable. A failed worker-2 start counts as unknown, not a breach. A second reviewer approved it with four required changes.

**How we measured.** Four arms on the desktop (off, 1, 2, 2 plus eager), and three on the handheld.

**Numbers (real >33.4 per minute, capture frames excluded; see the next-but-two section), race 1 and 2.**

| arm | desktop | handheld |
|---|---|---|
| off | 52.3 and 61.5 | 91.1 and 85.0 |
| value 1 | 49.5 and 60.2 | not run |
| value 2 | 54.9 and 66.2 | 119.0 and 93.3 |
| value 2 plus eager | 40.2 and 50.7 | 91.9 and 77.5 |

Value 1 did nothing. Under Proton every Windows thread runs at nice 0 in the same scheduling class, so priority maps to nothing. Value 2 cut stolen needs per slow frame by 80 to 85 % (desktop race 1: 36.5 to 5.5). But it did not cut slow frames. On the handheld it made race 1 clearly worse: zero-need heavy frames rose from 11.6 to 28 per minute, which looks like CPU contention on the 8-core part. Value 2 plus eager was the one real gain: minus 23 % and minus 18 % on the desktop, with an off a-to-b spread of only 1.9 and 2.7.

Two later confirmation rows agreed. Desktop real slow frames were 42.7 and 54.8 against 53.8 and 65.5, with no fps cost. On the handheld at a measured 15.0 W in-game APU draw, real slow frames were 98 and 70 against 106 and 88, and frames over 50 ms fell from 15.3 and 15.8 to 12.8 and 11.6 per minute. Fps stayed within one.

**Verdict.** The stealing count is not the hitch driver. Value 2 alone is neutral on the desktop and harmful on the handheld. Value 2 plus eager is a modest real gain, and the player adopted it with that pairing. Never value 2 alone.

**Gotcha.** The handheld's firmware power limits read 35, 37 and 45 W, but the GPU power sensor read 15.0 W in game. Check measured power, not configured limits.

**Lesson.** Predicted effect sizes matter. We had predicted minus 5 to minus 15 % in slow frames for value 2 and got plus 5 to plus 31 %. We had predicted stolen needs would fall 30 to 50 % and they fell 85 %. A mechanism that fully works while the outcome does not move means the mechanism is not on the critical path.

## 10. Wait split and the Linux sampler

**Hypothesis.** About 9 to 10 ms of each slow frame was "blocked" with no timer. We guessed waits on GPU fences, sleeps or kernel objects.

**What we built.** `XBOX_EXACT_WAIT_SPLIT`. It recorded the presenter's activity word, timed three forwarded critical-section and condition-variable imports, wrote per-frame marks, and ran a roughly 1 kHz sampler thread that did all file I/O. The plan was to read per-thread CPU, run-queue and sleep state from `/proc` inside the process.

**What went wrong.** Under Wine, opening a regular file goes through the wine server. The `thread-self` path therefore resolved to the wine server's thread. Every CPU, run-queue, syscall and sleep field described the wrong thread. We discarded those fields and used a Linux-side sampler joined to the valid parts of the switch's output.

**How we measured.** Six rows on the desktop, with counted rows all passing the strict lap check. The sampler cost about 8 % of one core (79 ms per second).

**Numbers.** The presenter thread was identified exactly: Linux CPU per frame 14.90 ms against the switch's 14.90 ms, correlation 0.90. A normal frame (14.8 ms) was 11.6 ms on CPU, 0.08 ms run-queue and 3.2 ms blocked. A slow frame (38 to 39 ms) was 25.8 to 27.1 ms on CPU, 0.25 to 0.32 ms run-queue and 11.8 to 12.8 ms blocked. Of about 24 ms of excess, 14 to 15 ms was CPU.

By class:
- **Streaming frames over 33.4 ms:** frame 39.2 to 40.1 ms, CPU 28.6 to 29.9. Barrier stage-rest was 6.9 to 9.4 ms running. Head-start joins blocked 1.4 to 3.1 ms. Loader lock acquisition cost 1.7 to 2.9 ms running plus 1.0 to 1.9 ms blocked.
- **Burst frames over 33.4 ms:** CPU only 20.5 to 21.0 ms, blocked 15.8 to 17.2 ms. A contended title critical section took 6.2 to 9.8 ms and a title wait on a kernel object 2.0 to 3.3 ms.

Run-queue time was under 0.35 ms in every class.

**Verdict.** The earlier "blocked 12 to 13 ms" totals were right but misread. They came from a CPU clock with 10 ms ticks under Wine, which made CPU work look like waiting. Pinning and priority levers are dead on the desktop. Streaming frames are CPU frames. Burst frames are waiting frames.

**Lessons.**
- Verify what an in-process observer reads under a compatibility layer before trusting it. A cross-check (Linux CPU equal to internal CPU) is what exposed which columns were valid.
- Coarse tooling clocks create misleading categories.

## 11. Capture-frame correction

**Hypothesis.** A design review looked at why the second worker cut stolen needs but not the headline slow-frame rate.

**What we found.** Our harness sampled a route position every 2 s. Every sampler frame was over 33.4 ms. In race 1 that was 157 to 159 frames, and in race 2, 78 to 86. That is about 30 slow frames per minute of pure harness cost. Against a headline of 83.6 per minute (race 1, off), 53.2 were real and 30.5 were capture. The harness had contributed 36 % of the headline we had judged every earlier step against. The play build has no sampler.

The review also split the real slow frames into three classes:
- **Streaming frames** (barrier needs at least one): about two thirds. Race 1 off: 154 of 284 real slow frames.
- **Submission bursts** (30 or more submissions, zero needs): the title submits about 75 extra command lists with no extra barriers. There were 190 episodes in race 1 and 106 in race 2, at the same route seconds in every arm. 104 of 284.
- **Zero-need frames:** 26 of 284.

Streaming frames follow bursts: they are 9 to 15 times more likely 2 to 5 frames after a burst, and never just before. A streaming step is a title burst, then first use of the new textures a few frames later. Head-start work touches only the streaming class.

We also learned that the desktop presents with vsync on. An ordinary frame has about 3 ms of slack. Any saving under that is invisible there, and the frame-time histogram has no cluster at 33.4 ms; slow frames are the tail of a broad band.

**What the second reviewer did.** It confirmed the 30 per minute (all 157 to 159 sampler frames were over 33.4 ms, minimum 36 to 38 ms). It weakened one claim: excluding "the frame after" each capture has no support in the data, though it costs only 0.2 to 1.9 per minute. It confirmed the class partition and found it stronger than reported: burst slow frames per minute were identical across submission thresholds of 22, 26, 30 and 40. It also corrected a conclusion. The extra burst and zero-need frames under value 2 alone are not run-to-run spread. Pass B repeated them at 32.2 and 34.7 per minute against 21.6 plus or minus 3.3 for eleven one-worker rows. The cause is unknown.

**Verdict.** From then on, every number was quoted as "real" slow frames, with sampler frames excluded and split by class. Several earlier verdicts were not wrong, but their effect sizes had been judged on a metric that was 36 % harness. The one-worker baseline over eleven rows was 52.0 plus or minus 3.1 and 62.4 plus or minus 3.0 per minute. The best second-worker arm sat at least 3.2 standard deviations below.

**Lesson.** Audit the harness's contribution to your headline metric at the start, not in the eleventh step. The fix cost nothing: a reader flag.

## 12. Burst lock

**Hypothesis.** In burst frames the presenter blocks on a contended title critical section for 6 to 10 ms. A design review argued that the loader's VirtualFree hook, running on one title thread, made that lock's hold long. It had measured a hooked VirtualFree at about 211 us per call, of which the bare syscall was about 35 us. The remainder was three trap-registry walks, two VirtualQuery walks, marks and about eight token disarms.

Evidence for it: the presenter's lock wait correlated 0.76 and 0.90 with that thread's VirtualFree time, and 98 to 100 % of burst slow frames had over 1 ms of contended wait. It proposed `XBOX_EXACT_ALLOC_LEAN`: skip empty walks, reuse one allocation walk, and skip the protect-back for tokens wholly inside a range being freed.

**What the second reviewer found.**
- The correlation used 31 and 6 frames. 140 and 54 were usable. On all of them it was 0.53 and 0.06.
- Other title threads also contend on the same lock in burst frames with zero VirtualFree calls. There each entry costs about 90 to 109 us. With 40 to 80 VirtualFree calls it is 150 us. So the hook adds at most 20 to 40 us per entry.
- One presenter wait is 1.5 to 1.9 ms on average and up to 17 to 19 ms, which spans 7 to 9 hook calls. "Blocked for the length of those hook calls" is not what happens.
- Part 2 of the fix was not exact. Skipping the restore leaves a per-page "restorer" thread id set. After a re-commit, that thread can later set a shared page read-write while another token still arms it, which is a silent trap miss. 61 % of arms are partial-page spans, so page sharing is the normal case. The change also opens a read-only window that turns a title write that succeeds today into an unhandled fault. Part 2 was withdrawn.
- Part 1's empty-walk skip already existed as Fix A.
- A proposed lever for leftover inline staging, "second worker plus preview", cannot run as written: value 2 starts the second worker only with preview off.

**How we measured.** A no-build test. Fix A off against on, with wait split on, on top of the second-worker-plus-eager baseline, two passes.

**Numbers.** Fix A had no hitch effect: 69.6 and 87.6 against 71.3 and 85.1 per minute (inside spreads of 1.7 and 10.4). Presenter contended-entry wait per burst slow frame moved 7.4 to 8.1 ms in race 1 and 10.5 to 10.0 ms in race 2. Fix A cut other threads' VirtualFree hook time (10 to 16 ms per frame) only 5 to 20 %. So the hook is not decisively the hold. We could not prove the lever dead, because Fix A removes only part of the hook's work.

**Verdict.** Unproven. The next step is an observer, `XBOX_EXACT_LOCK_OWNER`, that records the contended lock's address, its owning thread, and whether the owner is inside a loader hook at the moment of contention. That separates "loader time" from "title behaviour". At the time of writing, that build was in progress.

**Lesson.** Correlation inside a frame does not order events. Before designing an exact fix, record who holds the lock. The cheap no-build test told us the proposed lever might shorten only a fraction of the hold.

## What we would do differently

1. **Measure the harness first.** A route sampler added 30 slow frames per minute, 36 % of the headline. We needed ten steps to find it. At the start, take a row with the harness quiet and diff the tail.
2. **Define the population before the first row.** Real slow frames, race laps only, sampler frames excluded, split by streaming, burst and zero-need class. We quoted savings on mixed populations for weeks.
3. **Prove a change fires before spending rows on it.** The first release fast path never ran because of a gate that was always false. A counter check on the first offline fixture, with the real play flags, catches that.
4. **Estimate the ceiling before building.** Several fixes had predicted ceilings of 1 to 2 ms against 9 to 17 ms of inline staging. A static counterfactual (shorten each frame by the removed time, change nothing else) should come first. One such table showed that even deleting all inline staging leaves a slow frame near 31 ms.
5. **Ask whether a mechanism is on the critical path, not just whether it works.** The second worker cut stolen needs 85 % and barely moved hitches. The count that changed was not the count that mattered.
6. **Search for existing switches and prior rows before designing.** Both the retire-own switch and the empty-walk skip already existed, unscored and unused, when a new design was proposed.
7. **Validate observers under the real runtime.** An in-process `/proc` sampler read the wrong thread under Wine. A 10 ms CPU tick made CPU look like waiting. Add a cross-check that two independent measurements agree.
8. **Treat review rebuttals as data.** Each design review produced strong, coherent, mostly well-sourced claims. Each second review then reduced them: figures corrected from 80 to 60 percent, a correlation from 0.9 to 0.06, a mechanism refuted by a code read, an exactness defect found. Ask the second reviewer to run holdout tests the first did not, and require the first to run them before reporting.
9. **Fix invalid rows by rule, not by hand.** Stuck cars invalidated many rows. A strict lap check with a spare row for each arm kept the sets comparable.
10. **Keep observers cheap and off by default.** Several observers perturbed the thing they measured (the census doubled slow frames; the first lock observer cost 6 fps on the handheld). Report ratios from perturbing observers, rates only from quiet ones.
11. **Spend rows where the spread is smaller than the effect.** Pass-to-pass spreads of 5 to 25 slow frames per minute swamped most 5 to 10 % effects. Use paired rows and three passes before declaring anything real.
12. **Look at the real target device early.** The handheld at 15 W reacted differently from the desktop: more CPU contention from an extra thread, and no vsync slack. A change that helped one machine hurt the other.

---
[Back to the index](../README.md)
