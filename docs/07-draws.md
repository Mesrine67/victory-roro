# 7. Where draws meet memory

## 7.1 What a draw needs from us
On the console, a draw reads whatever is in memory when the GPU executes it. On the PC, by the time a command list reaches the GPU, we must have:

1. translated every title GPU address in it to a desktop address (section 3.1);
2. made sure every texture it can sample has current content (section 5);
3. made sure every descriptor it reads names a live desktop resource of the right type (section 7.5).

The first is done while the title records commands, in our command-list wrappers. The other two are settled at submission. The hard part is item 2's first question: **which textures can this submission reach?**

## 7.2 Draw reach
**Why it is hard.** A draw does not list its textures. It binds descriptor tables: pointers into descriptor heaps. Its root signature and shaders decide which slots it reads. Our game uses **unbounded descriptor tables**, which we discovered while testing an early "prepare at submit" design. That design found 40.6 million unresolved table references, because an unbounded table from a non-zero base could in principle reach any later slot in the heap.

**How we answer it.**
- While recording, our command-list wrappers keep the *binding state* of each list: root signature, pipeline, bound tables per root parameter, and the heaps set.
- At each draw, we resolve which (heap, slot range) pairs the bound shader can read. We use reflection where it is precise, and the declared ranges where it is not. Repeated draws with an unchanged binding revision are skipped.
- Each descriptor heap keeps **attention sets**: which slots currently name a tracked texture, and which slots are "unknown" (unregistered, dying, stale, or of a layout we cannot classify).
- At submission, the union over the submitted lists is the reach.

**Cost history of reach per direct submission:**

| Version | Cost per submission | Why |
|---|---|---|
| Whole-heap attention walk | about 5,235 µs | Visited about 21,600-24,500 *unknown* slots per submission, which belonged to only about 28-37 textures. |
| Per-heap (class, resource) table | about 324 µs | Same answers, fewer visits. A second reviewer re-derived every count and could not break it. |
| Per-texture index of unknown slots | **14.2 µs** mean (p99 under 63 µs) | Under our 30 µs budget. |

**Invariant.** Reach may over-include (copy something not needed). It must never under-include. Every shortcut is checked against the slow path in test builds by a *differential* mode: 14.18 million bundle checks and 5.99 million direct-draw checks with 0 mismatches before the fast path became the default.

## 7.3 Bundles: in-place reach
The title records many small bundles, at about one new bundle buffer per bundle, and executes them from direct lists. A bundle can set the pipeline, root signature and tables, and those settings are visible to the caller afterwards.

- **Default handling.** For each `ExecuteBundle`, compute the bundle's reach ranges from a cache and apply them to the caller's state. That made 13.5 million cache misses (and 5 hits) per run.
- **In-place handling.** Walk the bundle's recorded commands and replay only the *immutable metadata setters* (pipeline, tables, root parameters) into the caller's binding state. Resolve reach at each draw inside the bundle. **Never replay native commands.** Then restore the caller's topology and bump its binding revision exactly as before. A memo variant caches per-(binding revision, pipeline) results.
- **Result.** -1.5 to -3.5 ms per frame on the laptop, with 0 mismatches in the differential check.

**Lesson.** The cache that seemed obviously right missed almost always, because bundle buffers are recreated constantly. Measure the hit rate before trusting a cache.

## 7.4 Descriptor writes, "flat" bookkeeping and release-flip
To keep attention sets current, we observe every descriptor write: `CreateShaderResourceView` and friends into a heap slot, and `CopyDescriptors`. Our game makes millions of these.

**First version.** Tree maps per heap under one global mutex on the title thread:
- 30.2 million acquisitions in one run, 47.4 s of wait plus hold (4.2 ms per present);
- with everything else in the exact path switched off, this bookkeeping alone made the title run at 0.67 of the control's frame rate.

**Flat bookkeeping.** Per-heap flat arrays indexed by slot, with per-heap locking, and an early-out for writes that change nothing about tracked state. About 20 million writes per run took the lock-free early-out.
- Title-thread descriptor lock time dropped from **4.5 ms to 0.47 ms per present**.
- The side-effect-only cost went from 0.67 to **0.97** of control.
- Median frame on the laptop with the fix on went from 44.3 ms to 32.2 ms (control 22.4 ms).

