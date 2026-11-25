# Public references: an annotated bibliography

Sources gathered while building a PC translator for Xbox One D3D12.X titles on top of Vulkan. Each entry gives the page title, its address and a short note in our own words on why it mattered. Every address was fetched with a script and returned HTTP 200 unless marked otherwise.

## 1. D3D12 specification and resource binding

- **Resource Binding (DirectX specs)**  
  <https://microsoft.github.io/DirectX-Specs/d3d/ResourceBinding.html>  
  Establishes that a descriptor does not keep its resource alive, so a translator that releases native objects when the guest drops a descriptor must track ownership itself.

- **Advanced use of descriptor tables**  
  <https://learn.microsoft.com/en-us/windows/win32/direct3d12/advanced-use-of-descriptor-tables>  
  CPU edits to shader-visible descriptors the GPU may still read must wait for completion or use versioned copies; we used it to judge descriptor-volatility hazards.

- **Descriptors overview**  
  <https://learn.microsoft.com/en-us/windows/win32/direct3d12/descriptors-overview>  
  Clarifies that a descriptor heap is only descriptor storage, so keeping it alive says nothing about the images it points at.

- **Root Signature Version 1.1**  
  <https://learn.microsoft.com/en-us/windows/win32/direct3d12/root-signature-version-1-1>  
  Defines the DESCRIPTORS_VOLATILE and DATA_VOLATILE flags and what each promises; relevant when deciding whether descriptors can change after being bound.

- **ID3D12Device::CopyDescriptors**  
  <https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12device-copydescriptors>  
  The page is silent on thread safety and in-use overlap, so those rules have to be taken from the binding spec and the CPU-efficiency notes instead.

- **D3D12 CPU efficiency specification**  
  <https://microsoft.github.io/DirectX-Specs/d3d/CPUEfficiency.html>  
  States which device and queue methods are free-threaded and which command-list methods are not; a baseline for deciding where a translator needs its own locks.

- **Fence-based resource management**  
  <https://learn.microsoft.com/en-us/windows/win32/direct3d12/fence-based-resource-management>  
  The canonical ring-buffer pattern of polling a fence and freeing resources behind it; a model for deferred release.

- **Using resource barriers to synchronize resource states**  
  <https://learn.microsoft.com/en-us/windows/win32/direct3d12/using-resource-barriers-to-synchronize-resource-states-in-direct3d-12>  
  Reference for barrier semantics and batching when mapping D3D12 states onto Vulkan layouts.

- **ID3D12Device::CreatePlacedResource**  
  <https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12device-createplacedresource>  
  Notes that placed render-target and depth resources must be initialised after an aliasing barrier; important for heap-aliasing titles.

- **ID3D12Device::GetCopyableFootprints**  
  <https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12device-getcopyablefootprints>  
  Invalid descriptions return an all-ones total size, which explained a bogus size feeding a later copy fault.

- **D3D12 tight placed resource alignment**  
  <https://microsoft.github.io/DirectX-Specs/d3d/D3D12TightPlacedResourceAlignment.html>  
  Describes tight alignment as a feature-gated extension; a translator that hides the feature must not leave a resource with tight-only alignment.

- **D3D12_HEAP_FLAGS**  
  <https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ne-d3d12-d3d12_heap_flags>  
  Reference for heap flags (including the not-zeroed option) when mirroring console allocation behaviour.

- **Coming to DirectX 12: more control over memory allocation**  
  <https://devblogs.microsoft.com/directx/coming-to-directx-12-more-control-over-memory-allocation/>  
  Says new heaps and committed resources normally start zeroed and that the not-zeroed flag is only an optimisation, so zero-fill cannot be relied on to be skipped.

- **Uploading resources**  
  <https://learn.microsoft.com/en-us/windows/win32/direct3d12/uploading-resources>  
  Documents upload-heap use and the rule that CPU writes must complete before GPU reads; mapping alone is not synchronisation.

- **User-mode heap synchronization**  
  <https://learn.microsoft.com/en-us/windows/win32/direct3d12/user-mode-heap-synchronization>  
  Confirms that unified memory removes a copy, not the need to order conflicting accesses between CPU and GPU.

- **D3D12 residency**  
  <https://learn.microsoft.com/en-us/windows/win32/direct3d12/residency>  
  Explains budgets, over-budget behaviour and why destroying objects frees memory more reliably than evicting.

