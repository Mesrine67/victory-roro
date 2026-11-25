# 5. Exact texture staging

## 5.1 The contract: submission equivalence
After many attempts (section 6), we wrote down one rule and made everything else serve it:

> **Before the title's command lists in an `ExecuteCommandLists` call reach the GPU, every committed span of every CPU-produced texture those lists can reach has a GPU copy made from bytes that were not marked dirty after they were read.**

What the rule rules out:
- **Placeholders, neutral fills or "last good" content.** A console has no such state. The faithful value of an unwritten tile is what the console would show, not something we make up.
- **Lowering the mip, distance or quality** so that something "safe" draws instead.
- **Skipping or deferring a copy** to a later frame.
- **Waiting on a GPU fence** to "let things settle", or **dropping a submission**.

What the rule allows:
- **One-submission lateness for genuine races.** If the title writes a span *while* our barrier is reading it, the next barrier sees the mark and copies again. On the console the GPU and CPU would race in the same way. A racing title write becomes visible one submission later, and we count these events.
- **Doing the work early** (section 5.5), as long as the barrier can prove the early copy is still current.

The copy runs on the **same direct queue, ahead of the title's lists in the same submission**, so no cross-queue synchronisation is needed. The only thing the title can observe is the order of submissions, and that order is preserved.

An earlier experimental path tried to publish content when a *descriptor* was created ("first use"). It is off everywhere. The lesson we wrote into it: **creating a descriptor does not publish content.** Only something that sees *every* writer may treat bytes as final.

## 5.2 The barrier, step by step
The barrier runs inside our `ExecuteCommandLists` hook, under one *submission boundary* lock, before the native submit:

```
q_ExecuteCommandLists(lists)
 └─ boundary lock
     1. REACH      which textures can these lists sample?          (section 7.2)
     2. HARVEST    collect write-watch bits (if any)
        DRAIN      turn trap events into dirty marks
     3. CLASSIFY   per reached texture, per span:
                     trusted?  readable?  head-start ticket present and fresh?
     4. STAGE      per span that needs bytes:
                     fresh head-start ticket ─► use it
                     queued but not started  ─► take it back ("stolen"), stage inline
                     running                 ─► wait for it (or claim and continue, optional)
                     none / stale            ─► stage inline
     5. SUBMIT     record barriers → COPY_DEST, CopyTextureRegion × N, barriers back,
                   execute on the direct queue, signal the upload fence, mark spans "landed"
     6. POST-RACE  any mark that landed after its snapshot was read → span stays untrusted
                   for the NEXT barrier (counted, never retried inside this one)
 └─ native ExecuteCommandLists(title lists)
```

Lists bound for other queues get the reach walk for bookkeeping but no staging. All texture copies go on the direct queue in our current design.

## 5.3 Snapshot, hash, detile
For each span that needs bytes:

1. **Snapshot.**
   - Arm the trap for the span first. Arming before reading is what makes a racing write detectable.
   - Copy the span's title bytes into a private snapshot region, using a guarded read. The memory can be decommitted under us, and the guard turns that into "this capture failed", not a crash.
   - Hash the snapshot (SHA-256).
   - Record a capture: span, sequence, hash, whether it was all zero, and a mapping signature (section 5.4).
2. **Equal accept.** If the hash equals the last landed capture, the span becomes trusted again with no upload. After the retire-own fix, this path stopped being a large share of the work.
3. **Detile.** The console tiling library converts the tiled bytes to a linear subresource in an UPLOAD buffer.
4. **Record the copy** into the barrier's upload list.

