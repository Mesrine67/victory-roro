# 37. Optimisation passes and the handheld

## 37.1 What this chapter is

Section 5 describes the exact texture path: before each submission, every texture span the title can reach gets a GPU copy of bytes that were not dirtied after the read. That path is correct, and it is expensive. Right after it landed, the CPU cost was large on every machine and punishing on a handheld with a fixed power budget.

This chapter covers the series of overnight passes that cut that cost. Each pass changed speed only. None was allowed to change which bytes a texture holds at a submission. For every pass, this chapter gives:

- the idea, in generic terms;
- the measured effect on each machine, with numbers;
- whether it became a default, and why or why not;
- the lesson for someone building a similar translator.

Then it covers the handheld in particular (power, core topology, memory, thread priority, frame pacing), and the method by which a switch earned default-on status.

Switch names keep the `XBOX_*` prefix. Title-specific names are written `<TITLE>_*`.

### The machines

| name used here | hardware | OS and driver stack |
|---|---|---|
| the AMD handheld | Ryzen Z2 Extreme (3 Zen 5 cores, 5 Zen 5c cores), Radeon 890M, shared system memory | SteamOS, Proton, Proton's own vkd3d-proton 3.1 |
| the RTX 4090 desktop | 8-core X3D desktop CPU, RTX 4090 | Windows, vkd3d-proton 3.0.1 from our own folder |
| the RTX 3080 laptop | Ryzen 9 6900HS, RTX 3080 laptop 8 GB | Windows |
| the 5900X desktop | Ryzen 9 5900X, 32 GB RAM, RX 6700 XT | Windows |
| the AMD desktop | Ryzen 7 5800X3D, RX 9070 XT | Linux and Proton (used mostly as a build and test node) |

Most of the data below comes from the first four. The handheld is the machine where every millisecond mattered, so it is the reference for "does this pass pay?".

### Reading the numbers

- "fps" for a race is a median over the race part of a run, from the race clock start to the end.
- A "valid run" is not stuck (lap-1 time under 300 s), has no early device loss and was not disturbed by other work on the same PC.
- Many arms have n = 1 to 3 valid runs. Where a result rests on one run, the text says so.
- "Test build" means a build with extra checks and differential modes. "Shipping build" is what a player runs.

## 37.2 The starting point: where the time went

Three profiles, one per machine class, framed everything that followed.

**The RTX 4090 desktop.** The hottest title thread was the presenting thread, about 80% busy. With the exact path on, it cost roughly 2 ms per frame more than with it off (55.2 vs 62.1 fps in profiled runs, so 18.1 vs 16.1 ms). The extra time was not one function. It was spread across waits (+1.4 ms), our own bookkeeping, lock traffic and descriptor tracking. A scalar zero scan that the old path ran cost 1.9 ms per frame here.

**The RTX 3080 laptop.** The GPU direct-queue span (the wall time between two timestamps on the direct queue) grew from 25.3 ms to 44.9 ms at the start of a race. That number is not GPU busy time. It includes idle gaps where the queue waited for the CPU. A fit over 1,912 rows split the extra 19.6 ms into three parts:

- about 8 ms tied to roughly 40 extra upload submissions per frame (0.27 ms of span each);
- about 8 ms tied to time the title thread spent inside our submit path (0.77 ms of span per ms), meaning the queue starved while the barrier staged textures on the title thread;
- about 3 ms from extra draws, because the pacing clock ticked more often (see 37.6).

**The AMD handheld (15 W).** This profile mattered most. Fix on vs fix off, matched second race:

| quantity | fix off | fix on |
|---|---|---|
| fps | 39.3 | 37.0 |
| frame time | 25.45 ms | 27.01 ms |
| whole-process CPU per frame | about 95 ms | about 117-123 ms |
| busy cores | 3.7 | 4.4 |

The exact path added about 0.75 to 1 core of CPU, almost all on our helper threads. The present thread itself used less CPU with the path on, because the old upload path ran inline there. But it waited more: its waits on lock and condition variables rose by 3.1 to 3.9 ms per frame. Frame time went up only 1.6 ms because the other changes partly cancelled.

The single largest item was a surprise: **SHA-256 hashing of texture snapshots ran through Wine's portable bcrypt code.** It used about 300 ms of CPU per second (30% of a core), roughly 8.4 ms per frame spread over three kinds of thread. Throughput was only 115 to 135 MB/s. On Windows the same call uses the CPU's SHA instructions, so this cost never showed up on the desktops.

Two independent sources agreed on the number: the profile's bcrypt samples (8.4 ms per frame) and the loader's own hash-time counter (9.05 ms per present).

The profile also showed that the GPU was not the limit in light race sections. The present thread was busy or stalled for over 90% of the frame. In heavy sections (many draws) the GPU took over. The GPU trace confirmed that the exact path adds no GPU work on the handheld (22.6 vs 22.5 ms busy per frame). All its cost is CPU-side, so CPU-side passes were the right lever.

## 37.3 The passes

The passes are named A, B, C and so on, in the order of the narrative, not the order they ran.

### Pass A: resolve bundle inputs in place

**Idea.** The title replays bundles (pre-recorded command lists). The old path copied each bundle's inherited state, applied the resolve, and threw the copy away. The new path walks the bundle and resolves the descriptor tables it reaches directly on the caller's state.

**Switch.** `XBOX_TEX_DRAW_EXACT_BUNDLE_INPLACE`. A value of 0 restores the copy-and-apply path.

**Effect.**

| machine | result |
|---|---|
| RTX 3080 laptop | about 3.1 to 4.0 ms per frame less; measured, 0 mismatches in about 20 million checks |
| RTX 4090 desktop | about 2.4 ms less in the race (50.1 fps and 20.0 ms vs 44.7 fps and 22.4 ms); the two runs drew different numbers of calls (640 vs 730), so this is a rough figure |

**Default.** Yes, on whenever the exact path is on. It was the first pass to land, and it had a direct before-and-after measurement.

**Lesson.** The cheapest big win was removing a copy that existed only to keep the old code simple.

### Pass B: lean logging in the shipping build

**Idea.** Per-event log lines (one set per present after the first breach, plus per-texture rows) are valuable in test builds. They are waste in a shipping build.

**Switch.** `XBOX_LOG_DETAIL`: 0 in the shipping build, 1 in the test build, so test output did not change.

**Effect.** The handheld's fix-on run wrote a 96 MB log with 377,000 lines, nearly all from the present thread. Formatting cost was small (about 0.2 ms per frame), but it also removes burst cost at loads and scene cuts. Estimated 0.05 to 0.1 ms per frame on average, several ms less title-thread work near loads. No clean arm isolated it.