- **DirectXTK12 XboxDDSTextureLoader**  
  <https://github.com/microsoft/DirectXTK12/wiki/XboxDDSTextureLoader>  
  Shows the console pattern of creating a placed resource over caller-owned memory and freeing that memory separately, a hint about how titles own backing store.

- **Device Removed Extended Data (DRED) specification**  
  <https://microsoft.github.io/DirectX-Specs/d3d/DeviceRemovedExtendedData.html>  
  Describes auto-breadcrumbs and page-fault reporting with recently freed nodes; useful on native Windows D3D12 to cross-check a Vulkan-side loss.

- **ID3D12Device::GetDeviceRemovedReason**  
  <https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12device-getdeviceremovedreason>  
  Returns success until removal, making it a clean trigger point for dumping diagnostics.

- **Process residency budgets (display driver docs)**  
  <https://learn.microsoft.com/en-us/windows-hardware/drivers/display/process-residency-budgets>  
  Background on how the OS assigns per-process video-memory budgets, relevant to overcommit and late-texture symptoms.

## 2. vkd3d-proton internals and configuration

- **vkd3d-proton repository and README**  
  <https://github.com/HansKristian-Work/vkd3d-proton>  
  Main source for how a D3D12-to-Vulkan layer is built; the README lists the VKD3D_CONFIG switches we compared against.

- **vkd3d-proton device workarounds (source)**  
  <https://github.com/HansKristian-Work/vkd3d-proton/blob/master/libs/vkd3d/device_workarounds.c>  
  A catalogue of per-title workarounds (colour compression, forced barriers); a useful checklist of ways games break the spec.

- **vkd3d-proton pipeline cache implementation (source)**  
  <https://github.com/HansKristian-Work/vkd3d-proton/blob/master/libs/vkd3d/cache.c>  
  Shows how the layer versions and validates its on-disk cache, a reference for designing our own.

- **vkd3d-proton v2.6 release notes**  
  <https://github.com/HansKristian-Work/vkd3d-proton/releases/tag/v2.6>  
  Documents the descriptor-copy CPU overhead reductions, relevant to hitch analysis.

- **vkd3d-proton v2.12 release notes**  
  <https://github.com/HansKristian-Work/vkd3d-proton/releases/tag/v2.12>  
  Only added DRED settings stubs, so DRED diagnostics apply to native Windows D3D12 runs, not the Vulkan layer.

- **vkd3d-proton v3.0 release notes**  
  <https://github.com/HansKristian-Work/vkd3d-proton/releases/tag/v3.0>  
  Moved to driver-side zero-initialisation of device memory and ignores not-cleared allocation flags in all titles.

- **vkd3d-proton v3.0.1 release notes**  
  <https://github.com/HansKristian-Work/vkd3d-proton/releases/tag/v3.0.1>  
  Removed a global submission lock workaround on NVIDIA and added a fence-wait performance workaround.

- **vkd3d-proton v3.1 release notes**  
  <https://github.com/HansKristian-Work/vkd3d-proton/releases/tag/v3.1>  
  Added a workaround for broken root SRV/UAV behaviour that affects NVIDIA; a warning against binding null or garbage root descriptors.

- **vkd3d-proton changelog**  
  <https://github.com/HansKristian-Work/vkd3d-proton/blob/master/CHANGELOG.md>  
  Timeline of submit and descriptor-heap changes; handy for dating when a behaviour changed upstream.

- **vkd3d-proton pull 3074 (CPU timeline completion tracking)**  
  <https://github.com/HansKristian-Work/vkd3d-proton/pull/3074>  
  Replaces explicit-use retention with waiting for observed GPU completion, since read-only resources may lack barriers; a model for safe deferred release.

- **vkd3d-proton issue 2906 (GPU crash during heavy streaming)**  
  <https://github.com/HansKristian-Work/vkd3d-proton/issues/2906>  
  A page-fault crash during asset streaming suspected to come from freeing live resources; an example of the stale-resource class of bugs.

- **vkd3d-proton issue 2768 (layout errors found with validation)**  
  <https://github.com/HansKristian-Work/vkd3d-proton/issues/2768>  
  Shows placed resources that stay in a stale layout and the force-initial-transition workaround.

