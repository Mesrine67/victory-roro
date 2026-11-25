# 2. Owning the title's address space

## 2.1 Reserve before anyone else allocates
**What.** At the very start of the loader's `main`, before the D3D12 device or any driver DLL loads, we reserve the title's fixed ranges as `PAGE_NOACCESS` placeholders. When the title later asks for an address inside one, the hook frees the matching part of the placeholder and lets the real allocation through.

**Why.** If you create the device first, the driver sometimes puts its own allocations inside the title's ranges. The title then gets a failed reservation, and in our case a silent zero-size pool. That gave a crash minutes later, far from the cause.

**The remaining hole.** On native Windows, ASLR sometimes places the main thread's stack, the PEB or the first TEB inside a title range *before our code runs*. We measured this at about **1.85 % of launches**. No placeholder can win that race in-process. The fix is a relaunch:

- the loader detects the lost hold;
- it starts a fresh copy of itself, up to four times;
- the child exits with a dedicated retry code if it also loses;
- a kill-on-close job object keeps the process ID and exit code consistent for whatever launched us.

We never saw this under Proton, but the guard is on everywhere.

**Lesson.** A failure that shows up "1 in 50 launches, minutes in, as a crash in title code" was really a startup address-space race. Log every reservation result at startup with its address. It turns a mystery crash into a one-line diagnosis.

## 2.2 Hook every way memory changes
The title imports its VM API from the console's kernel library. We bind those imports to our own functions:

| Title API | What our hook does |
|---|---|
| `VirtualAlloc` | Strips console-only flags; maps console page protections to desktop ones (section 2.4); commits through the write-detection layer so new pages start tracked; records the range in the title-memory ledger; bumps a commit counter that tells waiting textures "memory you were waiting for may now exist". |
| `VirtualFree` | Resolves `size == 0` frees to the real allocation range (section 2.3); marks the range as freed or decommitted *before and after* the real call; updates the ledger. |
| `VirtualProtect` | Applies the change through the trap layer, so our own read-only arming is preserved or released correctly; updates the ledger. |
| `VirtualQuery` | Answers through the trap layer. A range we have armed read-only is first disarmed, so the title never sees our `PAGE_READONLY`. |
| `GetProcAddress` | Redirects dynamic lookups of `VirtualAlloc`/`Free`/`Protect` to our hooks. Without this, a title or middleware library that resolves them at runtime bypasses all tracking. |
| Console heap allocator (`XMemAlloc`/`XMemFree` and the custom-hooks setter) | Large allocations (64 KiB and up), and all allocations when exact mode is on, go through a write-watched `VirtualAlloc`, so their pages can be tracked. Small ones use an aligned CRT allocation (4 KiB alignment above 4 KiB, otherwise 64 bytes). A title-installed custom allocator is recorded, because its memory is no longer ours to see. |
| ESRAM mapping | Implemented as an ordinary commit at the title's chosen address, with the same tracking marks. |
| Memory status query | Reports console-sized totals (5 GB or 9 GB depending on console model) minus a fixed amount in use. An optional mode reports live numbers. |

All of these take one recursive "title memory mutation" lock while tracking is on. Each runs inside a *mapping scope* (section 4.6) that suspends trap arming over the range it touches.

**Invariants.**
- Every change to the mapping of title memory passes through one hook, under one lock.
- The hook marks the range dirty *before* the change and again *after* it. A concurrent reader either sees the old mapping with its old trust, or is told the span is untrusted.
- Hooks preserve `GetLastError`. Title code reads the error after calling hooked APIs. A diagnostic that calls something which clears the error changes title behaviour.

## 2.3 The size-zero free, our most expensive one-line bug
On the console, the title frees blocks with size 0, meaning "the whole allocation". Thousands of these happen at a level load. Our first marking code read `size == 0` as "everything", and marked *all title memory* dirty on each free. Each of those passes:

- held the submission lock;
- walked about 22,000 textures and 140,000 spans.

Measured:
- On the laptop, 8,648 such holds added up to 455 s in one run.
- On the 4090, 13,051 holds added up to 482 s, the longest 0.86 s.
- Submissions waited 138 s and 243 s in total.

The fix resolves each size-0 free to its real allocation range and runs callbacks outside the lock. The submission lock wait dropped from 243 s to 0.12 s, and the 4090 ran about as many presents as with the fix off.

**Lesson.** Any "whole memory" operation in a hook is a bug waiting for a title that calls it often. Resolve console shorthand (size 0, NULL base, flag tricks) to exact ranges at the hook boundary, once.

## 2.4 Page protections and write-combining
Console title memory is write-combined. A literal translation asks the PC for `PAGE_WRITECOMBINE` pages. That is faithful but expensive the moment *we* read the memory, and the exact path reads every byte it copies.

