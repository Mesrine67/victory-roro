# 4. Detecting CPU writes to GPU-visible data

## 4.1 The problem
To copy changed texture bytes before the GPU needs them, we must know *which* bytes changed. The title writes them with plain stores, sometimes across many threads, often in bursts during level loads. We found four writer classes:

1. **CPU stores** by title threads. This is the big one. In our game, large mips reach title memory by CPU stores; no completed file read landed in any texture we traced at a scene cut.
2. **File reads** that land directly in title memory (`ReadFile`, I/O completion ports). These are *kernel* writes. They do not fault on a read-only page; they fail with `ERROR_NOACCESS`. They also do not reliably set write-watch bits.
3. **DMA and copy-engine work** the title schedules. We emulate it, so we see it.
4. **GPU writes** (render targets, UAVs). These never touch title memory in our design, because the GPU writes the desktop resource.

## 4.2 Options we tried

| Approach | How it works | Why it lost or won |
|---|---|---|
| Periodic re-check (polling) | Revisit each waiting texture in a slice of time per present and compare. | The early design. 800-1,700 waiting textures shared a 2 ms slice, so each was revisited every 0.16-0.31 s on the handheld. That matched the black pops users saw. Also, each visit paid `VirtualQuery` costs. |
| Write-watch (`MEM_WRITE_WATCH` + `GetWriteWatch(RESET)`) | Every title reservation is made write-watched; harvest dirty pages. | Correct, but cost scaled with tracked memory. One 600 s run made **76 million** `GetWriteWatch` calls (about 1,370 per submission, 133 s in total). On the laptop: 41,000-56,000 calls and about 63 ms of harvest per present. Unplayable on the handheld. |
| Hash every reachable texture at submit | Hash, compare, upload changed bytes. | Correct and simple, and far too slow for full scans. We kept hashing to decide "equal, no upload needed" on already-captured spans. |
| **Page-protection trap** | Arm a texture's pages `PAGE_READONLY`; the first title write faults; record "dirty", restore the page and let the write retry. | Cost scales with *changes*, not with memory size. Our default, with write-watch kept as an independent checker in test builds. |

## 4.3 Trap design

**Data.**
- Each tracked texture has a state with a list of **spans**. A span is one mip, or one run of packed small mips.
- Each span owns a **token** holding:
  - its page-aligned range and exact content range;
  - the resource and generation it belongs to;
  - an atomic **phase**: `Dirty`, `Armed` or `Handling`;
  - a sequence number, the arm epoch, the last fault sequence, and a small per-span lock.
- The registry has two parts:
  - an interval map of spans;
  - a per-page table: how many armed tokens own the page, the page's original protection, its allocation, and which token will restore it.
- A per-page epoch survives until the allocation is freed, so a stale fault can be told apart from a new one.

**Arming.** Under the exclusive registry lock, a span may be armed only if every page is:
- committed and private;
- inside one allocation;
- plain read-write (optionally write-combined);
- not inside an active *exclusion* (section 4.6).

Otherwise the span is refused with a reason (unmanaged, busy, query failed, protection mismatch, mixed, neighbour conflict, protect failed). It falls back to write-watch only. Arming bumps the per-page counts, calls `VirtualProtect(PAGE_READONLY)`, and stamps the token with an arm epoch from a global sequence. That stamp lets us later order a capture against a fault.

**Arm merge.** Small textures put five or six mips in one page. Re-arming each one separately cost **132 ms for 24,453 re-arms** at the end of one track load. With merging, we skip the `VirtualProtect` when every page is already armed by a live neighbour.

## 4.4 The fault handler
A vectored exception handler. It acts only on write access violations inside a registered span. Steps:

1. Take the registry lock **shared**.
2. For each overlapping span, take that span's lock.
3. If the span is `Armed`, compare-and-swap it to `Handling`. Drop its page counts. The thread that takes a page's count from 1 to 0 restores that page's original protection, in batched runs, after checking with `VirtualQuery` that the page is still ours.
4. Push a pre-allocated event node onto a lock-free pending list, and the token onto a cleanup list. Set the phase to `Dirty`.
5. Signal the worker's semaphore.
6. Return `EXCEPTION_CONTINUE_EXECUTION`. The title's store retries and succeeds.

**Invariants. Treat the handler like an interrupt handler.**
- **No allocation, no logging, no container erase, no `shared_ptr` destruction, no external callback.** The fault can arrive on any thread, holding any lock, including the CRT heap lock.
- **Fixed lock order:** registry (shared), then one span, then atomic list publication. Nothing else.
- **No static thread-local storage.** We learned this from a crash under a debugger. The handler also runs on threads created without the loader's thread-attach step: debugger break-in threads, and threads created with "skip thread attach". On those, the TLS pointer is NULL. Any static TLS access faults inside the handler, which recurses until the stack is gone. A function-local `static` with a compiler-generated guard counts, because the guard uses TLS. Per-thread retry state now lives in `TlsAlloc` slots allocated at process start, with indices below 64, so `TlsSetValue` never allocates.
- **Stale and duplicate faults.** If no armed span matches, the handler checks under the same shared hold whether the page is now writable. That closes a read-write → read-only race against a concurrent re-arm. If it is writable, the handler retries the instruction, at most three times per address and epoch, then gives up and counts the event.