- **vkd3d-proton issue 3425 (reserved resource count at world load)**  
  <https://github.com/HansKristian-Work/vkd3d-proton/issues/3425>  
  A racing-game report of roughly eighteen thousand reserved resources created at load, illustrating the scale a streaming-heavy title can demand.

- **vkd3d-proton issue 610 (shadows vanishing on NVIDIA)**  
  <https://github.com/HansKristian-Work/vkd3d-proton/issues/610>  
  Maintainer attributes it to reads through wrong descriptors, where anything can happen; frames wrong-descriptor bugs as a root cause of flicker.

- **vkd3d-proton issue 1700 (flicker fixed by host-cached memory)**  
  <https://github.com/HansKristian-Work/vkd3d-proton/issues/1700>  
  Two switches removed the flicker, and one reporter traced it to a firmware setting; a reminder to rule out platform causes.

- **vkd3d-proton issue 2800 (blocky shadows)**  
  <https://github.com/HansKristian-Work/vkd3d-proton/issues/2800>  
  Resolved with a depth-compression disable switch; one more example of compression-related artefacts.

- **Phoronix: vkd3d-proton descriptor heaps**  
  <https://phoronix.com/news/vkd3d-proton-descriptor-heaps>  
  Reports the new descriptor-heap path merged but off by default because of remaining NVIDIA bugs.

## 3. Vulkan extensions, validation and tools

- **VK_EXT_device_fault proposal**  
  <https://docs.vulkan.org/features/latest/features/proposals/VK_EXT_device_fault.html>  
  After device loss, query fault info twice (counts then data) for address ranges and vendor data; our basis for post-mortem collection.

- **vkGetDeviceFaultInfoEXT reference**  
  <https://docs.vulkan.org/refpages/latest/refpages/source/vkGetDeviceFaultInfoEXT.html>  
  Explicit capacities and incomplete-result behaviour matter when sizing the vendor binary buffer.

- **VK_EXT_device_address_binding_report**  
  <https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_device_address_binding_report.html>  
  Reports bind and unbind of GPU address ranges, so a fault in an unbound range suggests use-after-free.

- **VK_EXT_robustness2**  
  <https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_robustness2.html>  
  Null-descriptor access is only defined with the nullDescriptor feature; a descriptor pointing at freed memory is undefined.

- **VK_EXT_descriptor_buffer proposal**  
  <https://github.khronos.org/Vulkan-Site/features/latest/features/proposals/VK_EXT_descriptor_buffer.html>  
  Describes descriptor buffers and their synchronisation rules; the basis for a descriptor path that avoids sets.

- **Vulkan spec: descriptor buffers**  
  <https://docs.vulkan.org/spec/latest/chapters/descriptorbuffers.html>  
  Requires referenced image views and samplers to outlive the last dynamic access, which constrains deferred release.

- **Vulkan-Guide: descriptor buffer**  
  <https://github.com/KhronosGroup/Vulkan-Guide/blob/main/chapters/descriptor_buffer.adoc>  
  Explains host-write visibility through submission and the barrier needed for GPU-written descriptors.

- **Vulkan spec: sparse memory**  
  <https://docs.vulkan.org/spec/latest/chapters/sparsemem.html>  
  Sparse residency lets one stream parts of an image; strict non-resident reads return zero where supported.

- **Vulkan spec: synchronization**  
  <https://docs.vulkan.org/spec/latest/chapters/synchronization.html>  
  Device loss does not prove submitted work finished, so failed-signal observations must be kept apart from successful ones.

- **vkFreeMemory reference**  
  <https://docs.vulkan.org/refpages/latest/refpages/source/vkFreeMemory.html>  
  Applications must finish GPU use before freeing memory, although implementations may keep payloads alive while bound objects survive.

- **Vulkan guide: robustness**  
  <https://docs.vulkan.org/guide/latest/robustness.html>  
  Overview of robust buffer and image access, relevant when guest shaders read out of range.

- **Vulkan guide: memory allocation**  
  <https://docs.vulkan.org/guide/latest/memory_allocation.html>  
  Practical advice on heap and memory-type choice, including host-visible device-local memory.

- **Khronos validation layer documentation**  
  <https://vulkan.lunarg.com/doc/view/latest/linux/khronos_validation_layer.html>  
  Covers GPU-assisted validation and descriptor checks, with their prerequisites and limits.

- **LunarG synchronization validation**  
  <https://vulkan.lunarg.com/doc/view/latest/windows/synchronization_usage.html>  
  Detects read/write hazards at submit; a cheap first check for missing barriers.