**Default.** Yes (0 in the shipping build).

**Lesson.** Decide early which diagnostics ship. A diagnostic that scales with the number of textures becomes a real cost once the title streams tens of thousands of them.

### Pass C: cache the GPU-heap lookup

**Idea.** Memoise `find_gpu(handle)`, the lookup from a descriptor handle to its GPU heap, with an epoch bump whenever the heap registry changes.

**Switch.** `XBOX_TEX_DRAW_EXACT_FIND_GPU_CACHE`.

**Effect.** A micro-benchmark: 19.7 to 22.2 ns per lookup uncached, 8.4 to 9.1 ns cached. Per frame this is far below 1 ms.

**Default.** No. The gain was too small to justify a cache that must be invalidated correctly.

**Lesson.** A 2x speedup of something called a few thousand times is not worth an invalidation rule.

### Pass D: a bundle memo that would never hit

**Idea.** Skip a bundle if its inputs are unchanged since its last resolve.

**Finding.** Before building it, we read the old copy-and-apply path's own cache, which already keyed on the same inputs. It had measured **8,796,288 misses and 0 hits** over a run. Bundles were reused (about 300 executions each), but the inherited state differed on every execution, probably because of the caller's table handles. So a per-bundle memo had a hit rate near zero, however exact.

What does repeat is the resolve point across bundles: about 11.5 resolves per bundle, but only 135,986 of 43.4 million bundle executions added a table their caller did not already hold (0.3%).

So the pass memoised one resolve point instead, keyed on everything the call reads: root and pipeline objects, the needs, the missing-table index, the heaps, the caller's handles, a registry epoch and liveness of every heap found.

**Switch.** `XBOX_TEX_DRAW_EXACT_BUNDLE_MEMO` (1 = memo; 2 = differential, test build only, keeps the uncached result).

**Default.** No. It never ran in a game, and later profiles showed bundle resolve at about 0.1 ms on the present thread.

**Lesson.** Read the existing cache's hit counters before designing a new one. The cheap experiment (a counter) killed the obvious design.

### Pass E: recycled upload staging, layout cache, vector zero scan

Three small ideas behind three switches: a pool of persistent upload buffers (`XBOX_TEX_UPLOAD_POOL`; about 940 committed upload buffers were created and destroyed per second in a fix-on race, about 200 fix-off), a cache of texture layouts (`XBOX_TEX_LAYOUT_CACHE`; the 24 µs "layout" stage was mostly the snapshot, and the footprint call itself took 0.7 µs), and an exact SSE2 zero scan (`XBOX_TEX_SCAN_FAST`; 240,240 random trials, 0 differences). None was measured as a separate arm in a game, and none became a default. The zero scan targeted a cost only the RTX 4090 desktop showed (1.87 ms per frame; the handheld showed 0.04). Lesson: a profile from one machine does not transfer, and the handheld's "layout" cost was Wine virtual-memory calls, not footprint maths.

### Pass F: descriptor tracking, fast moves and a dense table

**Idea.** Every descriptor copy updates a per-heap table of slots. The old table was an ordered map: three red-black-tree descents per copied descriptor, each missing the cache at most levels, plus refcount traffic from copying a slot object of 320 to 336 bytes. The title creates a 450,000-slot CPU heap and a 1,000,000-slot shader-visible heap, so the maps are big and cold.

**Two switches.**

- `XBOX_DESC_TRACK_FAST`: 1 = move slots instead of copying them, and cache a CPU-range lookup; 2 = moves only.
- `XBOX_DESC_TRACK_DENSE`: 1 = a three-level dense table (directory per 64 slots, 64-entry index pages, pooled slot chunks); 2 = map and dense table side by side, compared on every operation, test build only.

**Size of the target.** About 8 to 9% of the hot thread's leaf samples on the 4090 desktop, and 12.5% (2.27 ms of an 18.1 ms frame) in a fix-on profile. 2.2 to 2.4 ms per frame on the handheld's present thread, about the same with the fix off.

**Fixture numbers** (one thread, game-sized heap):

- one lookup: about 180 to 190 ns (map) vs 10 to 16 ns (dense);
- one slot write: about 180 to 195 ns vs 40 to 50 ns;
- 25 to 42% less time per copied descriptor in the test-build defines, 11 to 32% in the shipping defines (no identical-slot walk there);
- memory: 41.5 MB dense vs about 48 MB map at game size, worst case (every slot live) about 340 MB vs about 400 MB.

**Predicted vs measured.** The pass predicted +1.5 to +2.5 fps at a CPU-bound race start. Measured:

| machine | with vs without (valid runs) | gain |
|---|---|---|
| AMD handheld | 38.8 fps (n=3) vs 31.6 (n=5) | +7.2 fps, +23% |
| RTX 4090 desktop | 70.4 (n=1) vs 62.4 (n=3) | +8 fps, +13% |
| RTX 3080 laptop | never run | |
| 5900X desktop | both runs stuck | no timing |

The measured gain was several times the prediction. In a game the walks run over larger, colder maps than in a fixture, and at 15 W cache misses cost more.

**Exactness evidence.** Fast mode audit on the 4090 desktop: 0 breaches in 1.12 million checks. Dense differential mode: 0 mismatches in 116 million checks (4090 desktop) and 132 million (5900X desktop).

**Default.** Yes (fast=1, dense=1 in the shipping build).

**Lesson.** The right fix for a map that is walked constantly is not a faster map, it is no map. This was the largest honest win of the series on the handheld.

### Pass G: batched upload submission (not promoted, with a hang)

**Idea.** At the RTX 3080 laptop's race start the fix-on path made about 40 upload submissions per frame (about 22.5 from the barrier, about 18 from the uploader clock), each its own `ExecuteCommandLists` plus a `Signal` on the direct queue. Batch the jobs of one pass into one call and one signal: about 4 submissions per frame instead of about 40. Each job keeps its own command list, its own barrier pair, and the same failure path.

**Cost model.** Measured on the title thread: 5.55 µs per `ExecuteCommandLists`, 0.41 µs per `Signal`, about 35 µs total per job (collect 11.6, command object 2 x 2.5, record 4 x 1.6, submit 3.2, landing 1.5, epoch 0.6). Batching removes about 36 of about 40 boundaries but keeps the per-job barrier pairs. On the 4090 desktop the expected gain was small (about 0.13 ms per frame of title-thread CPU). On the laptop the expected gain was up to about 8 ms of queue span.

**Switch.** `XBOX_TEX_UPLOAD_BATCH` (2 = batch plus checks, test build only).

**Outcome.**

