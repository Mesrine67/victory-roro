# 1. The memory model gap

## 1.1 What the console gives the title
On the Xbox One, the title sees one physical memory pool with a unified address space. A few properties matter here:

- **CPU pointers are GPU addresses.** When the title places a resource at an address, the resource's GPU virtual address *is* that pointer. Titles rely on it. Ours frees a released resource's memory with `free(resource->GetGPUVirtualAddress())`.
- **The CPU writes GPU-visible memory directly.** Textures and buffers are filled by plain CPU stores into title memory, usually in the console's tiled layout. They are also filled by DMA engines and by file reads that land in that memory. No upload step, no copy, no `Map`/`Unmap` bracketing is needed.
- **Most title memory is write-combined.** That is fast for streaming writes and slow for CPU reads.
- **ESRAM.** There is a small, fast on-chip memory that the title "maps" at addresses it chooses.
- **Address ranges are fixed.** The title reserves large fixed virtual ranges at startup and carves its own pools from them. If a reservation fails, our game does not crash at once. It records a pool of size zero and later writes off the end of a heap block.
- **Placement is cheap.** A "placed resource at this address" call costs almost nothing, and the title makes them constantly. Ours creates about one resource per bundle buffer, roughly 1,000 a second, and destroys them as fast.
- **Descriptors are plain memory too.** A descriptor table is a block of memory the title writes with the CPU. The GPU reads it when a draw executes.

## 1.2 What a PC gives us
On desktop D3D12, and on vkd3d-proton, which maps D3D12 onto Vulkan:

- **CPU and GPU address spaces are separate.** A resource has a GPU VA chosen by the driver. You cannot ask for "this GPU address equal to that CPU pointer".
- **Heaps have types.** DEFAULT is GPU-local and not CPU-visible. UPLOAD is CPU-write, GPU-read, write-combined. READBACK goes the other way. CUSTOM heaps let you choose CPU page properties and memory pool. GPU_UPLOAD (Resizable BAR) is CPU-visible VRAM where the platform supports it.
- **Textures live in driver-chosen layouts.** The CPU cannot write a DEFAULT texture's bytes. Data arrives by a GPU copy from a buffer, or through an UPLOAD/CUSTOM placement with linear layout, which is restricted and slow.
- **Content must be copied explicitly.** Someone has to notice the CPU wrote, read those bytes, convert them (detile), and record a copy before the GPU uses them.
- **Placed resources need a heap.** `CreatePlacedResource` needs a heap object and an offset, not an arbitrary CPU address. `OpenExistingHeapFromAddress` can wrap existing CPU memory as a heap, with limits (section 3.3).
- **Lifetime is the application's job.** D3D12 already says the application must not free a resource while the GPU may still use it. The title obeys that *in console terms*, which do not always match the copies and stand-ins we create.

## 1.3 The gap as a picture

```
CONSOLE                                    PC (our translation)
-------                                    --------------------
title CPU stores ──► title memory ◄── GPU  title CPU stores ──► title memory (plain CPU pages)
                     (one copy)                                    │
                                                                   │ 1. detect the write  (traps / write-watch / I/O ledger)
                                                                   │ 2. read + hash + detile (snapshot)
                                                                   ▼
                                                              UPLOAD buffer
                                                                   │ 3. CopyTextureRegion, recorded on the same queue,
                                                                   │    ahead of the title's lists
                                                                   ▼
                                                    desktop texture (DEFAULT heap) ◄── GPU
title "GPU VA" = title pointer             title VA ──► (interval map, generation) ──► desktop GPU VA
```

The rest of the document is about the three numbered arrows and the address arrow at the bottom: making them correct, then making them cheap.

## 1.4 What we decided early, and would decide again
- **Textures are never aliased onto title memory.** The GPU always reads a desktop texture we own, and we copy content into it. Detiling alone makes direct aliasing impractical for the console's tiled layouts. It also gives us a clean point to enforce exactness.
- **Buffers can be CPU-visible.** The title `Map`s buffers that live in DEFAULT heaps, so we move DEFAULT-heap buffers into a CPU-visible heap (section 3.3).
- **Address identity is preserved for the title.** Every GPU address the title sees is a title VA. We translate it to the desktop GPU VA when the title hands it back to the API.

---
[Back to the index](../README.md)