- **Vulkan loader: layer interface**  
  <https://github.com/KhronosGroup/Vulkan-Loader/blob/main/docs/LoaderLayerInterface.md>  
  Layer path variables are ignored for elevated processes, which silently disables private layers.

- **VK_EXT_calibrated_timestamps proposal**  
  <https://github.khronos.org/Vulkan-Site/features/latest/features/proposals/VK_EXT_calibrated_timestamps.html>  
  Needed to correlate GPU and CPU clocks when measuring waits across the two.

- **Vulkan tutorial: crash minidump**  
  <https://docs.vulkan.org/tutorial/latest/Building_a_Simple_Engine/Tooling/04_crash_minidump.html>  
  After device loss, stop submitting, flush logs and persist the dump; naming objects makes dumps readable.

- **Fossilize README**  
  <https://github.com/ValveSoftware/Fossilize/blob/master/README.md>  
  Describes recording pipeline state as hashed create-infos for replay, the model behind shared shader caches.

## 4. NVIDIA device loss and crash tooling

- **NVIDIA Nsight Aftermath SDK**  
  <https://docs.nvidia.com/nsight-aftermath/SDK/index.html>  
  Initialise before the device and enable per-device capture flags; allow dump generation to finish before exit.

- **Nsight Aftermath release notes**  
  <https://docs.nvidia.com/nsight-aftermath/ReleaseNotes/index.html>  
  Only GPU crashes are captured, not CPU crashes in the driver or loader, which sets expectations for what a dump can explain.

- **Nsight Aftermath GPU crash dump monitor**  
  <https://archive.docs.nvidia.com/nsight-graphics/2025.5/UserGuide/gpu-crash-dump-monitor.html>  
  A no-code route to crash dumps with disabled, global and whitelist modes.

- **Nsight Graphics GPU crash dump inspector**  
  <https://docs.nvidia.com/nsight-graphics/UserGuide/gpu-crash-dump-inspector.html>  
  Notes that shader-register collection is on by default in recent drivers but visualisation needs the professional edition.

- **VK_NV_device_diagnostics_config**  
  <https://docs.vulkan.org/refpages/latest/refpages/source/VK_NV_device_diagnostics_config.html>  
  Enables shader error reporting and debug info for crash dumps on NVIDIA.

- **NVIDIA: Advanced API performance, debugging**  
  <https://developer.nvidia.com/blog/advanced-api-performance-debugging/>  
  Warns that checkpoints and breadcrumbs cost real time, so keep them to roughly a hundred per frame or fewer.

- **Themaister: hardcore Vulkan debugging on amdgpu**  
  <https://themaister.net/blog/2023/08/20/hardcore-vulkan-debugging-digging-deep-on-linux-amdgpu>  
  Explains NV checkpoint markers and how bottom-of-pipe versus top-of-pipe tells which commands finished before a fault.

- **NVIDIA forum: unused compute pipeline causes device loss on RTX 40 series**  
  <https://forums.developer.nvidia.com/t/windows-vulkan-unused-compute-pipeline-causes-device-loss-on-rtx-4090-4070-super/383848>  
  A vendor-acknowledged driver bug where a pipeline with a large private array loses the device even if never used.

- **NVIDIA forum: Witcher 3 ray tracing Xid 109 fix**  
  <https://forums.developer.nvidia.com/t/solution-for-witcher-3-with-rt-under-610-drivers/378760>  
  A game that reused memory without barriers lost the device on NVIDIA; fixed by an extra-sync setting.

- **NVIDIA forum: Xid 31 MMU fault on copy engine**  
  <https://forums.developer.nvidia.com/t/615-71-09-intermittent-vk-error-device-lost-with-xid-31-mmu-fault-on-copy-engine-ce0-fault-pte-virt-read/384945>  
  An intermittent device loss involving a separate transfer queue; shows that copy-engine faults can follow driver regressions.

- **NVIDIA forum: vkd3d descriptor_heap path device lost on driver 610**  
  <https://forums.developer.nvidia.com/t/vkd3d-proton-descriptor-heap-path-causes-vk-error-device-lost-on-driver-610-in-the-first-descendant/374747>  
  The new descriptor-heap path lost the device on one driver branch but not an older one.

