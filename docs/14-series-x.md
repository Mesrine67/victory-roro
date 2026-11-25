# 14. What changes for an Xbox Series X translator

We have not translated a Series X title. This section combines what we learned with Microsoft's *public* descriptions of the Series X|S platform, from its 2020 announcements and architecture talks and the public DirectX 12 Ultimate and DirectStorage documentation. Verify every point against your own title.

## 14.1 Platform differences that matter for memory (public information)
- **No ESRAM.** The Series X has 16 GB of GDDR6, described publicly as 10 GB of faster "GPU-optimal" memory and 6 GB of slower "standard" memory, still one unified address space. Expect the title to *place* GPU-heavy resources in the fast range. The PC does not care, but address ranges and reservation patterns will differ from the Xbox One's, so your startup holds (section 2.1) need the title's real ranges.
- **More memory, more objects.** Every per-span, per-page and per-slot structure we built grew with memory size. Design them O(change) from the start. Our registry walks became visible at about 111,000 spans and about 22,000 textures.
- **DirectStorage and the hardware decompression block** (zlib and the console's BCPack texture codec, per the public material). **This is the biggest change to write detection.** Data can land in title memory decompressed by hardware, written by no CPU store. Page traps will not see it, and write-watch may not either, just as kernel file reads did not for us (section 4.6). Treat the storage and decompression completion as a *producer event*: an I/O ledger keyed by destination range, marked at request and completion, as our file-read ledger was. You will also need a CPU or GPU BCPack decoder, since PC GPUs do not sample BCPack.
- **Sampler Feedback Streaming and tiled resources.** Series titles are publicly expected to use sampler feedback plus reserved (tiled) resources: map only the tiles the GPU samples, and use GPU-written feedback maps to drive streaming. That changes the residency model:
  - residency becomes **per 64 KiB tile**, through `UpdateTileMappings`, not per page of a placed resource;
  - you must translate tile-mapping calls onto vkd3d-proton's sparse resources, and decide what an *unmapped* tile reads. A D3D12 tiled-resources tier 2+ unmapped read returns zero, much like our zeroed mips;
  - the "exact" contract becomes: every *mapped* tile the title wrote has current content before the submission that samples it;
  - feedback maps are GPU-written. Read them back on the GPU timeline, not by CPU polling.
- **DirectX 12 Ultimate features:** mesh shaders, DXR, variable rate shading and sampler feedback, with root signature 1.1 more likely. More GPU addresses appear in more places:
  - raw buffer pointers in root constants;
  - acceleration structure build inputs;
  - indirect argument buffers.
  Address translation (section 3.1) must cover all of them. Some, like addresses inside GPU-written buffers, cannot be translated on the CPU at all. That argues for **identity mapping of GPU VAs** where you can get it, for example by placing desktop resources at addresses you control through placed heaps and a fixed GPU VA allocator, rather than a lookup.
- **Bindless is the norm.** Modern titles index large descriptor heaps from shaders. Our reach problem (section 7.2) becomes "the whole heap is potentially reachable". The per-texture unknown-slot index, or a resource-level "dirty since last submit" set with no reach at all, is the design to start from. Root signature 1.1 *static* descriptors, if used, actually make record-time decisions legal, so check the flags per table.

## 14.2 What carries over unchanged
- Reserving the title's ranges before the driver starts, and relaunching on a lost hold.
- Hooking every VM path, including `GetProcAddress` bypasses, kernel writers and size-zero frees.
- Generations on every address.
- The submission-equivalence contract and its rejected alternatives.
- The fault-handler rules: no allocation, no static TLS, fixed lock order.
- Leases while reading memory that the title can unmap.
- Descriptor volatility under root signature 1.0.
- Wine/Proton cost facts: VM lock, `VirtualQuery`, priorities, `/proc`, timer resolution.
- The 1 ms submission experiment.
- Observer-only instrumentation and the GPU timestamp layer.

---
[Back to the index](../README.md)
