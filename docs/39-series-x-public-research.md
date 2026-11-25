# 39. Series X|S translator: what public sources say

This note deepens section 14. It uses only public sources. Each claim cites a URL that returned HTTP 200 when checked, unless marked "not auto-verified". "Public fact" means a source states it. "Inference" means our reasoning for a translator, and you should test it on your own title.

## 39.1 Memory architecture

### Public fact: Series X

- Microsoft's launch post says the console has 16 GB of GDDR6 on a 320-bit bus. It splits into 10 GB at 560 GB/s and 6 GB at 336 GB/s. [Xbox Series X tech post](https://news.xbox.com/en-us/2020/03/16/xbox-series-x-tech/)
- The same post gives the GPU as 52 compute units at 1.825 GHz, about 12 TFLOPS. It gives the SSD as 1 TB NVMe at 2.4 GB/s raw, or 4.8 GB/s compressed.
- A trade-press write-up of Microsoft's Hot Chips 32 talk says the chip uses one 320-bit GDDR6 interface, partitioned as both video memory and system memory. It also reports a 360.4 mm² die and 15.3 billion transistors. [TechPowerUp Hot Chips coverage](https://www.techpowerup.com/271115/microsoft-details-xbox-series-x-soc-drops-more-details-on-rdna2-architecture-and-zen-2-cpu-enhancements)
- I could not fetch the Hot Chips slide deck itself. Treat TechPowerUp as a secondary source for that talk. The bandwidth numbers above come from Microsoft's own post.
- The 10 GB region is called "GPU optimal" and the 6 GB region "standard". Launch coverage says games get 13.5 GB: all 10 GB of the fast region plus 3.5 GB of the slow region. About 2.5 GB of the slow region goes to the operating system. [TechRadar](https://www.techradar.com/news/xbox-series-x-full-specs-revealed-just-how-powerful-is-microsofts-next-gen-console), [GamingBolt](https://gamingbolt.com/15-xbox-series-x-and-ps5-tech-details-that-the-layman-gamer-probably-doesnt-care-about/6)
- That reservation is a launch-era number relayed by the press. Later system updates may have changed it. Not confirmed for current firmware.
- Why the two pools differ in speed: not confirmed from sources I checked. Do not build on a guess about chip mix.

### Public fact: Series S

- Microsoft's product page lists 10 GB of GDDR6 on a 128-bit bus. 8 GB runs at 224 GB/s and 2 GB runs at 56 GB/s. [Xbox Series S page](https://www.xbox.com/en-gb/consoles/xbox-series-s?source=lp)
- The page does not state the OS reservation, and I did not find a verified figure. Not confirmed.
- The ComputerBase hardware comparison covers the Series S against the Series X. I used it only for context. [ComputerBase](https://www.computerbase.de/2020-09/xbox-series-s-hardware-vergleich/)

### Public fact: Hot Chips GPU details relevant to features

- The GPU is RDNA 2 class, with 52 active compute units out of 56 physical. [ComputerBase Hot Chips article](https://www.computerbase.de/2020-08/xbox-series-x-hot-chips/)
- TechPowerUp reports hardware mesh shader support, variable rate shading, sampler feedback streaming, and an Xbox-specific feature it calls "Custom Residency Map Filtering". It also reports ray-tracing intersection hardware in each compute unit.

### Inference for translators

- **One address space, two speeds.** A title can place hot GPU resources in the fast region and CPU-side data in the slow region. A PC has no such split, so you can ignore the speed. You cannot ignore the *address ranges*. The title may encode "fast" versus "slow" in which virtual range it allocates from. Log the ranges the title requests at start-up before you decide on a reservation plan.
- **Series S is a smaller budget, not a different API.** Expect the same code paths with smaller pool sizes and different texture quality tiers. A translator that works on the X should work on the S. Test both memory profiles, because a title may pick different residency budgets per console.
- **Budget on PC.** The console exposes up to about 13.5 GB to a game (X, at launch). A discrete PC GPU may have 8 to 12 GB of VRAM. Your translator must decide what lives in VRAM and what lives in system memory. Titles that assumed unified memory may assume free CPU access to GPU data. See 39.3 for the reserved-resource consequences.
- **Bandwidth does not transfer.** Titles tuned for 560 GB/s may thrash a PC GPU's PCIe link if you move data across it per frame. Count bytes copied over PCIe per frame from the first test run.

## 39.2 Velocity Architecture, DirectStorage and write detection

### Public fact: what Microsoft says it is

- The Velocity Architecture has four named parts: the SSD, a hardware decompression block, DirectStorage, and Sampler Feedback Streaming. [Velocity Architecture post](https://news.xbox.com/en-us/2020/07/14/a-closer-look-at-xbox-velocity-architecture/)
- Microsoft quotes 2.4 GB/s raw and 4.8 GB/s effective at an assumed 2:1 ratio. These are the company's own pre-release figures, not benchmarks.
- The hardware block handles the standard LZ format and BCPack, a codec made for texture data. The post says sampler feedback streaming can give roughly 2.5 times effective I/O and memory use, on average. These are Microsoft claims.

### Public fact: the console DirectStorage API

Microsoft documents the console API in the GDK. The page is public, though it links to topics marked as NDA. I used only the public text. [GDK DirectStorage overview](https://learn.microsoft.com/gaming/gdk/docs/features/console/storage/directstorage/directstorage-overview)

- Requests go into queues. The title flushes a queue with `Submit`, or auto-submit fires when the queue passes half full.
- The hardware writes straight into the title-provided destination buffer. The page says the mapping goes "directly into each pipeline layer", so there is no intermediate copy by the CPU.
- Decompression runs in the fixed order DEFLATE (zlib, RFC 1950), then BCPack, then swizzle. Any combination is allowed. In-place decompression is supported.
- BCPack is described as "a custom entropy coder designed specifically for BCn data". Colour endpoints are separated from indices and compressed with rANS.
- Source and destination can be given as arrays of 64 KB physical pages, allocated with `XMemAllocatePhysicalPages`. This matters: the destination may not be a normal virtual range.
- Memory-sourced queues take a compressed buffer already in memory. The same hardware can then act as a DMA copy engine when no decompression is requested.
- Completion is reported in FIFO order through an `ID3D12Fence`, a status array, or an event. A notification means all earlier requests in the queue finished. The GPU can also wait on the fence.
- Cancellation is best-effort. A cancelled request still completes as "success", and the title must not use its result.
- Limits: staging buffer defaults to 32 MiB. A single request may be up to 1 GiB (source plus destination) since the October 2022 GDK. Microsoft advises many smaller requests above 512 KiB so several decompression units run in parallel.
- The console minimum guarantee is 2.0 GB/s over a 250 ms window for raw data.

### Public fact: the PC DirectStorage

- DirectStorage 1.1 added GPU decompression. It runs on any DirectX 12, Shader Model 6.0 GPU, using a vendor "metacommand" if present, or a DirectCompute fallback. [DirectStorage 1.1 release post](https://devblogs.microsoft.com/directx/directstorage-1-1-now-available/)
- The format is GDeflate, contributed by NVIDIA. It splits each stream into independent 64 KiB tiles. [NVIDIA GDeflate post](https://developer.nvidia.com/blog/accelerating-load-times-for-directx-games-and-apps-with-gdeflate-for-directstorage)
- A destination in system memory is always decompressed on the CPU. Only D3D12 resources (buffers, texture regions, subresources, tiles) can use GPU decompression.
- There is a global switch (`DisableGpuDecompression`) but no per-request way to force the CPU. Microsoft suggests a custom compression format for that case.
- The PC runtime lets you register your own decompression, and you can turn off its built-in CPU thread pool and use your own jobs.
- The desktop SDK source is on GitHub. [DirectStorage repository](https://github.com/microsoft/DirectStorage)

### Inference for translators

- **BCPack output is plain BCn.** The GDK text says BCPack is an entropy coder for BCn data. So the decompressed result is ordinary block-compressed texture data that PC GPUs sample natively. Section 14's remark that PC GPUs "do not sample BCPack" is true only of the *compressed stream*. What you need is a BCPack decoder that emits BCn blocks, running on the CPU or in a compute shader. I did not find a public description of the bitstream. Not confirmed that a public decoder exists.
- **The swizzle stage may rearrange data.** The block can apply a swizzle after decompression. If a title requests it, the bytes in title memory are in a console-specific layout. See 39.5.
- **Zlib needs no special work.** Standard zlib on the CPU is enough. Use a worker pool at below-normal priority to keep the machine responsive.
- **The hardware write is invisible to page traps.** No CPU store happens, so a page-protection or write-watch scheme will not see it. Neither will a scheme that hooks `WriteFile` or `ReadFile`. This is the same class of gap as kernel file reads.
- **Hook the API, not the memory.** A translator that *implements* `IDStorageFactoryX` itself sees every request. It knows the destination range at enqueue time, and it knows completion because it signals the fence or status. Treat each completed request as a producer event for that destination range.
- **Respect FIFO and fences.** A title may read the destination as soon as the fence passes. If you signal the fence before the data is final, you create a race that the console never had. Signal only after decompression is finished and your own dirty-range bookkeeping is updated.
- **Memory-to-memory requests too.** A title may route its own cached compressed data through the hardware, or use it as a copy engine. Cover that queue type, since a plain copy is also a producer event.
- **Physical-page destinations.** If the title passes page arrays, you must translate each 64 KB page to the range your translator mapped for it. Keep a table from physical-page handle to your mapped address.
- **Alignment and size rules help you.** File offsets need 16-byte alignment when decompressing, and sizes are bounded. Reject or log anything outside the documented rules, since it points at a bug in your own mapping.

## 39.3 Sampler feedback, tiled resources and vkd3d-proton

### Public fact: tiled (reserved) resources in D3D12

- `UpdateTileMappings` maps virtual tiles of a reserved resource onto tiles of a heap. A reserved resource may take tiles from several heaps. All mappings start as NULL. [UpdateTileMappings](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12commandqueue-updatetilemappings)
- A tile is 64 KB. The docs list standard tile shapes per format. For 32-bit formats a 3D tile is 32x32x16 texels. [Volume tiled resources](https://learn.microsoft.com/en-us/windows/win32/direct3d12/volume-tiled-resources)
- The call takes regions, then ranges with flags: normal, reuse-single-tile, NULL, or skip. It is a queue operation, not a command-list operation.
- Tier 1: reads and writes to NULL mappings are undefined. Tier 2 and up: reads from a NULL-mapped tile return zero, and writes are discarded. Tier 2 also adds shader LOD clamp and a status return. Tier 3 adds 3D textures. [D3D12_TILED_RESOURCES_TIER](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ne-d3d12-d3d12_tiled_resources_tier)
- Feature level 12_0 hardware must support tier 2 or higher (same page).

### Public fact: sampler feedback

- Sampler feedback records which texture regions a shader sampled. Its two uses are texture streaming and texture-space shading. [Sampler Feedback spec](https://microsoft.github.io/DirectX-Specs/d3d/SamplerFeedback.html)
- There are two map types. MinMip stores the most detailed mip requested per region. MipRegionUsed stores a used/unused flag per mip per region.
- Feedback maps are opaque, single-sampled, UAV-capable resources. The map itself cannot be tiled. The paired texture must be single-sampled.
- A streaming system pairs a MinMip feedback map with a tiled texture. The app must keep a separate residency (clamp) map up to date whenever it maps or unmaps tiles.
- The spec recommends tiled resources tier 2 for streaming. Support is queried through the Options7 feature data. Tier 0.9 limits addressing to wrap and clamp. Tier 1.0 allows all modes.
- Microsoft's original blog introduces the feature and its streaming purpose. [Sampler feedback blog](https://devblogs.microsoft.com/directx/coming-to-directx-12-sampler-feedback-some-useful-once-hidden-data-unlocked/)

### Public fact: vkd3d-proton and reserved resources

vkd3d-proton implements reserved resources with Vulkan sparse binding, and falls back when the GPU lacks it. What the public tracker and release notes show:

- **Sparse 3D textures (tier 3)** arrived in release 2.4. [GamingOnLinux on 2.4](https://www.gamingonlinux.com/2021/07/vkd3d-proton-24-is-out-with-performance-improvements-for-direct3d-12-to-vulkan/)
- Release notes mention optimised sparse binding CPU overhead, a rewrite of sparse queue submission, a use-after-free fix in some sparse update cases, an overflow fix for sparse resources above 4 GiB, exposure of sparse TIER_4, and a start-up crash fix for GPUs without sparse support. [Releases page](https://github.com/HansKristian-Work/vkd3d-proton/releases) I confirmed these strings through the release feed.
- **Depth-stencil and MSAA sparse** are handled by a "crude workaround": the code allocates a committed resource instead. The notes say it is "not correct" but good enough. [Release 2.13](https://github.com/HansKristian-Work/vkd3d-proton/releases/tag/v2.13)
- A commit stops reporting tiled-resource support for depth-stencil formats, because D3D12 does not support it and the code validates at creation. [Commit dd7ad6d](https://github.com/HansKristian-Work/vkd3d-proton/commit/dd7ad6d2545cb79a27f907d8512c4f1c1520c24f)
- **No sparse support in the Vulkan driver.** Issue 3425 reports an out-of-memory crash on an Intel integrated GPU. With no sparse support, about 18,000 reserved textures fell back to committed allocations and filled one heap of roughly 12 GiB. A maintainer called out-of-memory "kind of expected" on hardware below a game's minimum spec. The issue was closed as not planned. The reporter's idea of demand-paged emulation of tile mappings was not implemented. [Issue 3425](https://github.com/HansKristian-Work/vkd3d-proton/issues/3425)
- **Fallback tiling data.** Pull request 3416 fixed `CopyTiles` for fallback committed resources, which had left format properties zeroed and produced invalid Vulkan copy commands. [PR 3416](https://github.com/HansKristian-Work/vkd3d-proton/pull/3416)
- **A user analysis, not a maintainer statement.** Issue 2884 is a crash report on a recent game. A user argues the game needs full tier 2+ sparse support and that hiding sparse at the Vulkan layer made it crash at launch. The thread is open, and the claim is unverified. [Issue 2884](https://github.com/HansKristian-Work/vkd3d-proton/issues/2884)
- Whether vkd3d-proton implements the sampler feedback shader objects: **not confirmed.** I did not find a verified statement either way. Check the current release notes before you rely on either answer.

### Inference for translators

- **Residency is per 64 KB tile.** A streaming title calls `UpdateTileMappings` often. Cost per call matters. The maintainers already rewrote the sparse submission path to cut CPU overhead. Measure it on your title.
- **Your "exact" contract becomes per mapped tile.** Every tile the title has mapped and written must hold current content before the submission that samples it. Unmapped tiles read zero at tier 2 and above, so you need not fill them.
- **Do not rely on the fallback.** If you run on a GPU without sparse binding, a title with thousands of reserved textures can exhaust memory. Require sparse support on PC, or build your own demand-paged layer on placed resources. The fallback path is a last resort and is not tested by the project for large streaming titles.
- **Feedback maps are GPU-written.** The title reads them back to decide what to stream. If your stack does not implement sampler feedback, the title's streamer will see empty or invalid maps. Likely result: it streams too little or too much. Decide early whether to emulate feedback with a compute pass that records sampled regions. That is a large piece of work.
- **Depth-stencil sparse is a known gap.** If a title reserves depth targets, expect the committed-resource workaround and wrong semantics. Log any such resource.
- **Linux adds driver risk.** Sparse correctness depends on the Vulkan driver. Public reports name driver bugs around sparse access on specific GPUs. Test one AMD and one NVIDIA configuration.

## 39.4 DirectX 12 Ultimate features and what they change

### Public fact: the feature set

- Feature level 12_2 requires: raytracing tier 1.1, variable rate shading tier 2, mesh shaders tier 1, sampler feedback tier 0.9, resource binding tier 3, tiled resources tier 3, conservative rasterization tier 3, root signature 1.1, Shader Model 6.5, and 40 GPU virtual address bits per process. [Feature level 12_2 spec](https://microsoft.github.io/DirectX-Specs/d3d/D3D12_FeatureLevel12_2.html), [Microsoft blog](https://devblogs.microsoft.com/directx/new-in-directx-feature-level-12_2/)
- Microsoft says DirectX 12 Ultimate targets PC and next-generation Xbox. The post does not name the Series X|S models. Hot Chips coverage ties the hardware to these features (see 39.1).

### Mesh shaders

- Mesh and amplification shaders replace the vertex, hull, domain and geometry stages. The input assembler is off, so there are no index buffers or input layouts. Launch is `DispatchMesh`, with an indirect form. The amplification shader passes a payload (up to 16 KB) to mesh groups. [Mesh shader spec](https://microsoft.github.io/DirectX-Specs/d3d/MeshShader.html)
- Translator effect: vertex data is read by the shader from buffers, usually by address or bindless index. You cannot track geometry by input-layout slots. Your dirty tracking must treat any buffer the mesh shader can reach as read.
- Indirect `DispatchMesh` arguments live in GPU-written buffers, so you cannot translate them on the CPU.

### Raytracing (DXR 1.1)

- Tier 1.1 adds inline raytracing through `RayQuery` from any shader stage, and GPU-initiated `DispatchRays` through `ExecuteIndirect`. Top-level instances point to instance descriptors in GPU memory. [DXR spec](https://microsoft.github.io/DirectX-Specs/d3d/Raytracing.html) My fetch truncated the page. I did not read the sections on acceleration-structure inputs or incremental state-object additions.
- Translator effect: build inputs and instance descriptors are GPU virtual addresses stored in memory. The addresses inside those structures are the title's console addresses. You must rewrite or map them. This is the hardest case for address translation, because the GPU, not the CPU, may have written the structure. See the identity-mapping idea in section 14.

### Variable rate shading

- Tier 1 sets a rate per draw (1x1, 1x2, 2x1, 2x2). Tier 2 adds a per-primitive rate from `SV_ShadingRate` and a screen-space image of 8-bit rates, with 8 or 16 texel tiles. [VRS spec](https://microsoft.github.io/DirectX-Specs/d3d/VariableRateShading.html)
- Translator effect: the shading-rate image is a GPU resource the title writes. Treat it as an ordinary render input. If the PC GPU lacks tier 2, you can ignore VRS at a quality and speed cost. Nothing in the spec requires you to honour it. Check the title's output for visible artefacts first.

### Sampler feedback

- Covered in 39.3.

### Root signature 1.1 and bindless

- Version 1.1 lets the app promise that descriptors or their data are static. In 1.1, descriptors are static by default, and an app must set `DESCRIPTORS_VOLATILE` to change them after recording. Version 1.0 behaves as fully volatile. Static descriptors also make out-of-bounds buffer view access undefined, where volatile ones may return defaults. [Root Signature 1.1](https://learn.microsoft.com/en-us/windows/win32/direct3d12/root-signature-version-1-1)
- Resource binding tier 3 allows very large descriptor heaps indexed from shaders.
- Translator effect: with 1.1 flags, you may make record-time decisions on static ranges. With volatile ranges or a 1.0 signature, treat the whole heap as reachable at submit time. Check flags per table.

### Net effect

- **GPU addresses appear in more places:** root constants, acceleration-structure inputs, indirect arguments, GPU-written buffers.
- **Shader-visible reach grows.** Mesh shaders and bindless make "which resource did this draw read" hard to answer at record time. Plan a per-submission dirty set instead.
- **Plan for feature gating.** Query the tier for each feature on the PC GPU and have a documented fallback or a clear failure for each.

## 39.5 Texture tiling and swizzle: only what is public

### Public fact

- D3D12 defines four texture layouts: unknown (driver chooses), row-major, 64 KB undefined swizzle, and 64 KB standard swizzle. [D3D12_TEXTURE_LAYOUT](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ne-d3d12-d3d12_texture_layout)
- Undefined swizzle arranges data in 64 KB tiles, but the order of texels inside a tile is up to the driver. Tail-mip packing is adapter-specific.
- Standard swizzle defines the in-tile order with a bit-interleave of X and Y. The page gives masks for a 32-bit-per-texel 2D case (X bytes `1010 1010 1000 1111`, Y `0101 0101 0111 0000`) and shows the address as a bit-deposit of each coordinate. Only some adapters support it.
- Volume tile shapes are listed per format in the volume page (39.3).
- The Series X GPU is publicly described as RDNA 2 class. The D3D12 runtime hides the in-tile layout on PC.
- A separate open source stack documents GPU tiling in general terms, which gives background on how block-linear and swizzled layouts work. [Mesa ISL tiling notes](https://docs.mesa3d.org/isl/tiling.html) This page covers Intel layouts. I did not verify that it describes any console layout.
- The GDK text says the decompression hardware has a swizzle stage (39.2). It does not describe the pattern in the public text.

### Inference for translators

- **Do not assume the console in-memory layout equals any PC layout.** If a title hands the GPU a texture in a console tiled layout, a PC GPU must see it in a PC layout. Either detect it, or have the translator control creation so both sides agree.
- **Titles that rely on tiled layout have two choices on your side.** One: detect the console layout and de-swizzle on upload to a PC-native texture. Two: keep data in the console layout and convert inside your own compute pass when the GPU samples it. The first is simpler. The second is cheaper in memory.
- **You need ground truth, from a public source or from your own title.** Without public layout documentation, derive the pattern by black-box tests: write a known gradient through the title's own upload path and read back the bytes. Document the result in your own notes. Do not copy console SDK material.
- **Reserved and tiled textures on PC use opaque in-tile order.** Tier 1 to 3 hardware may use any order inside each 64 KB tile. So `CopyTiles` is the only portable way to move data into a PC reserved texture. A translator cannot write tile bytes directly and expect them to sample correctly.
- **Standard swizzle is a possible bridge** only if the PC adapter supports it. Query it. Do not depend on it.

## 39.6 Public projects and talks

I searched for public projects that run Xbox One or Series games on PC. Findings are limited, and I describe only what the sources state.

- **Xbox 360 on PC.** Xenia is an open source Xbox 360 emulator for PC. A trade press report records that the Xenia team denied working with Microsoft on a PC emulator. [GamesRadar](https://www.gamesradar.com/games/the-biggest-unofficial-xbox-360-emulator-is-by-no-means-associated-with-nor-working-alongside-microsoft-in-any-capacity-despite-rumors-about-xboxs-preservation-efforts) I could not verify a public description of its GPU or texture code.
- **Xbox One and Series on PC.** I found no public project that claims to run Xbox One or Series console titles on PC. Not confirmed that none exists. The search was limited.
- **PC games from Game Pass on Linux.** Press describes an open source project called Xodus, from the authors of a game launcher. The headline says it brings Xbox PC and Game Pass titles to Linux through reverse engineering. The Tom's Hardware body text was not readable in my fetch, so I cannot state its approach or status. [Tom's Hardware headline (page loads, article body not read)](https://tomshardware.com/software/linux/xbox-pc-and-game-pass-titles-are-coming-to-linux-through-xodus-heroic-launcher-devs-embark-on-new-open-source-reverse-engineering-project) This targets the PC GDK, not console binaries.
- **Microsoft's own backward compatibility** on Xbox One and Series for Xbox 360 titles is described by the same GamesRadar piece as official software, not a fan project.
- **Talks.** I did not find a public conference talk on translating console GDK titles to PC. Not confirmed.

### Inference for translators

- Few public references exist for the *console-to-PC* direction. The nearest public material is the PC side: D3D12 specs, vkd3d-proton's tracker, and DirectStorage. Expect to build most things yourself.
- A PC GDK build of the same title, if it exists, is a useful reference for behaviour differences (for example which code paths use DirectStorage). Use only public builds you are entitled to run.

## 39.7 Checklist for the first month

1. Log every virtual range the title reserves before it creates a device. Note ranges that look like "fast" or "slow" pools.
2. Log every DirectStorage request: source type, options, destination type, size, fence. This tells you which producer events you must cover.
3. Count reserved resources and `UpdateTileMappings` calls per second in a typical level. Compare with vkd3d-proton's release notes on sparse overhead.
4. Check which DX12 Ultimate features the title actually creates (mesh pipelines, DXR state objects, VRS images, feedback maps). Gate your work on that list.
5. Dump root signature versions and flags for every signature the title creates.
6. Build a black-box test that finds the texture layout the title uploads, before you write any de-swizzle code.
7. Decide, in writing, what an unmapped tile reads, and test it on tier 2 hardware.

## 39.8 Sources kept and dropped

Kept (HTTP 200 when checked): 35 distinct URLs, listed inline above. One is marked "page loads, body not read" because I could not read the full article.

Dropped: the ISSCC 2021 slide PDF on a third-party host (no response), the Xbox 360 emulator wiki page (403, bot block), a Xenia docs mirror (HTTP 410), and other search results that I could not fetch or that only repeated these facts.

---
[Back to the index](../README.md)