- **NVIDIA forum: descriptor buffers and Xid 69 on Pascal**  
  <https://forums.developer.nvidia.com/t/vk-ext-descriptor-buffer-in-vkd3d-triggers-xid-69-on-pascal-cards-since-driver-535-series/291172>  
  Bisected to the commit that enabled global descriptor buffers; evidence of driver-specific descriptor-buffer faults.

- **NVIDIA forum: Xid 13/109 with VKD3D on 525 drivers**  
  <https://forums.developer.nvidia.com/t/multiple-cuda-rtx-vulkan-application-crashing-with-xid-13-109-errors/235459>  
  Fixed by downgrading the driver and acknowledged as an NVIDIA bug; a reminder to bisect driver versions.

- **DXVK issue 1791 (device lost on a large host-visible heap)**  
  <https://github.com/doitsujin/dxvk/issues/1791>  
  DXVK limits use of the device-local host-visible heap on NVIDIA as a workaround for a driver bug.

- **NVIDIA: Advanced API performance, memory and resources**  
  <https://developer.nvidia.com/blog/advanced-api-performance-memory-and-resources/>  
  Guidance on allocation and residency behaviour that affects hitching and loss.

- **NVIDIA: video memory overcommitment detection with GPUView**  
  <https://developer.nvidia.com/content/are-you-running-out-video-memory-detecting-video-memory-overcommitment-using-gpuview>  
  A method for confirming whether late textures come from video-memory overcommit.

- **NVIDIA: Vulkan memory management**  
  <https://developer.nvidia.com/vulkan-memory-management>  
  Recommendations on suballocation and host-visible device-local usage on NVIDIA hardware.

- **NVIDIA: using Nsight Systems to fix stutters**  
  <https://developer.nvidia.com/blog/using-nsight-systems-for-fixing-stutters-in-games>  
  A walkthrough for finding CPU and GPU causes of frame hitches.

## 5. AMD, RADV and Radeon tools

- **Mesa: RADV hang debugging**  
  <https://docs.mesa3d.org/drivers/amd/hang-debugging.html>  
  RADV_DEBUG=hang writes fault logs, buffer history and pipeline traces, which separates never-valid from use-after-free addresses.

- **Mesa: RADV driver options**  
  <https://docs.mesa3d.org/drivers/radv.html>  
  Lists driver options such as unified heap on integrated GPUs and the transfer queue, both off by default.

- **Mesa environment variables**  
  <https://docs.mesa3d.org/envvars.html>  
  Reference for debug and performance switches used while comparing RADV behaviour.

- **GPUOpen: Radeon GPU Profiler 2.6 update**  
  <https://gpuopen.com/learn/radeon-developer-tool-suite-update-rgp-2-6/>  
  Background on RGP capture and queue-timing views for AMD hardware.

- **GPUOpen: D3D12 GPU upload heap**  
  <https://gpuopen.com/learn/using-d3d12-heap-type-gpu-upload/>  
  Describes the GPU-upload heap that lets the CPU write straight to video memory.

- **GPUOpen: Vulkan device memory**  
  <https://gpuopen.com/learn/vulkan-device-memory/>  
  Lists memory-type combinations per vendor and cautions about host-visible device-local costs.

- **GPUOpen: D3D12 and Vulkan lessons learned (GDC 2017)**  
  <https://gpuopen.com/download/GDC2017-D3D12-And-Vulkan-Lessons-Learned.pdf>  
  Practical API-use advice from a vendor, including barrier and queue guidance.

- **GPUOpen: right on queue (GDC 2016)**  
  <https://gpuopen.com/download/GDC_2016_D3D12_Right_On_Queue_final.pdf>  
  Recommends the copy queue for uploads spread across frames and warns about waiting on it from graphics.

- **D3D12 Memory Allocator documentation**  
  <https://gpuopen-librariesandsdks.github.io/D3D12MemoryAllocator/html/index.html>  
  Reference allocator design; its quick-start says nothing about GPU use after release, so lifetime is the caller's job.

- **Vulkan Memory Allocator: staying within budget**  
  <https://gpuopen-librariesandsdks.github.io/VulkanMemoryAllocator/html/staying_within_budget.html>  
  Explains budget queries and how to avoid over-committing video memory.

## 6. Wine and Proton behaviour