A shadow mode builds the old maps in parallel and compares every answer: 0 flat mismatches. Flat bookkeeping is now the default.

The descriptor writes we *keep* still cost about 2.5 µs each (about 4.5 ms per present on the title thread in early builds), so their count matters.

**Release-flip.** When the title releases a resource, the exact path flips that texture's state to "dying" and reads the generation, in one hold of the texture lock. That keeps any later reach from treating a dying texture as current. We first got this wrong in a subtle way: a fast path that skipped the lock for plain buffers fired **0 times**. The real flag combination always reported "exact active", while the test fixture had stubbed that flag. Test with the real switch combinations.

**The release path in numbers** (AMD desktop):
- about 2,500 hooked `Release` calls per frame, 80 % of them for resources the exact path does not care about;
- each originally took the texture lock;
- the presenting thread took that lock about 1,400-1,450 times per frame (0.30 ms wait plus 0.27 ms hold).

A lock-free negative filter for untracked resources, plus a release fast path, cut the presenter's acquisitions from about 1,443 to 303 per frame (138.5 million filtered, 0 missed). This was a **real hitch win**: AMD desktop race-1 slow frames went from 98.2 to 85.5 per minute, and the handheld from 161.3 to 129.9 (-19 %).

## 7.5 Descriptor volatility
D3D12 root signature 1.0, which our game uses, treats table descriptors as **volatile**:
- the application may rewrite a descriptor after recording a list and before submitting it;
- the only forbidden window is "submitted and not finished executing";
- the data the descriptors point to is volatile in the same way.

(This is in Microsoft's public root signature version 1.1 documentation, which defines 1.0 behaviour as "descriptors volatile, data volatile".) vkd3d-proton honours this by treating every 1.0 descriptor as update-after-bind/bindless.

**Why this matters to a translator.** Any design that **copies a descriptor at record time** and binds the copy is not 1.0-conformant. The title may legally:
1. record a list using slot S;
2. rewrite S to point at texture B;
3. release texture A, the old content of S, because no submitted list references it;
4. submit.

A record-time copy still points at A, now destroyed: a use-after-free on the GPU. We had exactly such a copy in an NVIDIA-specific guard that rebinds tables to private copies. An audit at submit counted frozen slots that the title had `rewritten`, `changed`, `dead` or `dead-changed` since recording. The last one is the dangerous case. The legal fix is to **refill changed slots at submit**. No fence wait is needed: if the title changed a slot, no in-flight list can be reading it, or the title itself broke the rule.

**Caveats we found.**
- A slot that no shader references may legally change while a list is in flight. A refill that reads *every* slot can race there.
- Lists can be submitted twice. Refill only slots that changed.

## 7.6 Per-hook costs: none of them is the hitch
A sampled timing observer (1 in 64 calls) on the five draw-path hooks (flat-copy, default bundle, in-place bundle, draw reach and release-flip) measured them in normal and slow frames:

- Together they explain only **1.5-3 ms (15-25 %)** of the +9 to +12 ms of extra title-thread time in slow frames.
- The largest single difference is release-flip: +0.98 ms, with about **3x the calls** in slow frames. Slow frames are frames where the title releases a lot, which is streaming again.
- Named loader locks explain about 3 ms of the 12-13 ms the presenter spends blocked.

We switched off groups of hooks to price the whole side-effect cost on the laptop:

| Hook group off | Frames vs control |
|---|---|
| Barrier skipped | 0.54 |
| Draw recording off | 0.55 |
| Bundle reach off | 0.58 |
| Descriptor path off | 0.68 (the largest single item; it also slows the title's own code, probably through cache pressure) |
| All five hooks off | 0.85, so about 15 % lies outside the hooks entirely |

**Lesson.** Per-draw instrumentation costs grow with draw count, about 35-40 µs of extra cost per additional draw in early builds. Design draw-path hooks as table lookups, not searches, from the start.

---
[Back to the index](../README.md)