Measured:
- Write-combined reads on Windows: about **3.2-3.8 ms per MiB**, against 0.8-1.0 ms on the Linux handheld.
- A byte-wise zero scan: about 25 MB/s per worker on write-combined pages against about 890 MB/s on cached ones.
- A readability scan went from 52 MB/s to about 4,800 MB/s when we allocated title memory as ordinary cached pages.

With cached title memory on the 4090, hitches fell from about 27 a minute to about 1 a minute, and the write-detection audit stayed clean. So we remove `PAGE_WRITECOMBINE` and `PAGE_NOCACHE` from title allocations by default. A protection with no access bits falls back to read-write.

**Platform trap.** Windows commits real pages the first time you *read* never-written memory. Linux maps a shared zero page until the first write. So "scan to see whether it is zero" has very different costs, and very different memory growth, on the two.

## 2.5 The ledger: answering "what is this memory?" without asking the OS
**What.** A *title memory map*: an interval structure fed by the hooks. It holds spans of `{allocation base, state, protection}` plus reservations, with neighbours merged. A *mutation epoch* counter is bumped after every successful alloc, free, protect, heap alloc/free or ESRAM map.

**Why.** We need "is this span committed and readable?" thousands of times per frame. Under Wine, `VirtualQuery` is a call into the wineserver. On a sibling title we measured about **250 µs per call**, and its source-readability scan used about 30 % of the title's main thread in `VirtualQuery`. Earlier, our own re-check slice of 2 ms per present could afford only about 8 such calls, so the cost of *asking* limited how many textures we could notice per second.

**Invariants.**
- The map is a cache of facts *we* caused through the hooks. Any address outside it gets one real `VirtualQuery`, and that answer is never cached.
- If the map fails to allocate, proof is switched off for the process. We fall back to the OS, never to a guess.
- A separate *readability cache* sits on top. It must be invalidated by every mark. When it went stale, a span cached as "not readable" while it actually was readable could skip its copy. That is the black-road bug again (section 6.6).

**Results.** On the sibling title the map removed the scan cost: 69 s down to 0.12 s per run. Texture visits rose from about 0.1 M to 3-3.6 M per run, because noticing became cheap. Frame rate was unchanged within noise, which is a reminder that removing one cost exposes the next.

**What we'd change.** Build the ledger on day one. Make *every* consumer use it. Keep a debug mode that compares each answer with `VirtualQuery`.

## 2.6 Allocation bookkeeping costs real time
Once the exact path is on, every title VM call does more work than the kernel call underneath. A timing observer that split each hook into "native" (inside the kernel or driver) and "wrapper" (our code) found, on the AMD desktop, title-thread hooked create/alloc time of **24.7 ms per slow frame** against 2.2 ms in an average frame:

| Hooked call | ms per slow frame | inside the OS / driver | our wrapper |
|---|---|---|---|
| VirtualAlloc | 6.6 | 5.0 | 1.5 |
| VirtualFree | 5.6 | 1.6 | 4.0 |
| Placed resource (console variant) | 2.8 | 0.6 | 2.2 |
| Committed resource | 1.7 | 0.5 | 1.2 |
| Command list creation | 1.7 | | |

More measurements:
- Per call: VirtualFree about 211 µs (35 µs in the syscall), VirtualAlloc about 123 µs (16 µs in the syscall).
- The bare kernel time was only about 0.1 ms per frame. Almost all the cost was ours, or Wine's VM bookkeeping.
- vkd3d's own time inside `CreateCommittedResource` was only 0.5 ms per slow frame. Sub-allocating committed resources would have gained almost nothing. We measured that before building it.
- The cost follows streaming. A parked car gave 34 slow frames a minute against 86-95 while racing.
- One `VirtualFree` of a tracked range did three trap-registry window scans over about 111,000 spans, five registry lock acquisitions, three `VirtualQuery` walks and two marks.

We also hit plain memory growth:
- A bundle table was never pruned: 62,475 entries after three races, 55 % dead. A sweep cut private bytes 13-15 % (about 1.7 GiB).
- Tightening small-buffer alignment through a vkd3d-proton private flag cut private memory by a third (15.3 GiB to 10.2 GiB). The flag is hidden from `GetDesc`, with fallback if anything fails.
- About 940 upload jobs a second each created a committed UPLOAD buffer. Native creates ran at about 2,460 a second and frees at about 3,470 a second.

**Under Wine, `VirtualAlloc`/`VirtualFree` take a process-wide VM lock.** Every off-thread worker that allocates scratch memory competes with the title. Our snapshot buffers alone cost about 2.5 ms of CPU per frame in VM calls on two worker threads, until we pooled them (section 5.4).

**What we'd change.**
- Keep hook work O(change), never O(registry).
- Pool every scratch allocation from the start.
- Time "inside the OS" separately from "inside us" in every hook from day one.

---
[Back to the index](../README.md)