- **Wine merge request 9090 (write-watch handling)**  
  <https://gitlab.winehq.org/wine/wine/-/merge_requests/9090>  
  Documents MEM_WRITE_WATCH semantics and the reset race, relevant to dirty-page tracking.

- **Wine vkd3d merge request 933 (tile mappings)**  
  <https://gitlab.winehq.org/wine/vkd3d/-/merge_requests/933>  
  Shows UpdateTileMappings support for textures and buffers derived from the proton variant.

- **Wine vkd3d merge request 216 (tile mappings series)**  
  <https://gitlab.winehq.org/wine/vkd3d/-/merge_requests/216>  
  A series with open sequencing questions about when mapping changes take effect.

- **Linux kernel: ntsync driver**  
  <https://docs.kernel.org/userspace-api/ntsync.html>  
  Documents the kernel primitive that lets Wine emulate Windows synchronisation objects faster.

- **Phoronix: Linux 6.14 NTSYNC driver**  
  <https://www.phoronix.com/news/Linux-6.14-NTSYNC-Driver-Ready>  
  Reports when the ntsync driver landed in the kernel.

- **Wine bugs list: ntsync thread**  
  <https://list.winehq.org/hyperkitty/list/wine-bugs@list.winehq.org/thread/3SPFLSRTLQD7HTUU7OOTHOE3MAXZL5W6/>  
  A bug thread on the Windows-versus-Proton synchronisation path and related Proton switches.

- **Wine mailing list: dirty descriptor handling in vkd3d**  
  <https://list.winehq.org/hyperkitty/list/wine-gitlab@list.winehq.org/thread/IOBMBB3STETKB56V6KOQMSWRXHCSDDCB/>  
  Describes how a failed compare-and-swap on a dirty list leaves stale Vulkan descriptors and corruption without device loss.

- **Kernel amd-pstate documentation**  
  <https://docs.kernel.org/admin-guide/pm/amd-pstate.html>  
  Preferred-core ranking and energy-preference modes that affect frame pacing on AMD handhelds.

## 7. Texture streaming and residency

- **Tiled resources: operations on streaming resources**  
  <https://learn.microsoft.com/en-us/windows/uwp/graphics-concepts/operations-available-on-streaming-resources>  
  Reads from unmapped tiles return zero, and mapping changes affect only later submitted work.

- **SRV behaviour with non-mapped tiles**  
  <https://learn.microsoft.com/en-us/windows/win32/direct3d11/srv-behavior-with-non-mapped-tiles>  
  Some first-generation Tier 2 hardware zeroes the entire filter if any weighted texel is unmapped.

- **Tiled resources: texture sampling features**  
  <https://learn.microsoft.com/en-us/windows/win32/direct3d11/tiled-resources-texture-sampling-features>  
  Details of residency-clamp and status feedback in sampling, used to design late-texture fallbacks.

- **Sampler feedback specification**  
  <https://microsoft.github.io/DirectX-Specs/d3d/SamplerFeedback.html>  
  Applications update the minimum-mip map when mapping tiles, with no ordering rule between the two; a gap to design around.

- **DirectStorage 1.1 announcement**  
  <https://devblogs.microsoft.com/directx/directstorage-1-1-now-available/>  
  Overview of GPU decompression and queued reads, a possible model for streaming performance.

- **Vulkan tutorial: transfer queues and streaming**  
  <https://docs.vulkan.org/tutorial/latest/Synchronization/Transfer_Queues_Streaming/01_introduction.html>  
  A walkthrough of streaming through a dedicated transfer queue with cross-queue synchronisation.

- **Unreal Engine: texture streaming configuration**  
  <https://dev.epicgames.com/documentation/en-us/unreal-engine/texture-streaming-configuration-in-unreal-engine>  
  Shows pool sizing and mip bias behaviour; the low mips stay resident as placeholders.

- **Unreal forum: AsyncCompute texture pool copy hang**  
  <https://forums.unrealengine.com/t/gpu-hang-on-asynccompute-texturepoolcopyops/2743119>  
  A real-engine GPU hang traced to missing synchronisation between async compute and copies.

- **Dolphin commit on hi-res texture fallback**  
  <https://github.com/dolphin-emu/dolphin/commit/d03e09c8fd911d1b4e66184ceedbeca989b75a0c>  
  Illustrates showing a lower-detail texture rather than a blank one while a replacement loads.