Costs we hit, and what fixed them:
- **Hashing under Wine.** Wine's crypto provider did not use the CPU's SHA instructions. Hashing cost about **300 ms of CPU per second**, and the presenting thread waited for it. We wrote our own SHA-256 with SHA-NI (scalar where the CPU lacks it). A startup self-test checks that its digests are bit-identical to the system provider's, or we keep the system one. A later mode hashes the snapshot in place and folds the all-zero test into the same loop.
- **Detiling.** The console tiling library cost about 65 µs per 21 KiB call on the handheld: 3.2 ms per MiB, 55-60 seconds per race across all threads.
  - For the one layout class this game uses heavily, the detile is a pure byte permutation made of short runs.
  - A *learned run table*, built on a lowest-priority thread, does the same copy about 10x faster.
  - The first four uses of each layout class are compared byte for byte with the library's output. One mismatch disables the class for good.
  - It is off by default until it has run in more titles.
- **Scratch allocation.** Snapshot regions were reserved and freed per job. Under Wine each `VirtualAlloc`/`VirtualFree` takes the process-wide VM lock: about 2.5 ms of CPU per frame on the worker threads. A snapshot pool now reuses reservations with their committed pages. For exactness, a reused block is made indistinguishable from a fresh one before any job reads it:
  - pages outside the needed range are decommitted;
  - reused pages are zeroed except the bytes the caller will fill;
  - a test mode verifies all of this.
- **Upload batching.** A barrier pass's copies are gathered into one command list, one `ExecuteCommandLists` and one `Signal`, capped at 64 jobs per batch. A conflict (same resource twice, or an ineligible job) flushes early.
  - An earlier batching variant built batches of **4,470 members** on the 5900X desktop and **53,405** on the handheld in one submission. Every 5900X run hung about 30 s in, and one handheld run lost the device at 158 s.
  - **Cap batch sizes.**
- **Upload buffer churn.** An upload-block pool and a layout-footprint cache exist. They are off by default because they did not move frame time.

## 5.4 Leases: keeping the memory mapped while you read it
**Failure.** In every run with exactness on, we saw "snapshot read" breaches: a guarded copy faulted while the span looked readable. Root cause:

1. the barrier checked a span, found it readable, and armed it;
2. meanwhile the title, on another thread, decommitted the same pages with `VirtualFree(MEM_DECOMMIT)`;
3. our resource pin kept the *desktop resource* alive, but nothing kept the *title memory* mapped.

One test run had 2,502 such rows. 2,492 were on the speculative head-start worker, which is harmless because the barrier re-stages. Ten were on the barrier thread, which is not harmless.

**Fix: a striped snapshot lease.**
- Title VA is cut into 64 KB regions, mapped onto 1,024 stripes. Each stripe has a reader-writer lock and an epoch.
- Readers (snapshots) take stripes **shared**.
- Mutators take stripes **exclusive**, always in ascending stripe order, so no deadlock is possible. Mutators are free, decommit, protect-to-inaccessible and `MEM_RESET`.
- Each reservation has an identity record `{base, end, id, epoch}`, and every mutation bumps the epochs.
- A capture stores a signature (a hash of identity records and region epochs) taken under its shared lease. Whoever accepts the capture recomputes it. If it changed, the mapping moved under the copy, and the capture is refused.

**Invariant.** A capture is valid only if the memory it read was continuously the same mapping from before the read until after it.

## 5.5 The head-start worker
Staging inline on the submitting thread costs frame time. The head-start (HS) worker prepares captures *early*, from the moment a texture is marked dirty or first viewed. The barrier then usually just picks up a finished **ticket**.

**When the barrier asks for a span, it finds one of these:**

| Outcome | Meaning | Barrier action |
|---|---|---|
| Used | A finished ticket whose capture is still current. | Accept, record the copy. |
| Stale | A ticket exists, but the span was marked after the capture. | Drop it, stage fresh inline once. |
| Stolen | The job is queued but not started. | Remove it from the queue, stage inline. |
| Running | The worker is in the middle of it. | Wait (about 0.37 ms per wait; 22,000 waits and about 8 s per race on the 4090), or claim it and keep going (optional). |
| Missing | Nobody ever requested it. | Stage inline. |