## 4.5 Drain and the event ring
One exclusive consumer **drains** the pending list. It turns events into "this span is dirty" *marks*, removes dead entries, and refreshes trust.

**Failure.** Our first event ring had a fixed size. It overflowed **127,391 times** in one run, and each overflow forced a re-check of *every* texture. The exact path then ran at 1-5 fps. Because the title's own streaming is clocked by frames (section 6.5), the world on a long straight vanished and came back about 10 seconds later.

The fix made draining O(changes), with no overflow, and batched the harvest. On the laptop, per present:

| | Before | After |
|---|---|---|
| Barrier stall | 203 ms | 7.0 ms |
| Drain | 182 ms | 0.3 ms |
| Harvest | 65 ms | 0.3 ms |
| Frame rate | about 2 fps | about 16 fps |

**Lesson.** A bounded queue whose overflow means "assume everything changed" is a cliff, not a degradation. Size it from measured peaks, make the overflow path cheap, and count every overflow.

## 4.6 Disarm on purpose: exclusions and mapping scopes
Our own `PAGE_READONLY` must never leak into title-visible behaviour:

- **Mapping scopes.** Every VM hook, ESRAM map, `VirtualQuery`, and file read begin/end opens an *exclusion* over its range. Opening one disarms every overlapping span in one exclusive hold and holds an exclusion count. Arming refuses any range under a live exclusion, so no trap can be armed during a mapping change.
- **Kernel writers.** If the title uses a kernel-side write path we do not cover (raw file reads, device I/O control, socket receive, file mapping views), we switch trapping off globally. A kernel write to an armed page fails instead of faulting, and the title would see an I/O error. A blocked kernel write is counted as a correctness breach.
- **File reads into tracked memory** go through an **I/O ledger**: a bounded open-addressing table of in-flight reads and their completions. When a read is issued, its range is excluded and marked "read issued". When it completes, the range is marked "read complete". The ledger is explicit because kernel writes neither fault nor reliably set write-watch bits.

## 4.7 Retire and "retire own"
When the title releases a texture, we **retire** its state and disarm its tokens. Originally, retire disarmed the whole page range, including *other textures' tokens* on shared pages. Two problems followed:

- A neighbour's token, disarmed by someone else's retire, looks dirty. The next barrier then re-checks and re-hashes it, finds it equal, and re-arms it. That is about **130,000 needless "equal after disarm" re-checks** per run.
- Those re-checks were what made the head-start worker look idle while the barrier staged inline. They misled one of our own design reviews (section 9.5).

The **retire-own** mode disarms only the retiring state's own tokens. Measured on the AMD desktop:

| | Before | After |
|---|---|---|
| "Equal after disarm" re-checks per run | about 130,000 | under 100 |
| Mark-staging time per slow frame | 1.9 ms | 0.08 ms |
| Stolen needs | baseline | about halved |
| Slow frames | baseline | unchanged, within noise |

The correctness run showed 0 "changed without mark" events over 3.1 million audit checks, 738,832 own-token disarms and 0 foreign-range tokens.

It is a clean, exact improvement. It did not fix hitches by itself. It is still worth having, because it removes false work that hides the real bottleneck.

## 4.8 Trust, the oracle and self-tests

**When a span is trusted.** A span is *trusted* (its last GPU copy still matches title memory) when all of these hold:
- it has a landed copy newer than its last mark;
- it is not being read by I/O;
- the capture is provably ordered after the last fault epoch.

**The oracle.** In test builds, the write-watch harvest still runs as a *second, independent detector*. When it reports a dirty page under a trusted span, we hash that span in bounded slices: SHA-256 in 64 KiB pieces, with an 8 MiB budget. A mismatch is a **trap miss**, recorded as a breach. A separate audit mode re-hashes trusted spans at random. A typical clean audit row: 1-3 million checks, 0 "changed without mark".

**Breaches never drop the title's work.** After a breach the run is flagged invalid, but the title's submission is always preserved.

**Self-tests at startup** check that the platform behaves as the design assumes: faults are delivered, write-watch works, and `ReadFile` into a protected page fails rather than faults. Under Wine, check each of these on every Wine/Proton version you support.

**Counters in a healthy run.** In one audited run:
- 39,860 traps and 1.71 million arms, matched by 1.71 million disarms;
- 14,715 refused arms;
- zero trap misses, armed-but-not-read-only, trusted-but-unarmed, blocked kernel writes and failed protects.

Before retire-own, arms were 2.2 million.

## 4.9 Lessons for write detection
- Build two independent detectors, and keep the second one in test builds. The disagreements are your bug reports.
- Every arm, disarm, fault, refusal and overflow gets a counter, and every run prints them. "Trap health all zero" became a required check before reading any performance number.
- Kernel writers, file reads and anything not on a CPU store path need their own ledger.
- Treat the trap layer's own `VirtualProtect` and `VirtualQuery` calls as hot-path costs. Under Wine they are.

---
[Back to the index](../README.md)