- **Dolphin: texture cache source**  
  <https://raw.githubusercontent.com/dolphin-emu/dolphin/master/Source/Core/VideoCommon/TextureCacheBase.cpp>  
  Reference for how a mature emulator tracks texture lifetime and invalidation.

- **Game Developer: managing D3D12 resource lifetimes**  
  <https://gamedeveloper.com/programming/managing-d3d12-resource-lifetimes>  
  Describes a release queue tagged with command-list numbers and purged each frame.

- **GPU memory pools (MJP)**  
  <https://therealmjp.github.io/posts/gpu-memory-pool>  
  Pool allocation trade-offs for GPU memory, relevant to placed-resource designs.

## 8. Shader and pipeline caches, load times

- **Unreal Engine: PSO precaching**  
  <https://dev.epicgames.com/documentation/en-us/unreal-engine/pso-precaching-for-unreal-engine>  
  Describes compiling pipelines ahead of need to avoid first-use hitches.

- **Unreal 4.26: PSO caching reference**  
  <https://docs.unrealengine.com/4.26/en-US/SharingAndReleasing/PSOCaching/PSOReference>  
  Batch modes (fast behind loading screens, background in menus) and pausing batches during play.

- **Unreal forum: PSO precache and loading screens**  
  <https://forums.unrealengine.com/t/pso-precache-loading-screen/2658405>  
  The remaining-count may never reach zero reliably, so holding a loading screen on it is hard.

- **Unreal forum: PSO precache hitches**  
  <https://forums.unrealengine.com/t/pso-precaching-hitches/2674115>  
  Reports of hitches that remain despite precaching.

- **Dolphin ubershaders**  
  <https://dolphin-emu.org/blog/2017/07/30/ubershaders/>  
  The classic write-up of avoiding compile stutter with generic shaders until specialised ones are ready. (not auto-verified: site blocks scripts)

- **Dolphin stutter fix (press summary)**  
  <https://www.pcgamer.com/the-dolphin-emulator-developers-spent-two-years-fixing-a-nearly-impossible-to-solve-stuttering-problem>  
  A press summary of the long effort to remove shader compile stutter.

- **Valve improves Linux Vulkan shader pre-caching**  
  <https://gamingonlinux.com/2020/05/valve-improve-linux-vulkan-shader-pre-caching>  
  Describes shared pre-compiled pipeline caches distributed with the platform.

- **Steam beta: Vulkan shader processing**  
  <https://gamingonlinux.com/2020/05/steam-beta-adds-vulkan-shader-processing>  
  Background processing defaults to a fraction of cores at idle priority.

- **Mesa 23.1: RADV graphics pipeline library enabled**  
  <https://gamingonlinux.com/2023/05/mesa-graphics-drivers-23-1-0-out-now-with-radv-gpl-enabled>  
  Pipeline libraries cut link-time hitches on AMD.

- **Fossilize issue 312 (replay memory use)**  
  <https://github.com/ValveSoftware/Fossilize/issues/312>  
  Parallel replay workers used many gigabytes, so cache pre-building must bound its worker count.

- **Fossilize issue 288 (memory accounting)**  
  <https://github.com/ValveSoftware/Fossilize/issues/288>  
  A contributor notes summed resident size overstates use; measure proportional size instead.

- **DXVK release v0.93 notes**  
  <https://newreleases.io/project/github/doitsujin/dxvk/release/v0.93>  
  Introduced a compiler-thread setting, showing early recognition of shader compile threads.

- **NVIDIA forum: compute pipeline assertion under Proton**  
  <https://forums.developer.nvidia.com/t/assertion-failed-expression-status-vkcreatecomputepipelines-with-ue5-games-with-wine-proton/293470>  
  A report of a pipeline-creation assertion coinciding with cache-file writes; unresolved.

## 9. Frame pacing and hitches

- **Unreal forum: multi-second freeze in pool allocator**  
  <https://forums.unrealengine.com/t/multi-second-freeze-in-fd3d12poolallocator-createnewpool-createcommittedresource-after-upgrading-to-5-5-4/2670404/29>  
  A committed-resource creation stall inside a pool allocator; evidence that allocation can stall the frame.

- **Unreal forum: deleted-resource gathering on the render thread**  
  <https://forums.unrealengine.com/t/render-thread-gathering-deleted-resources/2718044>  
  Shows an engine deletion queue and its render-thread cost, relevant to deferred-release overhead.

