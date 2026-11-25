# 3. GPU resources over title memory

## 3.1 The console "placed at address" call
The console API places a resource at a title address. We cannot place a desktop resource at an arbitrary CPU address, so:

1. We create a real desktop resource. In practice it is committed. A placed-heap scheme is possible, but this is the version that shipped.
2. We record `{title VA, size, desktop VA, generation}` in an address registry under a reader-writer lock.
3. We patch that resource's vtable:
   - `GetGPUVirtualAddress` returns the **title VA**, because the title frees memory by calling `free()` on that value;
   - `Release` is replaced, so we see destruction.
4. Every GPU address the title passes back to the API is translated title VA → desktop VA. That covers vertex and index buffer views, root CBV/SRV/UAV, indirect arguments and bundle contents. The translation is an ordered-map interval lookup, with a cached last interval keyed on a registry revision counter.

```
title VA space                         desktop GPU VA space
[ A gen 7 ][ B gen 3 ]   ──lookup──►   A' (resource for gen 7), B'
      ▲ title destroys A gen 7, immediately places C at the same base
[ C gen 8 ]                            C'   (A' may still be in flight on the GPU!)
```

## 3.2 Generations, address reuse and overlap
The title creates about 1,000 resources a second, and a destroyed resource's address is often reused by a new one *before our `Release` hook has removed the old entry*. So:

- every registry entry carries a **generation**, and every lookup is "address plus generation" wherever we can know it;
- when ranges overlap, we keep the newest live generation at each base. A fallback scan handles the case where an ended nearer interval hides a live older one, and logs it;
- a separate *identity classifier* sorts a descriptor's texture into `Current`, `OlderCompatible`, `Ambiguous` or `Unknown` among the live textures at the same title VA. It never dereferences anything. This is our ABA defence for diagnostics.

**Failure we hit.** On NVIDIA under Vulkan, device losses consistently pointed at a GPU read of memory that had been freed 5 to 389 seconds earlier. The faulting page was always in the last 4 KB (the small-mip tail) of streamed block-compressed textures, created and destroyed repeatedly at one title VA (section 8.4). Address reuse plus a stale descriptor copy is still our leading theory.

**What we'd change.** Make generation part of every handle from the start, including the ones that live inside descriptor bookkeeping. "Address equals identity" is never true here.

## 3.3 Buffers
- **The title `Map`s DEFAULT-heap buffers.** We reroute DEFAULT-heap buffers to a CPU-visible heap: GPU_UPLOAD (Resizable BAR VRAM) where available, otherwise a CUSTOM write-combined system-memory heap. An option forces write-back system memory instead.
- **Zero-copy title backing (optional).** For buffers, we can wrap the title's own pages as a heap with `ID3D12Device3::OpenExistingHeapFromAddress` and place the buffer at the right offset. The GPU then reads the title's memory directly, as on the console. Conditions are strict:
  - the whole span must be private, committed, read-write and not a guard page;
  - the wrapping heap is kept alive as private data on the resource, with no global cache, so no address-reuse confusion;
  - anything unusual falls back to the normal path.
  It is off by default. It works, but the interaction with our write traps and with Wine needs more proof than we have given it.
- **Buffer churn.** An optional slab allocator pre-creates 64 MiB buffer-only heaps (up to 1.5 GiB in total) on a low-priority thread and recycles buffers up to 4 MiB by key. Reuse is gated on fence completion on *all* queues and on pins, and memory is zeroed before reuse. It exists because of the 1,000 creates a second. It stayed off: the measured native create cost turned out small (section 2.6).
- **Console ABI quirks.** The console's custom-heap-properties call puts its hidden return buffer in a different place from the desktop one, so we need an adapter. Some console heap flags have no desktop equivalent and must be stripped. We retry heap creation without an optional "not zeroed" flag where a driver refuses it.

## 3.4 Textures
- A console tiled texture is registered in a *tiled texture table* with its detiling helper (the console's tiling library computes the layout), its epoch, and its exact-tracking state.
- The desktop texture is a normal DEFAULT resource. Its content is always uploaded by us (sections 4-5).
- **Committed resources start zeroed.** A mip we have not copied yet reads as zero, which is black. There is no placeholder, and the GPU does not wait. That one fact explains the whole "black pop-in" history in section 6.

---
[Back to the index](../README.md)