- On the AMD handheld, all three runs with the batch switch showed one single-frame stall of 50 to 80 seconds at about 120 to 150 seconds into the run. The two that ran longer showed a second stall of 7 to 10 seconds. Complete runs of the same stack without the batch switch had no frame over 5 seconds.
- On the 5900X desktop it hung in all four runs, about 30 seconds into loading, and once lost the device on the handheld.
- Batches had reached 4,470 members (5900X desktop) and 53,405 members (handheld) in one call. The leading hypothesis was the member count, but it was never proven.

**Later follow-up (not yet measured in a game).** A hard cap of at most 64 members per batch, a cap on the "collect" sweep (run it every n-th job instead of every job), recording several jobs into one list per group, a readable-range cache, and a merge of neighbouring page re-arms. All were built behind switches, fixtures pass, and a design review found no exactness blocker. Status: the order for the first test was the 5900X desktop (where it hung), then the handheld, then the 4090 desktop.

**Default.** No.

**Lesson.** Batching command lists on the submission queue is a hang risk on AMD under Proton. Cap batch size from the start and test on the weakest driver first. And note the risk of the generic flush: a pass that ends without its flush must reset thread-local state, or the next pass inherits a dead list.

### Pass H: shorten the submit-gate hold, take statistics off the upload lock, optional priority boost

**Idea.** The title's presenting thread and our uploader-clock thread share one submission mutex. The title holds it for its whole barrier pass. The uploader's own tick held it while it did work that did not need it.

First we corrected an earlier reading. The uploader's "drain time" counter included its wait for the mutex, so the numbers quoted as "hold" were upper bounds. The real hold was small: below 0.1 ms per frame on the 4090 desktop.