- **Unity: asyncUploadTimeSlice**  
  <https://docs.unity3d.com/ja//ScriptReference/QualitySettings-asyncUploadTimeSlice.html>  
  Time-slices uploads on the render thread per frame to bound hitches.

- **Unity: asyncUploadBufferSize**  
  <https://docs.unity3d.com/530/Documentation/ScriptReference/QualitySettings-asyncUploadBufferSize.html>  
  The upload ring buffer size bounds how much streaming can happen per frame.

- **DXVK developer guidelines**  
  <https://github.com/doitsujin/dxvk/wiki/Developer-guidelines>  
  Guidance on staying inside the video-memory budget and keeping hot paths cheap.

- **DXVK release 1.2 (separate submission thread)**  
  <https://github.com/doitsujin/dxvk/releases/tag/v1.2>  
  Moving submission to its own thread is a standard remedy for render-thread stalls.

- **Phoronix: amd-pstate and better 1% lows**  
  <https://www.phoronix.com/news/AMD-P-State-Better-1p-Lows>  
  Scheduler and power-governor work aimed at frame-time lows.

- **Phoronix: cache-aware scheduling regression on a Zen 5 laptop chip**  
  <https://phoronix.com/news/Linux-CAS-Regression-AMD-HX-370>  
  A task could stay on slow cores while faster ones were idle; a caution for hybrid-core handhelds.

- **NVIDIA: Advanced API performance, barriers**  
  <https://developer.nvidia.com/blog/advanced-api-performance-barriers>  
  Barrier batching advice that reduces GPU bubbles.

## 10. Alignment, address space and memory

- **VirtualAlloc**  
  <https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc>  
  Reference for reserving and committing address space, including write-watch reservation flags.

- **GetWriteWatch**  
  <https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-getwritewatch>  
  Returns pages written since the last reset; a cheap dirty-page tracker, with a reset race to handle.

- **VirtualQuery**  
  <https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualquery>  
  Scans pages until attributes change; cost grows with fragmented address space.

- **The Old New Thing: a fixed address the system never uses**  
  <https://devblogs.microsoft.com/oldnewthing/20221222-00/?p=107609>  
  Explains that no address is guaranteed free, which matters for titles that expect fixed virtual addresses.

- **The Old New Thing: page protection costs**  
  <https://devblogs.microsoft.com/oldnewthing/20180124-00/?p=97875>  
  Page protection is free until touched, then costs one exception per first write and has races on multi-threaded faults.

- **NtQueryVirtualMemory documentation**  
  <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntqueryvirtualmemory>  
  Only basic memory information is publicly supported, limiting address-space inspection tricks.

- **Working set (Windows memory management)**  
  <https://learn.microsoft.com/windows/win32/memory/working-set>  
  Explains how trimmed pages become standby and why re-referencing them counts as soft faults.

- **Windows x64 exception handling**  
  <https://learn.microsoft.com/en-us/cpp/build/exception-handling-x64>  
  Handlers are found only at dispatch by table lookup, so protected regions add no cost on the normal path.

- **Windows TDR registry keys**  
  <https://learn.microsoft.com/en-us/windows-hardware/drivers/display/tdr-registry-keys>  
  Documents timeout-detection settings relevant to long GPU work and apparent device loss.

- **Intel Xe Vulkan wave size report**  
  <https://community.intel.com/t5/Graphics/Intel-R-Iris-R-Xe-Graphics-RPL-P-Vulkan-reports-incorrect/m-p/1681831>  
  Shows wave size differs by vendor, so subgroup-width assumptions in translated shaders can be wrong.

- **HLSL wave size specification**  
  <https://microsoft.github.io/DirectX-Specs/d3d/HLSL_SM_6_6_WaveSize.html>  
  Defines shader-model 6.6 wave-size control, needed to match console wave64 behaviour.

- **Xenia release notes: gamma render targets**  
  <https://newreleases.io/project/github/xenia-project/release-builds-windows/release/v1.0.2839-master>  
  A precedent for handling console gamma render targets with a linear format.

- **Themaister: translating DXIL to SPIR-V, part 3**  
  <https://themaister.net/blog/2021/11/07/my-personal-hell-of-translating-dxil-to-spir-v-part-3/>  
  The author of the shader translator explains descriptor palettes, view ownership and copy lifetimes.

---
[Back to the index](../README.md)
