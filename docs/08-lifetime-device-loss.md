# 8. Residency, lifetime and device loss

## 8.1 The lifetime mismatch
The title releases a resource when, *in console terms*, the GPU is done with it. The title tracks that with its own fences. Our translation adds objects the title cannot see:

- upload buffers;
- private copies of descriptor tables;
- desktop resources standing in for title VAs;
- in some modes, resources that live longer than their title counterparts.

So "the title released it" and "nothing on the GPU can touch it" are not always the same moment for *our* objects.

## 8.2 Deferred release
**What.** An optional retirement queue. A title `Release` that would destroy a desktop resource becomes a *retirement*:

- We hold credits on the resource. All intercepted references share one logical count: the native count minus our outstanding credits.
- Each retirement records a goal per queue: an epoch plus a fence value. The resource is destroyed only after **every registered queue** has certified completion past its goal.
- Placed resources and aliased heap ranges delay the release of their backing, so an early title heap release cannot free storage that a retained resource still uses.
- The collector is a worker thread. Pressure thresholds come from the DXGI local memory budget. **Retirement never makes a caller sleep.**

**Invariants.**
- A completion value, once assigned to a captured epoch, never moves. A later queue certificate must not move an older ticket's goal.
- `UINT64_MAX` from a fence is *device removed*, not "completed".
- An undrained batch that is destroyed **leaks on purpose** rather than release something the GPU may still use.
- If the bookkeeping is ever inconsistent, the process stops with a diagnostic file. A wrong release is worse than a crash with a report.

**Outcome.** As a *device-loss fix* it did not work (section 8.4), and it stays off by default. As infrastructure it is sound, and we would build it again earlier, with the invariants above.

## 8.3 Fences, and the GPU memory guard
- **Fence waits.** A signalled event is only a wake-up. The fence *value* is the authority. Every wait re-checks `GetCompletedValue` on timeout and maps `UINT64_MAX` to device removed. A wait with a null event handle is refused, because `SetEventOnCompletion` with a null event blocks until completion, possibly forever.
- **GPU memory guard (optional).** Wraps committed resource creation:
  - samples the DXGI budget every 250 ms and estimates what was admitted between samples;
  - keeps headroom of at least 256 MiB or 10 % of the budget, capped at 1 GiB;
  - under pressure, or after an out-of-memory error, places the resource in system memory instead.
  It never invents a resource, a fake success or a smaller image.
- **Budgets the console expected.** The game reads a console-sized texture budget. Raising or lowering the budget we report changed nothing visible: peak texture memory was about 1.3 GiB either way.

## 8.4 Device loss: what we learned
This is our longest-running open problem, so here are the parts that bear on memory.

**The signature.** NVIDIA, Vulkan path, desktop and laptop:
- an MMU read fault in the graphics engine, where every resource at the faulting address had already been destroyed;
- the faulting page always in the small-mip tail (last 4 KB) of streamed block-compressed textures that are created and destroyed repeatedly at one title VA;
- the vendor crash tool reported a texture fetch through a stale or wrong descriptor, into memory freed 5-389 s earlier;
- time to loss on the 4090 at normal speed: 195-430 s. A slow laptop configuration lost the device in 5 of 5 runs.

**Fixes that did not work.**

| Attempt | Result |
|---|---|
| vkd3d-proton's own deferred resource destruction | Still lost (314 s). |
| Our deferred release behind full queue completion | Failed 3 of 3 on the laptop. A 60 s hold made it worse (3 of 6). |
| A retention patch keeping memory alive longer | 0 of 20 losses with it against 0 of 3 without: unproven. Cost +265 MiB VRAM and +1.8 GiB RAM. |
| Typed null descriptors for empty slots | Lost at 374 s despite 1,086 substitutions. |
| An NVIDIA guard copying descriptor tables at record time | Hides a class of faults, but violates descriptor volatility (section 7.5). One of its sub-guards also omitted real world draws. |

**What we learned.**
- **Fence-based holds cannot fix a stale *descriptor*.** If a copy of a descriptor points at a dead resource, holding the resource longer only moves the fault. The bug is the descriptor, not the lifetime.
- **Crash tools can name the wrong resource when GPU addresses are reused.** "Freed minutes earlier" may partly be an artefact. Log your own address history and resolve faults against it.
- **Diagnostic layers change the odds.** Our timestamp layer was the prime suspect in one traced loss: the traced row lost the device and untraced rows did not. RenderDoc-on-Vulkan captures lost the device in 2 of 2 runs (section 12.3).
- **Correlations at small n mislead.** Eager staging was "on" in 5 of 8 lost runs and 0 of 17 clean ones. A paired 6-pair A/B then gave 1 of 6 losses either way.
- **Churn may multiply the risk without causing it.** On a sibling title, a survival correlated with a frame cap and about 25x fewer copy lists (4,000 against 101,000).

**What we'd change.** From day one, keep a ring of `{descriptor heap, slot, resource, generation, time}` writes and a ring of resource creations and destructions with GPU VAs. Dump both on device removal. Our late-added slot dump was the most useful single artefact for this class of bug. Check device removal regularly on submit paths (every sixteenth `ExecuteCommandLists`, plus every queue `Signal`), log it once, and dump everything you know.

## 8.5 Commit limits
On a 32 GB machine, the Windows commit limit (about 34 GB) invalidated test runs that peaked at 27 GB private. Several runs ended with "host commit exhausted", which looked like random crashes. Watch private bytes and commit charge as well as the working set. The translator's memory overhead (snapshots, tables, retained objects) can be gigabytes.

---
[Back to the index](../README.md)