**Refresh.** In the first design, a repeat request for an entry that already existed was dropped. A job that ran before the title had finished writing therefore always ended in an inline stage. With *refresh*, a request advances a per-entry "wanted" revision in any state. A finished pass publishes immutable tickets, and a new pass is queued if "wanted" moved. A refresh keeps the still-valid tickets and re-stages only the others. The chain is capped, so a texture the title rewrites every frame cannot monopolise the worker. This is on by default.

**Eager mode** stages as soon as a mark arrives, rather than when the texture is next viewed.

**What we tried for the "stolen" problem, in order:**

1. **A cheaper exclusion lookup** (an exact page-aligned key replacing a window walk). About 700,000 walks skipped per run with 0 misses; no change to frame time.
2. **Hot/cold queues** (viewed textures first). 37,000-50,000 hot-stolen needs per run remained; no change.
3. **Both together.** No change.
4. **Preview** (stage readable textures before their first view). Worse in one race (112 against 89 slow frames per minute), slightly better in the other. Rejected.
5. **Normal thread priority for the worker.** No effect under Proton, because every Windows thread there runs at the same Linux nice value (section 9.8).
6. **A second worker.** Stolen needs fell **85 %** (36.5 to 5.5 per slow frame), but real slow frames were unchanged on the AMD desktop and *worse* on the handheld at 15 W: zero-need slow frames went from 11.6 to 28 per minute, which looks like CPU contention.
7. **A second worker plus eager staging.** The first measured gain (section 9.6).

**Why ordering fixes did nothing.** The *lead time* was already long. A histogram of how long before the need each stolen job was queued showed 33,913 of about 37,500 queued **16 ms or more** ahead. The jobs were not late. One worker simply could not get through them. Throughput was the limit, not order.

## 5.6 Fan-out
When the barrier still has a big inline batch, an optional fan-out splits it across up to eight helper threads plus the submitting thread, by texture, largest first. It starts above 16 spans or 2 MiB. One texture's spans stay on one thread, because its detiling helper is not thread-safe. On hybrid CPUs the helpers avoid the presenting thread's fast cores. This mattered most for level loads.

The original level-load frame took **12 s**. The barrier copied 84,058 spans (65 MB) one by one: about 5.1 s staging, 4.0 s preparing and 2.7 s submitting. The time was per-span overhead, not bytes. Batching per texture, pooled upload memory, batched submission and fan-out brought the level-load stall down to a few hundred milliseconds. In one checked run, the race-load window was 435 ms and its largest present 216 ms, where it had been 11.7 s in a single present.

## 5.7 Why exactness, and what it costs
**The "speed is correctness" proof.** We ran the game with exactness *off*, deliberately paced to about 2.5 fps. The game itself dropped trees, an overpass and other scenery, because its own streaming runs on frame-clocked logic. So a correct but slow implementation is not correct to the player: the title's world depends on frame rate. Any design that makes frames slow enough breaks the game in a different way.

**Measured cost of the exact path once mature** (fix on against fix off, same build):

| Machine | Race start | In race |
|---|---|---|
| RTX 4090 desktop | about -19 % fps (52.3 vs 64.5) | about -13 % (62.4 vs 72.0) |
| RTX 3080 laptop | about -45 % (20.8 vs 36.9) | about -30 % (28.9 vs 41.4) |
| Handheld 15 W | | 22.6-23.5 vs 28.1-32.9 fps |

**It adds almost no GPU work.** Traced on the 4090: 3.1 vs 3.3 ms of GPU work per frame, with the same 24 command buffers per frame. On the handheld: 22.6 vs 22.5 ms. The whole cost is CPU on the path to the GPU. In GPU terms it shows up as a **9-26 ms idle gap before the title's first big render list** (section 12).

After the exact path was correct, a viewer on the 4090 reported that, apart from performance, the game finally looked right.

---
[Back to the index](../README.md)