**What the title feels** (presenting thread's lock wait per frame, fix on vs fix off):

| machine | fix on | fix off |
|---|---|---|
| AMD handheld | 1.34 ms median (1.74 mean) | 0.54 |
| RTX 4090 desktop | 0.22 | 0.07 |
| RTX 3080 laptop | 1.1 | 0.14 |

**Design.** `XBOX_TEX_SUBMIT_HOLD`, one character from 1 to 7:

- bit 1: do the copy lane and slot collection before taking the mutex, and prerecord command lists under a prediction that must equal the real record field by field (otherwise discard and record normally);
- bit 2: raise the uploader thread from below normal to normal priority only while it holds the mutex, and restore on every exit;
- bit 4: write the counters row only.

`XBOX_TEX_STATS_FREE`: update upload statistics without the upload lock (no decision reads them).

**Effect.** No standalone A/B ever ran on any machine. The stack of hold, stats-free and core pin on the handheld gave 40.6 fps (n=1) vs 38.8 (n=3), +1.8. A prerecorded job that is discarded costs about 10 µs of uploader CPU, which on the handheld comes out of the same 15 W.

**Default.** Yes (hold = 3, stats-free = 1) on the strength of exactness plus the stack result. This is among the weakest evidence in the table (see 37.8).

**Lesson.** A lock "hold" counter that includes wait time is an upper bound, not a hold. Name what a timer starts and stops before building on it.

### Pass I: pin the hot thread to the fast cores

Covered in 37.5 (core topology). Result: +0.3 fps on the handheld (39.1 vs 38.8, n=1, noise), noise on the 5900X desktop (a no-op on a uniform CPU). Default on, because it does nothing where it cannot help.

### Pass J: snapshot backing lease (a correctness fix that was disguised as a breach)

**Finding.** Audit logs showed repeated "snapshot-read" breaches. Every one, in six runs on three machines, had the same shape: a direct snapshot read overlapped a title decommit (the title's `VirtualFree(MEM_DECOMMIT)`, with its pre-mark and decommit running inside our copy). Our pin held the resource but not the memory behind it. Capture sequence was below the post-map sequence, so the copy had straddled the change.

**Design.** The title's address space is cut into 64 KB regions; each region maps to one of 1,024 stripe locks. A snapshot copy takes every stripe of its span shared; a mapping change takes its stripes exclusive, always in ascending order. Every reservation gets an identity and an epoch; each mapping change bumps the region epochs before it releases its stripes; a capture stores a hash of the identity and region epochs over its span; acceptance compares them.

**Switch.** `XBOX_TEX_SNAPSHOT_LEASE` (2 and 3 add a mutation log, test build only). Any non-zero value also splits the coarse "snapshot-hash" breach into precise reasons.

**Effect.** Correctness, not speed. 4090 desktop audit: 0 breaches in 30,945 mutation-log rows. 5900X desktop audit: 0. Shipping rows: 0. Speed was indistinguishable from noise (+0.2 fps on the handheld, +1.1 to +3.8 on the 4090 desktop depending on the baseline).

**Default.** Yes.

**Lesson.** When breaches share one shape, find the mechanism before tuning anything. Several "performance" runs were failing a gate because of one lifetime race.

### Pass K: head-start tickets (eager and refresh)

**Idea.** The head-start worker stages a texture's bytes off the title thread before the barrier needs them. Three behaviours cost us:

- a request for a texture already in the work map was dropped if an entry existed, even if the entry was running or completed;
- a completed entry lingered about 1 second;
- an empty pass left an entry that scheduled nothing, so the barrier staged inline.

On the 4090 desktop, 173,501 of 196,843 passes were empty. On the handheld the head start was saturated: 387 requests per second dropped as busy, against 132 used and 90 stolen spans per second (spans the present thread did itself).

**Two switches.**

- `XBOX_TEX_DRAW_EXACT_HS_EAGER`: erase completed-empty entries so a later request is not suppressed.
- `XBOX_TEX_DRAW_EXACT_HS_REFRESH`: advance a per-entry revision on every request. At completion, if a newer revision was wanted, queue a successor pass (new object, same key, FIFO so no texture starves the others) that carries the earlier tickets and keeps the ones still current. A ticket records the span, capture sequence, bytes, lease signature and the prepared job.

**Effect.**

| switch | machine | result |
|---|---|---|
| eager | AMD handheld | +1.7 fps, +5% (31.6 n=5 vs 29.9 n=1) |
| eager | RTX 3080 laptop | +2.0 fps (31.1 vs 29.2) |
| eager | RTX 4090 desktop | one device loss at 258 s (that machine lost fix-on shipping runs sometimes without this switch too) |
| refresh | AMD handheld, median fps | no gain |
| refresh | RTX 4090 desktop, hitches | frames over 50 ms in the race: 51 per minute to 7.2 per minute, worst frame 163 to 106 ms, race fps 88 to 101 (one run) |

The repeat of the refresh test, run when the machine was slower (a remote desktop session was costing about 2 cores), still cut frames over 50 ms per minute by about a quarter (92.1 to 68.9, medians of 3 vs 3) and halved frames over 100 ms (22 to 12).

**Default.** Eager: yes, with the weakest evidence of the ten promoted switches. Refresh: not promoted at the time, because the first verdict used median fps on the handheld, and refresh does not move the median. The hitch study (37.7) showed it was the cure for in-race hitches, so it moved to the top of the "to promote" list.

**Lesson.** Judge a fix by the metric the player feels. A switch that does nothing for the median can cut the long frames by 7x.

### Pass L: hardware SHA-256 for exact hashes

**Idea.** Replace the OS SHA-256 with one that uses the CPU's SHA instructions, at every exact-path hashing site. SHA-256 is a fixed function, so identical digests leave every comparison and decision unchanged.

**Switch.** `XBOX_TEX_HASH_FAST`. Values are cumulative so each part can be measured as a pair:

| value | adds |
|---|---|
| 1 | SHA extensions (scalar fallback, CPUID check, start-up self-test with fallback to the OS hash) |
| 2 | test build only: use the OS hash and also compute ours on every call, compare, and raise a breach on any mismatch |
| 3 | hash the private snapshot in place, with no 4 KB read-copy |
| 4 | fold the all-zero test into the hash loop |
| 5 | charge the snapshot from the ranges just committed instead of a `VirtualQuery` walk (a statistic only) |

**Safety.**

- One implementation per process, chosen once, used by every producer. A stored hash and a fresh one always come from the same function.
- The self-test runs at every start with the switch on: the standard example messages, one million `a`, plus hundreds of random buffers (lengths 0 to 64 KiB, offsets 0 to 63) whole and split. If anything fails, the loader keeps the OS hash and says why.
- Fixture: digests equal the OS's on about 47,500 buffers (about 233 MB) and every production site; a corrupted constant raises the mismatch breach while the OS digests stay in use.

**Effect.**

| machine | with vs without | verdict |
|---|---|---|
| AMD handheld | 43.7 fps (n=1) vs 40.05 (n=2), +3.6 fps, +9%; later runs with other changes added 44.4 to 44.8 | real gain |
| RTX 3080 laptop | 28.3 vs 29.2 (n=1 vs n=4) | noise |
| 5900X desktop | 48.3 vs 47.3 | noise |
| RTX 4090 desktop | suspect (other work on the PC) | |

The Windows machines were neutral, as predicted: Windows already hashes with the SHA instructions. The handheld estimate (about 7.5 ms of CPU per frame freed, about 0.27 core) was met in direction; fps rose 9%.

**Audit.** Mismatch 0 of 4.57 million checks on the handheld (SHA instructions executing under Wine), 0 of 2.88 million on the 5900X desktop. The 4090 desktop's audit lost the device at 353 s with 0 mismatches up to then.

**A bug found by the audit.** The "start row" and the self-test breach were logged once per texture creation, about 90,000 rows per run. They were fixed to once per run.

**Default.** Yes (value 5).

**Lesson.** The same portable-C-in-Wine pattern probably hides in other OS wrappers (compression, CRC, crypto). Profile Wine builds for them. And a pass that is neutral on Windows can still be the biggest win on Proton.

### Pass M: snapshot pool and environment cache

**Snapshot pool** (`XBOX_TEX_SNAPSHOT_POOL`). Every exact snapshot job reserved, committed span by span, and released its private copy. Under Wine each virtual-memory call takes the process-wide VM lock, and the profile showed about 2.5 ms of CPU per frame on the head-start and texture-pool threads (free 0.97, allocate 0.56, query 0.79, protect 0.21). The pool keeps released reservations with their committed pages in power-of-two classes (64 KiB to 64 MiB), at most 8 per class and 32 MiB in total.

Exactness argument: before a job reads a byte, a reused block equals a fresh reservation. Pages outside the job's commit set are decommitted at acquire, missing ones are committed by the job's own calls, and a reused page is zeroed except bytes the caller fills before any reader.

Fixture: 3,000 random call patterns, 6 threads for 2 seconds (about 10,000 jobs, 0 bad), three mutants (no zeroing, no decommit, no commit) caught in 325 of 400, 394 of 400 and 208 of 400 jobs. Per-op benchmark: fresh 3.7, 6.7, 19, 42, 125 µs vs pooled 0.33, 0.43, 1.0, 4.2, 17 µs for 4 KiB, 20 KB, 64 KiB, 256 KiB, 1 MiB.

**Environment cache** (`XBOX_ENV_CACHE`). Four sites in per-call wrappers (barrier, reset, submit, present) read environment variables on every call. The audit estimated 1,500 to 3,000 reads per frame on the present thread (1 to 4 ms under Wine; not measured). Cache the first value; mode 2 re-reads and compares on every call and raises a breach on any change.

**Effect.**

- Pool alone, handheld: 44.9 vs 44.8 fps (n=1 each), stall 2.56 vs 2.83 ms per present, head-start CPU 2.74 vs 3.12 ms per present.
- Pool plus environment cache, handheld: 50.0 race fps vs 45.2 for the pool alone (+12 to +14% on every measure, one valid row; the first attempt got stuck).
- Windows machines: neutral (4090 desktop 62.9 vs 63.0; laptop 29.9 vs 30.5; 5900X desktop 49.3 vs 49.8).
- Exactness: pool 2 million checks and 0 mismatches (1.29 million on the handheld alone); environment cache 70 million or more, 0 mismatches.

**Default.** Yes, both, even though the speed evidence is one row.

**A correction.** The audit that proposed the environment cache first blamed an unnamed hot ntdll bin (2.6 to 3.7 ms per frame on the present thread). A later symbol lookup showed that bin is reached from our wrapper for `WaitForSingleObject`, so it is the title's own waits, not the environment reads. Always resolve the hot address before claiming a cause.

**Lesson.** The gain looks Proton-only. Under Wine the same call costs more (name conversion, process lock, linear scan of an environment block of more than 140 variables).

### Pass N: pacing clock attribution, deadline policy and a fixed clock

**Idea.** The title paces its frames on a vblank-like event. We signal it from a pacer thread: one signal per present, plus a floor tick if no present arrives within the floor interval. Logs showed ticks per present above 1.0 (see 37.6), so extra render work ran on extra ticks.

**Switches** (written `<TITLE>_*`):

- `<TITLE>_VBLANK_ATTRIB`: counters, and (test build) a per-registration, per-tick, per-waiter record.
- `<TITLE>_VBLANK_DEADLINE`: one tick per present, absorb a present that follows a floor tick, fire the floor only after a whole interval of silence, wait on a high-resolution timer checked against QPC.
- `<TITLE>_VBLANK_FIXED_HZ`: an exact QPC grid (a simulated 600 s run held its rate within 0.1%; a real-timer smoke run gave 60.003 Hz).

**Default.** None. The deadline policy showed no gain or regressed. The floor setting became a user setting instead (see 37.6).

**Lesson.** See 37.6: a pacing change is a pacing change. Do not promote it as a performance switch.

### Pass O: smaller exactness-and-diagnostic changes

Three passes changed breach accounting or crash behaviour, not speed.

1. **Superseded-submit classification** (`XBOX_TEX_DRAW_EXACT_SUPERSEDED`, default 1). A barrier job refused because a newer capture had already landed on a span of its mip range was counted as a breach. The refusal is correct (the queue already holds bytes at least as new). So it is now counted as "superseded", not a breach. Five of six "submit" breaches on the handheld were this one shape. Every other refusal still breaches.
2. **Pinned jobs ignore `dying`** and **reach on the multi-sample plan path** (two switches, default 0). A job that holds its own pin sees `dying` only mid-flip. Both were designed with the condition that retired and GPU-owned checks stay untouched.
3. **Trap handler without static thread-local storage.** A debugger hotkey caused a crash. With a debugger attached, pressing F12 makes Windows start a break-in thread without the loader's thread attach. That thread's thread-local pointer is null. Our write-trap exception handler read a `thread_local` as its first statement, faulted, re-entered itself 48 times and killed the process. The fix replaced `thread_local` and function-local statics with a handful of `TlsAlloc` slots allocated at start. A fixture reproduces the crash with the old header and passes with the new one.

**Lesson.** Anything an exception handler can reach must not touch static thread-local storage. A function-local static with a dynamic initialiser counts: its guard reads thread-local storage on every access.

### Pass P: one shipping build for Windows and Proton

**Idea.** The Windows and Linux launch scripts had grown to 126 names across three batch files and one shell file. 99 were redundant (the shipping build already did the same; 7 were not read by the loader), 14 differed, 7 were plumbing, 5 differed by OS, adapter or CPU. 14 lines silently overrode the launcher's own settings. The Windows scripts also ran the game under a debugger, and the exe did not create its own log folder.

The pass built a profile table inside the shipping build: nine defaults the exe now supplies, each logged with its source. Variables that were absent or empty take the default; an explicit value wins. It also added three launcher settings (frame pacing, tick floor, exact fix on or off) that grey out when an environment variable overrides them.

**Effect.** No speed change by design. The debugger question was closed later: with a remote-desktop session off, running under the debugger cost nothing (92.4 vs 88.5 fps, 44.9 vs 52.8 frames over 50 ms per minute, within the run-to-run spread of 31.8 to 57.9).

**Lesson.** The per-machine scripts were themselves a source of silent behaviour differences. One binary, one profile table, and one census row per switch let any log say what ran.

### Pass Q: the hitch fixes (designed from the logs, tested only in fixtures)

These came from the hitch study (37.7). Five switches, all default 0:

| fix | what | exactness argument |
|---|---|---|
| `XBOX_TEX_RANGE_INDEX` | a size-class index of texture ranges so birth, mark and lease walks visit only states that can overlap | a state that overlaps [lo, hi) has base above lo minus its class maximum, so every possible overlap is still visited, in today's order (equal keys keep insertion order); mode 2 also runs the old walk and compares |
| `XBOX_TEX_BIRTH_PREPARE` | do per-span birth setup before taking the global texture lock | nothing can see the state until the locked block publishes it; a mark in between triggers the existing recompute |
| `XBOX_TEX_UPLOAD_BATCH_CAP`, `XBOX_TEX_COLLECT_EVERY`, `XBOX_TEX_UPLOAD_GROUP` | cap batch members, collect less often, record several jobs into one list | same copies, same order, same queue |
| `XBOX_TEX_DRAW_EXACT_READABLE_CACHE` | within one texture, remember a positive readable range | negatives are never cached |
| `XBOX_TEX_DRAW_EXACT_ARM_MERGE` | skip a page re-arm when every page is neighbour-owned and verified | needs the trap author's review |

Fixture result for the range index: over 61,166 windows, the index visited 10.9 million entries against 207 million for today's walk (19x fewer), with identical sequences; a mutant with a halved bound was caught in 18,241 of 20,304 cases. The birth matrix showed the same digest in every combination (3,031 states, 1,844 callbacks, 28 aliases), and the differential counted 4,031 walks with 0 mismatches and 3,962 entries visited against 4,377,816.

A design review of all five returned "apply" on each, with two small hardening fixes for the grouped-list path (a stale thread-local group and a null-list release in a retire sweep). No in-game numbers existed at the time of writing.

## 37.4 Findings that became future work

A later pass mined the logs of a 36-minute handheld session and the test-row logs. Nothing here shipped as a switch in this series, but the numbers are useful as a checklist for anyone with the same architecture.

1. **Base-path churn.** In an idle pre-race scene, the texture copier thread prepared 53,000 textures per minute and sent 34. In races it prepared 59,600 to 95,000 per minute and sent 6,400 to 10,600. A refused job went back through the queue and was prepared again (full snapshot plus de-tile) on the next visit. That is about 0.69 to 1.1 cores in menus and up to 0.4 to 0.5 core in races on the handheld, inside the fixed power budget. 349 to 1,083 textures were stuck at their smallest mip for 139 to 270 seconds, which may also be a visual issue.
2. **Barrier on the title thread in races.** 2.0 to 2.8 seconds per minute on the handheld (3.3 to 4.6% of race frame time), tied to the slow patches.
3. **Uncapped menus, pre-race scenes and podiums.** One thread called `Sleep(0)` 20.5 million times (110 seconds inside it); the pacing timer ticked 1,000 times per second outside races. A 30 fps menu cap saved about 1.5 cores and halved GPU load in menu rows. Races were not affected.
4. **Residency probe.** One memory-working-set query per 64 KiB chunk was slow under Proton: 113 stalls of 10 to 170 ms in a session. A setting skips it.
5. **Re-copies.** About 16.3 GB per handheld run was copied a second time (325,000 of 1.49 million copies). A skip switch exists and needs one clean audit.
6. **QPC.** `NtQueryPerformanceCounter` is a real syscall under that Proton, about 1.0 ms per frame process-wide, 0.18 on the present thread, because each timing scope reads the clock twice. Timing-only scopes can use a cheaper clock or compile out of the shipping build.
7. **First frame after a track load.** A 1.1 to 2.5 second freeze at each track start, 0 inside races.

## 37.5 What the handheld teaches

### Power: configured versus measured

- The test rows on the handheld ran at a 15 W cap. A normal frame took 17.4 ms (about 52 fps). Normal play sessions ran at 25 W: a normal frame took 13.3 ms (about 75 fps in races).
- The loader never writes power settings. Test rows must be run at the setting the player uses, and the setting must be recorded with the row.
- These passes were judged in CPU core-seconds, not watts. Measure power anyway: later, on the same handheld, the firmware limits read 35/37/45 W while the GPU's measured average power in game was 15.0 W. Configured limits are not what the device draws, so record the *measured* figure for every handheld row. The budget is shared: CPU and GPU draw from the same cap, so one extra busy core costs clock speed for everything else. The profile saw this indirectly. With the exact path on, the title code on the present thread was 0.5 to 1.6 ms slower and the GPU span 1.3 to 1.6 ms longer, with no extra work. The reading is lower clocks and cache pressure from about one more busy core at 15 W (inferred).
- At 15 W almost every frame is over 16.7 ms. At 25 W the same texture streaming is easier to see as discrete spikes. The same test run at the two caps asks two different questions.

A practical rule: at 15 W, every CPU millisecond cut on a helper thread helps even when the helper is off the critical path, because it returns power to the cap.

### Core topology: three fast cores, five compact cores

The Z2 Extreme has 3 Zen 5 cores and 5 Zen 5c cores (16 threads). A public report on a related chip put the compact cores at up to about 3.3 GHz against about 5.1 GHz for the big ones. In the handheld's logs, the cache-domain masks matched the expected fast set, a cross-check for the clock-based detection.

What we saw:

- The title's 53 `SetThreadAffinityMask` requests all came back as the full mask. Every thread floated across all eight cores.
- The presenting thread is the title's core-0 thread. It is the limiter in light race sections.
- Public reporting describes a Linux scheduler problem on Zen 5 / 5c where a task can stay on slow cores while fast ones are idle; the fix was reported as merged for a later kernel with a stable backport. Whether SteamOS has it was not verified.

**The pin** (`XBOX_FAST_CORE_PIN`): 1 = pin the presenting thread to the fast cores; 2 = also pin our uploader clock and head-start worker; 3 = observe only. Detection:

- cores from `GetLogicalProcessorInformationEx`;
- per-logical-processor max clock from `CallNtPowerInformation`, per-core clock is the maximum of its threads;
- the largest relative gap between neighbouring clocks must be at least 10% of the upper clock, and the fast set must have at least 2 logical processors, otherwise it reports "uniform" and does nothing.

The pinned thread is the one that presents 30 times in a row, found by behaviour, not by address. While pinned, the title's own affinity calls on that thread keep the fast set and the title's mask becomes the restore value.

**Result.** +0.3 fps on the handheld in one row, which is noise; a no-op on uniform CPUs (the 4090 desktop, the 3080 laptop and the 5900X desktop all report one clock per core). An earlier experiment on the same handheld had measured about 16% less hot-thread work per frame after pinning, so the pass expected +1 to +3 fps. The one clean row did not show it.

**Design rule.** The pin is a seed for a better policy: put the presenting thread first, the head start second (its stall feeds the present thread), and keep the uploader clock (12% busy, off the critical path) and the texture workers off the three fast cores. Mode 2 pins the uploader and head-start to the fast set; the profile suggested keeping them off it.

### Thread priority is not a lever under Proton

The submit-gate pass raises the uploader's priority while it holds the mutex. A later pass raises a join-worker's priority during a barrier. We later measured whether it does anything under Proton: it does not. Every Windows thread of the process ran at Linux nice 0 with `SCHED_OTHER` and rtprio 0, and the process's maximum nice priority was 0, so `SetThreadPriority` (below normal, normal or a temporary boost) had no effect on the Linux scheduler. Only Wine's own internal disk threads ran at nice 19. A test that ran the head-start worker at normal priority showed no change, as expected.

The design consequence: every priority change is paired with a counter (`boost-failed`, `restore-failed`), restores on every exit including exceptions, and is kept out of defaults until a pair shows the lock wait falling without a frame-time loss. Do not design anything that is only correct or fast if priority works. Under Proton, treat priority as a hint that can vanish.

(Related: run helper processes such as profilers at normal priority. Under a busy host, the CLI clients we used for analysis starved at the lowest priority and failed.)

### Memory: RSS, GTT and commit

On the handheld, GPU memory is system memory. The numbers from a 900-second route, measured with a 5-second sampler:

| quantity | value |
|---|---|
| loader peak resident set (RSS) | 5.94 GB |
| process GTT (system RAM used as GPU memory) | 9.36 GB |
| swap | 1.05 GB |
| lowest free memory | 13.5 GB |
| total | about 16 GB |

- The 5.7 GB of "title shadows" was our own native GPU resources created for the title's placed resources (about 3.65 GB for textures and about 2.0 GB for buffers). It is not a CPU duplicate. Change detection already uses hashes, so hashing saves nothing.
- A 512 MiB cap on snapshots saved nothing (GTT 9.585 to 9.725 GB; RSS 6.084 to 6.117 GB). It was rejected.
- The requested heap split: about 6.0 GB upload heap, about 4.1 GB default heap.
- The target (under 12 GB with the road lit) was not met. The next lever, importing title memory as a heap, can reach only the buffer part (about 2 GB). It also needs a rule: withdraw the heap after a fence before the title frees or decommits that memory, and keep a no-free-under-live-heap assertion.

On a Windows desktop with a small commit limit the numbers are different but the lesson is the same. The 5900X desktop has 31.9 GB of RAM plus a 2 GB pagefile, a commit limit of 33.9 GB. Runs that peak at 27 GB or more of private memory hit it: allocations and list closes then fail, and a single present raises 200 to 750 breaches. Such rows are marked environment-invalid, not switch regressions. Forcing upload heaps into host memory (`VKD3D_CONFIG=force_host_cached`) ran that machine out of commit during a load spike (out-of-memory, 2,352 breaches) and was harmful there.

Keep idle pools bounded (the snapshot pool holds at most 8 blocks per class and 32 MiB), and make "memory" a first-class column in every test row on a shared-memory device.

### Profiling under Wine

- The sampler manages about 3.4 samples per second per thread (about 240 µs per sample), and overhead was 6.5% of a core. Use a thread filter, not a whole-process sample, for about 8x the samples per thread.
- Wine's syscall thunks start at a fixed offset and are ordered by Windows syscall number. Three wait anchors fit exactly, so waits can be named. IDs above 0x62 are not Windows-stable; name them from the thread's role.
- Wine's own ntdll cannot be symbolised without the matching file. Copy Proton's `ntdll.dll` and `bcrypt.dll` from the device to the analysis PC.
- A hot 16-byte ntdll bin with a constant share of the frame, uncorrelated with fps, is the signature of a spin loop. Here it turned out to be reached from our own wait wrapper (see pass M).
- Public source reading found that `Sleep(0)` under Wine costs three syscalls per call (two `getrusage` calls and `sched_yield`). A render thread that spins on `Sleep(0)` burns CPU in syscalls. The cheap fix is on our side: replace the spin with a blocking wait.

## 37.6 Frame pacing versus fps

### What "smooth" meant in the measurements

A median fps number hid the problem. The hitch study judged each arm on:

- frames over 50 ms per minute in the race;
- frames over 100 ms in the race;
- race worst frame;
- 1% low;
- the share of frames over the display's frame time (16.7 ms at 60 Hz).

The table below is from the RTX 4090 desktop, 360-second runs, a fixed save, no one at the PC and no remote session:

| arm | avg fps | race fps | race frames over 50 ms per min | over 100 ms | race worst |
|---|---|---|---|---|---|
| exact path off | 86.3 | 87.7 | 9.2 | 1 | 209 ms |
| exact path on, trap detection | 83.6 | 87.8 | 50.8 | 9 | 163 ms |
| on, with head-start refresh | 92.9 | 101.0 | 7.2 | 2 | 106 ms |

The median barely moves between the first two rows. The in-race hitch rate goes up 5.5x. That is why "fps unchanged" was not good enough.

On the 25 W handheld, during real driving, 550 frames per minute went over 16.7 ms (12% of frames), 165 over 25 ms, 32 over 33 ms, and only 2 over 50 ms. Most of the time over 16.7 ms came with texture streaming: in the quarter of the race that streams the most, the game ran at 68 fps with 821 frames per minute over 16.7 ms; in the quietest quarter it ran at 84 fps with 181. The exact path runs on the same thread that draws, already about 85% busy.

### Floors, ticks and the title's clock

The title does its per-tick work on the clock we drive.

- Every tick above 1.00 per present is a floor tick inside a frame that its present also ticked (a double count). On the handheld at a 60 Hz floor, 2.12 to 2.47 ticks per present; at a 30 Hz floor, 1.17 to 1.27, so 17 to 27% of frames got a second tick. On the 4090 desktop, 1.04 to 1.20.
- A 30 Hz floor waited by `WaitForSingleObject(…, 34 ms)` is 29.4 Hz, not 30. On a Windows machine at the default timer resolution it measured 21.5 Hz. Wine waits are precise. The deadline pass uses a high-resolution timer checked against QPC.
- The race clock and audio do not follow the tick count. Across 1.0 to 2.5 ticks per present the race clock ran at 0.995 to 1.004 of wall time, and a pooled fit gave 0.99979 with an error of 0.00047. Game time only slows when frames are very long (a run at 7 to 23 fps read 0.68 to 0.83).
- What the tick does drive is render work: about 120 to 160 extra draws per tick on the laptop, and about 2.1 to 2.4 ms per tick on the handheld between a 60 and a 30 Hz floor. Nobody named the consumer that issues it.
- The title does not poll a waitable timer, timer queue or multimedia timer, and the frame-statistics call returns zeroed records, so the title gets no vblank counts from us.

**What shipped.** Frame pacing became a launcher setting (normal, or one tick per frame), with a tick floor choice (20, 30, 40 or 60 Hz), default normal. It is not a promoted performance switch. At 15 W the handheld ran about 30 fps with the floor at 30; it targets a steady cap, not more frames.

**Two rules.**

- Never trade pacing for fps in a "performance" switch. A lower floor does fewer updates, which does not prove they were unnecessary.
- Menus are a pacing problem too. An uncapped menu burns a core per hot thread. Cap menus (30 or 60) and leave races alone.

## 37.7 The end-of-load and pre-race hitches (what the hitch study found)

The study ranked causes by what the player feels:

1. **In-race barrier staging on the title thread** (the head start does not cover the textures). 11.2 s in 411 barriers of 16 to 50 ms and 4.3 s in 66 barriers of 50 to 250 ms. Of the 50 to 250 ms barriers, 75% was inline staging and 9% submit. A site profile of the 16 to 50 ms barriers put snapshot read at 31%, XG convert at 24%, hash at 8%, upload allocation at 5% and page re-arm at 4%. Only 9% of needed spans had a ready head-start job (3,003 of 32,073). 124,322 of 136,297 needs had a head-start entry with no job for that span, and 106,129 of those had been dirtied at least 64 ms before the barrier needed them. That is plenty of time to stage off-thread. Cure: head-start refresh (pass K).
2. **A launch under a debugger.** Closed: it costs nothing once the remote session is off.
3. **The end-of-load frame of about 1 second.** One barrier needed about 23,000 to 35,000 spans at once (34,933 jobs, 5,824 textures, 26.8 MB in total, the largest texture 6,144 bytes). About 93% of the 963 ms was fixed cost per job or span, not byte copying: per-job submission chain 59% (about 570 ms: 109 ms creating command objects, 101 ms recording, 97 ms collecting, 76 ms submitting, and so on), `readable` walks 15% (43,321 `VirtualQuery` walks), per-texture bookkeeping 16%, byte work 7%. Head-start refresh does not change this frame because the jobs were already prepared; submitting them is the cost. Cure: capped groups and fewer collects (pass Q).
4. **The pre-race menu at 4 to 7 fps while the track loads.** The global texture lock was held across texture birth, whose alias walk explodes during the load: 18,971 births in the 25-second load walked 2,304 states per birth, each holding the lock for 143 µs, a total hold of 2.7 s; in the race a birth walks about 21 states and holds for 10 µs. Presenting-thread lock wait was 8.9 s with the exact path on vs 0.14 s off. The release hook also took the same lock on every release of a tracked texture, 13.5 million times per run. Cure: the range index and birth setup outside the lock (pass Q).
5. **Shader compiles.** The shipping profile set pipeline recording off, so the pre-build never warmed. A race start compiled 6,466 pipelines with 19.9 s of driver time on a cold driver cache; warm, the same pipelines took under 1 s. On the handheld (Proton) it is likely worse.
6. **Write-watch detection on Windows is not an option.** Harvesting the watch each barrier cost about 210 s of a 360 s run (22 fps). Trap detection plus the crash fix is the path.

## 37.8 How a switch earned default-on

### The checklist

A switch became the default in the shipping build only when all of the following held:

1. **An exactness argument written down.** What is the same before and after? Which decision reads each input? For the hash: SHA-256 is a fixed function. For the pool: a reused block equals a fresh reservation before any read. For the range index: every state that can overlap is still visited.
2. **A fixture with a failing-first control.** For each fixture, a deliberately wrong version (a mutant) must fail. A pool without zeroing was caught in 325 of 400 jobs. A halved index bound was caught in 18,241 of 20,304 cases. A corrupted hash constant raised a breach. Without the control, a passing fixture proves nothing.
3. **A differential mode in the test build.** Mode 2 runs the old and new code side by side on every call, keeps the old result, counts mismatches, and raises a breach on any difference. A switch at 2 is exact whatever the new code does. Where the differential is cheap and harmless it is also accepted in the shipping build (the range index), so a differential row can run on a machine without a test build.
4. **A full-run audit on a real machine.** For the stack as a whole, the final audit on the handheld read:

   | counter | value |
   |---|---|
   | checks | 1,768,696 |
   | changed without a mark | 0 |
   | read faults | 0 |
   | hash mismatch | 0 of 4.57 million, plus 0 of 2.52 million lean |
   | snapshot pool | 1,291,668 checks, 0 mismatch |
   | environment cache | 22.5 million checks, 0 mismatch |
   | breaches | known families only (prepare 4, submit 2, snapshot-hash 1, bundle 2) |
   | laps | 1 to 3 seen, no device loss |

5. **A census row.** Every switch has a "switch effective" row at start that prints its effective value, its source (environment, profile default, invalid value fallback) and its profile default. A verifier reads the row. A receipt with the old default is refused by a negative test.
6. **A speed measurement, where available.** Not a blocker for exact, harmless switches.

### The rule for defaults

- Applies only to the shipping build of one title. Test builds keep 0.
- An absent or empty variable takes the profile default. An explicit value, including 0, is parsed exactly as before. (One switch, `HS_EAGER`, treats empty as present because its census row does.)
- One source of truth: a header generated from a selection file. The census verifier mirrors it, and its own test refuses to run if the two differ.
- Test fixtures set an explicit 0 for every promoted switch, so they keep testing the baseline. This was needed: eleven fixtures wrote descriptor slot maps directly or assumed the old defaults, and three failed without it.
- Kit baselines that need the old behaviour must say "0" explicitly from that point on.

### What was promoted

Ten switches, together:

| switch | default | evidence |
|---|---|---|
| descriptor tracking fast + dense | 1 + 1 | handheld +23%, 4090 desktop +13%; audits 0 breaches and 0 of 116 million and 132 million |
| hash fast | 5 | handheld +9%; Windows neutral; audits 0 mismatch |
| snapshot lease | 1 | audits 0 breaches; speed neutral |
| submit hold | 3 | inside a measured handheld stack (+1.8, n=1); 4090 desktop audit clean |
| stats free | 1 | inside the same stack; no decision reads the statistic |
| fast-core pin | 1 | noise on the handheld; no-op on uniform CPUs |
| head-start eager | 1 | handheld +5%, laptop +2.0; **weakest evidence**: one 4090 desktop row lost the device at 258 s |
| snapshot pool | 1 | exact on every node (2 million checks); neutral fps; handheld stall 2.56 vs 2.83 ms |
| environment cache | 1 | exact on every node (70 million checks); handheld +12 to +14% in one row; Windows neutral |

Plus the superseded-submit classification (default 1 in both builds, because it only changes how a correct refusal is counted).

### What was not promoted, and why

- **Upload batch:** hangs on AMD. Still unproven cause.
- **Head-start refresh:** no median-fps gain on the handheld at the time; strong hitch evidence arrived afterwards (one run, then a repeat with a smaller effect).
- **Pacing deadline:** no gain, or a regression.
- **Tick floor:** changes pacing. A user setting, not a default.
- **Bundle memo, find-GPU cache, layout cache, SSE2 scan, upload pool:** no measured gain, or too small to justify their invalidation rules.
- **The hitch fixes:** designed and fixture-tested only.

### Measurement discipline for valid timing

- **Alternate A and B** across short chains (the kit refuses duplicate kinds in one chain).
- **Know your noise.** An A/A pair on the same machine differed by 0.5 to 1.5 fps. A single-row gain of 0.2 to 0.3 fps is noise.
- **Valid-run rules:**
  - lap-1 time over 300 s means stuck, discard;
  - a device loss before the end means discard;
  - peak private memory at or above 27 GB on the 5900X desktop means environment-invalid;
  - no other heavy job on the machine in the same window;
  - no remote-desktop session during timing.
- **A promotion table.** One row per switch, one column per machine, each cell: audit / losses or hangs / fps against base, with n for each arm. Cells say "never run", "stuck" or "unreadable" instead of staying blank. The table's "gaps" list was the work list: every handheld arm had n of at most 3; there was no clean standalone A/B for stats-free or submit hold anywhere; the laptop had no rows for several switches.
- **Estimates are marked.** Labels in analysis notes: measured from a log, estimated, unknown. Several estimates were off by a factor of three (descriptor tracking gained more than predicted; the batch pass predicted a large win and hung). Plan gates on measurement.
- **A second reviewer.** Each non-trivial pass had a design review before it was applied, and a read of the code afterwards. The reviews found real errors, such as a wrong claim that counters were unchanged, a possible stale thread-local group, and a bin attributed to the wrong function.

## 37.9 Short lessons for a Series X translator

1. **Measure on the weak machine first.** The largest single cost (SHA-256 in portable code under Wine) did not exist on the desktops, and the best gain (descriptor tracking) was more than three times its prediction on the handheld.
2. **Per-call work is the enemy.** Environment reads, virtual-memory calls, logging and clock reads that cost nanoseconds on Windows cost microseconds under Wine, and they run thousands of times per frame.
3. **A lock-hold counter must say what it times.** Include waits in a "hold" and you chase the wrong lock.
4. **Batching has a hang risk on AMD under Proton.** Cap it and test on that driver first.
5. **Exact work needs a differential mode** and a failing-first control.
6. **Cheap counters first.** A hit-rate counter killed a whole design.
7. **Do not rely on thread priority under Proton.** Affinity does work: pinning the hot thread to the fast cores of a hybrid CPU helped on the handheld.
8. **Judge smoothness by long frames, not the median,** and report the power cap and the number of valid runs.
9. **Hold the environment still.** A remote-desktop session cost about 2 cores and about 30 fps in one arm and briefly produced a false "the debugger costs a third of the frame rate" conclusion. Other builds on the same PC, a full disk and a stuck lease cost more runs than any switch bug. Check that an environment token exists in the DLL before A/B testing it: one vkd3d token we tested did not exist, so the arms were A/A.
10. **Promote on proof of exactness, not on proof of speed.** Exact, harmless switches can ship with a weak speed result; say so in the table.

---
[Back to the index](../README.md)
