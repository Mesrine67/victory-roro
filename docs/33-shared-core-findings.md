# Lessons from several titles on one shared core

This document collects what we learned while running several console-era D3D12.X and D3D11.X titles through one shared translation layer. The layer targets desktop D3D12 on Windows and vkd3d-proton on Linux. We call it "the core". It is aimed at an engineer building a similar translator for Xbox Series X titles.

Most findings concern memory, GPU data and measurement. The same mistakes repeated across titles. Many of them came from reading our own numbers wrongly, not from the title or the driver. So each section gives what we found, the numbers, and the rule we now follow.

A note on evidence. Several fixes below are "default off". That means a measurement showed promise but we have not proven it for every title. We say so where it matters. We also list the dead ends, because they saved us the most time once written down.

Test hardware, in generic terms: an RTX 4090 desktop, an RTX 3080 laptop, an AMD desktop on Windows, an AMD desktop on Linux/Proton, a 5900X desktop and an AMD handheld under Proton. Numbers from different machines are never mixed in one comparison unless we say so.

---

## 1. Console DMA and move engines

### What the console does

The console has DMA (move) engines. A title records copy, fill and decompress work for them. The engines can inflate zlib streams and re-tile data on the way into GPU memory. The title treats this as free work. A desktop has no equivalent. Your translator must emulate the engine, on the CPU or on the GPU.

### Stubs that report success

The first lesson is about stubs. Our core had a DMA slot for "decompress zlib into this destination". It was a stub that returned success and wrote nothing. In one title the destination stayed zero. That emptied a visibility table and later broke model containers. The title never checked the output, so the symptom appeared far from the cause: a parser fault later, and wrong visibility.

The captured evidence was clean. The decoder reported success for 1,516 compressed bytes with a valid zlib header, while all 64 captured destination bytes stayed identical before and after. We wrote a generic CPU implementation. It runs pure-decompress batches at queue submission, after a queue-specific fence. It validates the zlib framing and checksum before it copies the complete output. It rejects mixed batches that combine decompress with other native work.

Rules:
- Never leave a DMA, device or fence slot as "return zero". Return a real error, or implement it.
- Log every stubbed slot call with a counter. Grep the counter after every run.
- A stub returning success is worse than one returning failure, because the title trusts it.

A related stub was a "create cached pipeline state" slot. It returned success with a null output. The title cached the null. Its bind helper then skipped the native setter, so the previously bound compute pipeline stayed active at the next graphics draw. An AMD driver faulted on it. Do not read this as proof that the console draws with a compute pipeline. It was a null that nobody checked.

Another title used a different decompress ABI. Its slot did work, with 8,198 inflates all ending in a clean stream end. So "this title needs DMA decompress" is not a universal fact. Check which ABI the title uses before you copy a fix between titles.

### Never return "not implemented" from a wait slot

On the DXVK path, a DMA fence-wait slot returned E_NOTIMPL more than 32 times in one capture. The title then reads DMA output before the copy engine finishes. We now flush and wait in that slot. Rule: never return NOTIMPL from any wait or fence slot. A wrong wait is a data race. A refused wait is just a slower frame.

### CPU DMA on GPU-owned data

On the DXVK path, GPU results never reach title memory. The DMA engine ops (copy, fill, inflate) ran as plain CPU operations on title memory. We suspected two bugs:
1. A DMA copy whose source is a GPU-rendered surface reads stale bytes.
2. A UAV bind permanently flips a CPU-tiled texture to GPU authority, so later CPU or DMA streaming never uploads.

Instrumentation ruled both out. Only 6 of 5,299 placement textures were GPU-authoritative at creation. There were no CPU-to-GPU flips and no DMA reads from GPU-owned data. But the log showed something else: DMA fills landing on GPU-owned textures every frame. The value was all ones, over two large depth-like textures created at load. That was 3,666 fills and 11 GB in about ten minutes. On the DXVK path the fill reached title memory only and never reset the GPU texture. The likely purpose was a depth or metadata reset.

Rules:
- Decide data authority per resource: CPU copy or GPU copy. Make the decision at creation, and log every flip.
- A DMA fill on a render target or depth target should become a GPU clear.
- Count the operations before you theorise. Two of our three theories died in one log.

### Queue dependencies and retirement

A CPU DMA path has to respect queue order. A title fence can be shared with work outside the copy engine. If the CPU signals it early, the title sees unfinished work as finished. Our design review reached three rules:
- Own the submission order of the emulated engine.
- Honour every wait the title recorded.
- Route unsupported batches to a safe path.

Reuse of DMA allocators needs queue-owned retirement. A completed title fence value alone does not prove that a given copy submission retired. Cross-queue signals, CPU signals, rewound fence values and still-open lists can all break the shortcut. We shelved two pooling prototypes for this reason.

### Pooling and batching pitfalls

Two test mistakes cost us time.

First, our pooling test missed the real boundary. A wrapper allocator reached the native DMA Reset because its dispatch table lacked the translating override. The wrapper pointer went to native code unwrapped. The fixture called the wrapper's own function and passed. Only a native reproduction distinguished wrapped from unwrapped arguments. Rule: exercise the real interface dispatch in fixtures, not the internal helper.

Second, a batching prototype was meant to coalesce submissions. In practice nearly every submitted list was an opaque split, and each flushed. Extra private retirement signals even raised the number of real queue calls. Rule: measure actual submissions and signals per frame before and after any batching change. Intent proves nothing.

### Primary frame admission cannot bound other queues

We tried limiting overlap between frames to bound GPU memory. The compositor already had a three-frame fence reuse limit. Cutting it to one was an experiment, and it does not bound independent title queues or driver scratch memory. Adding a blocking CPU wait to every execute call is dangerous. It can block a producer Signal that another wait depends on, and the process deadlocks.

---

## 2. CPU-visible textures and desktop restrictions

The console lets a title create textures in CPU-visible memory. The title maps them, writes texels and unmaps. Desktop D3D12 and vkd3d reject many of these with E_INVALIDARG. One title's first rejected case was a 64x64x64 BC4 volume texture on an upload heap. Its fatal handler then ended the run.

Our fix retries a rejected allocation in two parts:
- The native resource becomes DEFAULT storage in the native format.
- A guarded CPU shadow takes its place for Map.

Unmap packs the copy footprints and uploads them through one persistent, fenced upload context. A successful native allocation keeps its original path untouched. The fix covers single-plane, single-sample, non-attachment textures, with mip chains and arrays. Tests completed 8 BC4 64-cubes and 32 RGBA8 16-cubes in a real run.

What it does not cover: concurrent re-mapping after GPU writes, and placed-heap aliasing. The upload context pins a bounded set of native objects until exit.

Gotchas we met while writing fixtures:
- The native D3D12 core crashed walking deferred-destruction callbacks with a null device. Keep lifetime holds on the objects.
- A reference count of 2 right after Release is backend retirement through a 32-slot deferred ring. It is not a leak. Do not chase it.

Rules:
- Treat any E_INVALIDARG from CreateCommittedResource or CreatePlacedResource on a CPU-visible 2D or 3D texture as this class.
- A texture on a readback heap or a CPU-mapped placed resource needs a shadow plus an explicit sync point.
- Changing depth or stencil flags to make a copy succeed changes the data layout. We saw this with a depth copy. Removing the depth flags let the copy succeed, but with a different layout and 64 KiB less overhead. It was not the same texture. Reject that shortcut.

### Console allocation answers are a separate contract

One title sized its own arenas from the console's allocation-info answers. When our desktop mirror padded resources for desktop alignment, the title ran out of its own arena while VRAM was plentiful. Keep two contracts:
1. What the title is told, which follows console backing semantics.
2. What the desktop allocation actually needs.

Preserve parameter versions, hidden-return ABIs and overflow checks in the first. Do not let desktop padding leak into the answers.

### Related cases: unaligned constant buffers

Desktop D3D12 needs 256-byte aligned constant buffer views. The console does not. One title's translator adds a packed four-bit residue in a reserved constant register. Shaders add it to their reads. The loader binds a 256-aligned GPU base. When alignment or bounds block a direct view, it copies the logical range to an aligned slice. Eight embedded shaders already used the reserved register. The cleaner logs this, disables the rewrite for that shader and keeps the arithmetic lowering. Check for a collision before reserving any register.

---

## 3. Texture hashing and detiling

### Layout is usually not the problem

Offline, we checked 137,006 texture entries from ordinary media archives. The console's tiling library, called by the title, computed exactly the payload size for each one. Seventeen real textures with 247 subresources (BC1, BC3, BC5, BC7, an array and BC6H cubemaps) retiled and detiled byte for byte. A second corpus held 178,439 texture records and 28 GiB of payload. All 174,173 mip pieces agreed in layout across 332 descriptor combinations. Twenty samples across all nine format codes passed.

An older version of the tiling library gave identical bytes and mip offsets. So we did not need version handling for the library. Every "wrong texels" theory based on layout died in one offline test. The open worry was residency and upload scheduling.

Rules:
- Prove layout with a retile-and-detile cycle before blaming it.
- A detile must follow the library's per-subresource offsets. Packed mip tails are the place where a hand-written detile goes wrong. Distant mips show it first.

### Hashing whole textures is the cost

In one title a CPU sample of the render thread showed about 70% in a full-texture hash loop. The loop detected changes to copy-on-write textures. A later perf run measured 740 MB hashed per completed present, and 36 ms for full-source texture hashes in a roughly 190 ms present.

Alternatives we measured:
- A four-lane hash with bounded cached reads gave 3-4x. Cached reads ran at 9,020 to 25,230 MiB/s. Write-combined reads ran at 88 to 337 MiB/s. Fingerprints were identical.
- Exact-compare snapshots keep the last uploaded image and compare bytes, stopping at the first difference. Static texture checks dropped from 1.62 ms to 0.42 ms with zero mismatches in 264 checks. The price is private memory for the retained images. An optional page mode compares only pages whose write-watch generation changed.

A different approach to change detection was worse. Write-watch holding a mapping lock across GPU submits caused severe stalls in an early version. In a strict A/B/A/B on a parked drive, write-watch alone cost 23.9% in presents per second. All switches on together cost 40.1%.

Rules:
- A cheap feature alone can be expensive in a stack. Test each switch alone and all together.
- Never hold a lock that guards a mapping while you wait for the GPU.
- Detect writes with a retained exact copy or hardware write-watch. Do not hash whole textures each refresh.
- Hash the meaningful bytes only. Honour placed offsets, row pitch, rows and depth slices. Padding can hold noise that hides zero texels (see section 6).

### Sparse signatures are lower bounds

One signature sampled only the start of each page. A low changed count from it is a lower bound, because writes outside the samples are invisible. We do not report such counts as the total rewrite rate. A tiny-texture-only repair also missed a larger format class (BC6H) that the logs showed changing after upload. Pick the scope from observed resource sizes, not from the smallest class that fits your theory.

---

## 4. Uploader caps: latency versus throughput

### The cap that looked like a bug

Our texture uploader let the GPU sample zero-filled textures first and filled them later from a queue. Hard-coded caps limited it:
- 4 job submits per 16.7 ms window, which is 240 per second.
- 1 MiB of CPU copy per 1,000 us window.
- 24 jobs and 128 MiB in flight.

On the handheld, the whole first lap sat at 237-240 uploads per second with 24 of 24 jobs live. About 93% of waiting visits were readable title data we had not yet copied. Much of the visible black geometry was our own lag.

With the caps lifted to 64 submits, 64 MiB, 8 ms per window, 256 jobs and 1 GiB, a backlog of about 15,000 textures cleared in about 44 s at roughly 500 per second. At 240 per second it had taken about 118 s. On an AMD Windows desktop the 16,600-texture backlog cleared in about 63 s. Frame pacing was no worse. One run had 349 hitches over 50 ms in 480 s against 734 in 900 s for the capped build. The lows were about the same.

Current defaults remove the windows and allow 1,024 jobs and 2 GiB in flight. The windows are env-scaled, and zero means unlimited.

Rules:
- A fixed per-frame cap turns a backlog into visible latency. Size the cap from the measured backlog, not from fear of a hitch.
- Throughput (textures per second) and latency (demand to landed) are different goals. A change can help one and hurt the other.
- Log which limit is hit, not just the setting.

### Faster service moved the cost elsewhere

A readability scan fix (see section 5) made visits nearly free. Texture visits jumped from about 0.1 million to 3-3.6 million per run. Frame rate was mixed, because the service now did far more work per frame. Median new-texture latency rose from about 15 ms to about 600 ms, because more textures queued at once. Yet the average frame rate rose 27-30% on the desktop, and the slow tail dropped from about 72 s to about 10 s at the 95th percentile. We read both numbers together: the median alone would have called the change a regression.

### A priority lane that admits everything is not a priority lane

Our first "priority" lane admitted every registered texture and hinted it on its first view. Ordering then did nothing. A priority queue needs genuine eligibility rules and a ready queue. The review proposed them. We never shipped the first version as a fix.

### Related scheduler results

- A change scheduler that visited only changed textures cut expensive visits from 237,000 to 7,700 per run and landed new textures in 5-14 ms instead of seconds. It crashed in 3 of 3 early handheld sessions, in title code at several addresses. A fix for a stale same-generation finish was real but did not cure the crash. The fault was in releasing a job-owned upload buffer, and a rebuild of 1,548 upload lifecycles found no double release, so premature destruction was not excluded. The scheduler stays off.
- Cancelling queued texture jobs that became obsolete before snapshot allocation is cheap and safe. It passed charged worker tests, but no runtime benefit is proven. It is also off by default.
- A settle-and-revisit wake for textures in a "seen, not yet bound" state helps one title on Vulkan only. We keep it off elsewhere until measured.

### Staging and copy queues: the order of work

Review advice that we adopted:
1. Reuse staging buffers, using fence-based reclaim.
2. Service prepared jobs independently of Present.
3. Only then add a dedicated copy queue with explicit ownership transfer.

A queue change alone does not fix sampling. Source readiness, upload completion and descriptor publication each need their own tracking. Upload draining may also run before primary direct submits, so "N jobs per Present" is not the whole limit. A DMA helper that waits for the GPU inside its processing path is not a drop-in async scheduler.

In another case, a full-texture refresh created and released a full-chain staging texture each time. We tried a pooled 32 MiB ring. Staging creations fell 75% but private-byte growth did not change. Staging was not the memory cause. Pool anyway, but do not expect it to fix a leak you have not located.

---

## 5. Write-combined memory, write-watch and VirtualQuery

### Write-combined reads are slow, and slower on Windows

Console graphics memory is write-combined (WC) on every machine. Plain CPU reads of WC memory are uncached and very slow. Measured cost per MiB when a texture snapshot copies out of title memory:
- Windows AMD desktop: 3.84 ms per MiB (median), and 3.52 on another run.
- 5900X desktop: 3.19 ms per MiB.
- AMD handheld under Proton: 0.77-0.98 ms per MiB.

With a 1,000 us copy budget per 16.7 ms window, the Windows nodes sat pinned at 16-20 MiB/s all race. The unlit geometry never cleared on them.

Fixes, by effect:
- Streaming loads (MOVNTDQA, needs SSE4.1) are the designed way to read WC memory. On the 5900X the copy cost fell from 3.19 to 0.37 ms per MiB and average fps rose from 58 to 86. Alone it did not clear the missing geometry.
- Asking for cached memory instead of WC changed a scan from 52 MB/s to about 4,800 MB/s on one node. That gave no win in fps or hitches on earlier Windows runs.
- In a different title, a worker thread copied exactly 254,016 bytes per call out of a buffer we had mapped from a WC upload heap. About 2.85 ms per copy. A guarded streaming-load copy cut that to about 0.23 ms, a 12.6x gain. On an RTX 4090 at 1080p uncapped, average fps went from 159 and 144 to 285 and 289, and the 99th-percentile frame time from 9.9 and 13.9 ms to 5.9 and 5.3 ms. A later cohort with a review-fixed patch showed +59% average and +37% on the 1% low. On the handheld the title was capped at 60 fps, and the gain was within noise.

### A diagnostic byte scan can be the bottleneck

A diagnostic zero scan read WC upload memory byte by byte. Texture preparation ran at about 25 MB/s per worker. With the scan off it ran at about 890 MB/s. Rules:
- Keep diagnostic scans of GPU-visible memory off in production.
- Never read WC memory byte by byte. Use bounded, cached, or streaming copies.
- Fingerprints are cheaper than scans.

### VirtualQuery is not free

A readability scan answered "can I read this title page?" by calling VirtualQuery up to 64 times, or 200 us, per texture visit. A profile of one title's main thread showed 30% of samples in the kernel's virtual-memory query. Measured cost: about 250 us per query, 148,000 scan steps per run and 145,700 deferred on the budget. That was 69 s of scan time in about 230 s of process time.

We first blamed churn. Every title heap allocation and free reset every texture's cached scan, with about 10,000 resets per run. Two fixes failed on measurement:
- Skipping the reset for small heap paths gave no effect.
- Range-scoped invalidation retained nothing. It was rejected.

The cost was the query itself, probably a huge committed region that makes the kernel's same-attribute scan run long. The cure was to answer readability from the loader's own map of title memory. Scan time per run fell from 69 s to 0.12 s, and 0 deferred steps remained.

The first A/B read "fps got worse" because our script logged only the last of three tries per arm. All tries together showed 107.6 mean for map-off and 106.9 for map-on: no difference. We withdrew the claim. The honest statement was: the map removes the scan cost, and its effect on fps was noise-dominated until a better design ran.

Rules:
- Treat VirtualQuery as a syscall with a heavy tail. Keep your own region map with a generation counter.
- Step by region size, not page by page.
- Record all tries, not the last one.
- A fix that makes one stage nearly free can expose a second stage that was masked by the first.

A related fault: a probe for an uncommitted mip. The detector swept from the start of the allocation and treated a fault differently from an unknown or zero result. Some tails never reached the publication path. Restrict probes to readable spans, and allow safe tail fall-through on unknown results.

### A title-side lock and a sleep resolution trap

In one title's main thread profile, about 37% of "busy" time was the title waiting on a job group. Counting 150 million calls to a yield function showed 99.99% under 100 us. Only 81 sleeps of 1 ms or more occurred, and all lasted at least 16 ms. The default 15.6 ms Windows timer resolution turned short sleeps into full ticks. Raising the timer resolution gave no measurable benefit, so we kept it off. The profile still taught us to count waits before assuming a thread is working.

---

## 6. Readbacks and the evidence they do not give

This theme caused more wasted test cycles than any other. A readback that looks right can mislead. The following list is ours, with the cases that taught each point.

1. **A populated readback does not prove the resource produced the pixels you are chasing.** We read back a road-surface texture correctly, and the road was still dark. Pixel ownership was open. The readback had been tied to an early parent draw, not the draw that colored the region. Join the actual draw, bindings and pixel ownership before ruling out a sampled input. Prove who owns the pixels before you analyse shaders.
2. **Committed memory does not prove the title finished writing it.** A proposed full-upload shortcut skipped zero-content verification. Readable pages can still hold zeros, and a terminal "resident" state could block later correction. Keep commit checks and content checks separate.
3. **Zero padding and nonzero padding both deceive.** An early audit called a buffer populated because its padding held nonzero noise. The meaningful R8 texels were all zero. A fenced read later proved the generation succeeded before an overwrite. Every population statistic must honour offsets, pitch, rows and slices.
4. **GPU-produced buffers can be populated while the title's CPU copy is zero.** Native transform, vertex, argument and lighting data disagreed with all-zero CPU shadows. A blanket CPU refresh would destroy valid GPU data. Classify the producer first (section 9).
5. **A fenced pre-list snapshot proves the copied bytes, not their identity at a later draw.** A debug fence taken before the list does not show what writes happen inside the target list. Establish source state, every producer queue, and every intervening copy, UAV or alias write.
6. **"No probe result" is not "zero content".** A probe silent for a texture may mean no eligible upload, protected ownership, a failed capture setup, or a size/format/mip gate that excluded it. Require a successful capture, export or readback receipt before interpreting contents. Descriptor metadata, zero placeholders and capped mismatch logs cannot establish complete content either.
7. **A cached zero verdict must not outlive its payload.** A cached "lower mips are zero" flag does not prove a new capture is zero. Later nonzero upper-mip or last-row bytes must not inherit the old verdict. Scan the whole meaningful prepared payload, exclude padding, and bind the proof to a generation plus snapshot version. Charge scratch memory at admission. Incomplete proofs carry no authority to suppress an upload.
8. **A sparse late-write signature is a lower bound** (section 3).
9. **Recording counts are not execution counts.** A long-lived bundle can run many times after one recording. Missing direct draw records do not show an omitted draw. Log actual executions and draw ownership.
10. **An interval is not a queue depth.** We read "graphics queue lag" off the time upload jobs waited for admission. That was CPU admission latency, not submitted GPU backlog. Name both endpoints of every interval, and separate CPU admission, submission and GPU completion.
11. **A decompress shader that depends on console metadata may zero your data.** One title's stencil-decompress compute shader reads console compression metadata. On desktop the stencil is already uncompressed and the metadata does not exist, so the shader wrote zero motion IDs over the whole frame. Every object then got near-limit velocity, and the world composite smeared. GPU readbacks showed stencil 0 over 2,072,373 pixels while depth was nonzero. We replaced that exact shader with one that copies native sample-0 stencil. Any title shipping console depth or stencil decompress shaders may lose stencil IDs the same way.
12. **A bad shader blob is a bug in the handover, not the translator.** One cache wrote 1,178 of 3,666 kept records as success when the input was not a shader at all. A rewrite function returned 0 for both "nothing to rewrite" and "not a shader". Of 703 unique captured failure blobs, 672 lacked a bytecode header. Make the return codes distinct, and validate at the driver call.
13. **A device-capability probe is not a feature test.** We found that a folder holding a core DLL did not prove which backend was selected. Log the module that owns the real device. Query the adapter for resource-heap, binding, tiled and typed-UAV tiers, and do not infer them.

---

## 7. Descriptors and bundles

### Bundle inheritance

A title created its persistent bundles with a console flag that means "draws inherit the executing pass's targets and bindings". Such bundles record against 1x1 dummy targets. Render targets, constant-buffer, shader-resource, sampler, vertex-buffer and index-buffer bindings, and pipeline state come from the pass that executes the bundle. An explicit null overrides inheritance. Missing this made the world draw into 1x1 targets. A capture showed 401 draws into the main HDR target before the fix and 997 after. The DXVK path needed the same inheritance in its replay.

Rules:
- Log the creation flags of every deferred context or bundle on first sight.
- Retention can be intentional. A bundle table that never frees is not always a missing Release.

### Command bundles are a growth root

In one title the wrapper kept a table entry for every bundle the title filed: title address to native bundle plus a draw snapshot. Entries were replaced only when the title re-filed at the same address. After three races the table held 62,475 entries:
- About 55% were dead (34,092): the title had overwritten the 64-byte tag block we wrote into the bundle buffer.
- About 94% were never executed through the table at all.
- Nothing else pinned them. Every entry had reference count 1.
- Ordinary direct command lists stayed flat at about 320.

A sweeper reclaims only provably dead entries. The tag must be overwritten or unmapped, seen dead on two passes 30 s apart, and at least 600 presents old. The native list is released after 16 presents and only once the present fence passes the last possible submission. Release is rate limited, 64 per present. The first version removed 10,000-13,000 entries at once and caused 178-958 ms hitches. Chunks of 128, destruction outside the lock and the rate cap fixed that.

Result on an AMD Windows desktop over six three-race runs: process private (committed) memory 13-15% lower in races two and three, about 1.7 GiB, and 3-6% lower in race one. Frame rate was equal. No lookup misses occurred, and no device loss. We do not call this a working-set saving: working set differed by about 0.2 GiB, inside the run spread. The effect on physical RAM on the handheld is unmeasured. We left about 27,000 never-executed bundles with an intact tag alone, since the title could still execute them and we have no rebuild-on-demand fallback. It is on by default only on Vulkan, because vkd3d-proton replays bundles on the CPU and no run exercised GPU-fence release on a real GPU bundle.

A grave that frees a replaced bundle after 16 presents does not prove the GPU has finished with it. Safe retirement needs both "no owner can replay it" and "every execution complete".

### Descriptor rewrites and in-flight work

Before an NVIDIA Vulkan device loss, one title rewrote 23,674 shader-visible descriptor slots that submitted, not-yet-finished GPU work still used. No fence wait lay between. NVIDIA loses the device. AMD tolerates it. Waiting only when a write hits an in-flight slot stopped the crash three of three times, including a full 4090 race. But early versions ran at 4-9 fps, and the slowness came from the fix's own bookkeeping. A later version measured 0.086 us per use and 0.454 us per write in a CPU lab.

Design review findings on the fix:
- **A single "last list" stamp is not enough.** Recording a newer list can overwrite the remembered owner while an older submitted list still reads the slot. Track submitted use per queue and by completed fence epoch. Bind-time ownership alone is insufficient.
- **Never wait under a lock that submission needs.** The reviewed designs waited under a publication lock. A cross-queue dependency could then block the very progress that satisfies the wait. Release the lock, wait, then recheck.
- **Wait counts from a slow run do not predict cost at speed.** At low fps the GPU has more time to retire work. Rewrite ages and waits per frame depend on CPU lead. Measure the whole design at target throughput.
- **A guard that changes several things at once proves nothing about which thing mattered.** Our first guard copied tables, used fresh private heaps, rebound state, normalised unknown views and added CPU work. Survival did not identify an active ingredient. Later guard failures superseded the early "perfect success" headline. Separate the interventions.

Before comparing Vulkan with D3D12, check these on any wrapper:
- A command-list "write immediate" slot may be a stub. It was a no-op in our base.
- A waitable swapchain flag set unconditionally can stall, with a fixed 1,000 ms wait. In some runs the waitable object never signalled.
- Use SDR for the 2x2 comparison. HDR and tearing need separate checks.

### An external report, checked

An NVIDIA developer-forum report said one vkd3d-proton configuration (the descriptor-heap extension path) loses the device on NVIDIA. We checked our logs: the bundled version had no such extension, and every NVIDIA run used the default descriptor-buffer path. So the report did not apply. Turning descriptor-buffer off had been tested before and did not cure our loss. Check the descriptor model in your vkd3d log before importing someone else's fix.

### Descriptor values as a diagnostic

An optional passive-removal dump records descriptor slot values on device removal. It is off by default, and no real fault capture has joined it to an owner yet. A diagnostic of this kind needs the owner record before it can be evidence.

---

## 8. Resource slabs, heap types, lifetime and failed-wait release bugs

### Slabs must keep the effective heap type

Our loader returned raw native resources to the title, and translated requested buffer heaps to effective CPU-visible types. A proposed slab or wrapper keyed only by the requested type would break that contract. Partition by effective heap type. Reuse the existing registry and lifetime mechanism instead of adding a parallel one.

### Mirror resources are memory, not copies

On the handheld under Proton, one title used about 16 GB: RSS about 6 GB plus about 9.6 GB of GTT, the system memory the GPU uses. A 512 MiB "snapshot diet" saved nothing: GTT went from 9.585 to 9.725 GB and RSS from 6.084 to 6.117 GB. The 5.7 GB we had labelled "title shadows" were the native resources we create for title-placed resources, about 3.65 GB of textures and 2.0 GB of buffers. They were GPU resources, not CPU duplicates. Change detection already used hashes, so removing snapshots could not help. The heap split was about 6.0 GB upload-type and 4.1 GB default.

Rule: before you shrink a footprint, measure what it is made of. We spent a lane optimising the wrong thing.

A candidate next lever opens an existing heap over title memory. That has a strict lifetime rule: withdraw the heap after a fence, before the title frees or decommits the range. Never free memory under a live heap. Keep backing fence-retained. Create the heap over the committed range, not over the whole reservation. It can reach only the buffer part, about 2 GB. We do not use it on the 4090.

### Failed-wait release

After ExecuteCommandLists succeeded, the queue Signal failed or a 10 s wait timed out. The upload callers then Released their upload resource at once. A timeout does not prove the GPU stopped. The failed wait turns into use-after-release or device removal instead of a controlled error. Separate "never submitted" from "submitted, completion unknown". Quarantine the second until fence completion. The same applies to debug fences: a failed Signal or a device removal should quarantine live resources, not allow premature release. Retain the source, list, allocator and readback leases until the debug fence really completes.

### Native retirement is not a leak

Native D3D12 held completed resources through a 32-slot deferred ring. A reference count of 2 right after Release was not a bug. A deferred release that frees a resource behind queue completion failed our test. It produced an unexpected native reference in 3 of 3 laptop runs, and we left it unmerged. A passing AMD run did not override that.

### Startup address space

A title failed about 1 launch in 100 on native Windows, with an access violation 1-2 s after start, before any frame. The cause was address-space layout randomisation. The kernel placed the main stack, process block and first thread block in a slot about 9 MiB long, anywhere in the first 1 TiB. Our loader asks for 16 GiB at a fixed address in one call. If the slot fell inside, the whole hold was lost, silently. The title asked for its ranges in five pieces and handled a failed reservation by recording a pool of size 0. Pool setup then computed 0 entries, allocated a 0-byte table and looped to entries minus 1. That underflows. It wrote off the end of the heap block until it hit an unmapped page. The fault address varied every time, which is why it looked random. A lost hold affected about 1.85% of launches. On the 4090 it was fatal in 4 of 4 saved logs. AMD ran normally if the slot missed every ask.

Fixes we rejected: moving the stack (the process block and thread blocks share the slot and cannot move), reserving only the pieces beside the hole, and a non-relocatable DLL (the slot is placed before any DLL loads). A link flag that turns off high-entropy addressing gave 0 lost holds in 1,000 probes, but it moves every allocation in every launch, so we did not choose it. We chose: when a hold is lost, relaunch the same executable, up to 4 tries, before the log opens. The first process stays as a proxy for exit code, output and priority. A job object kills the child with it. A launch whose holds succeed runs none of this.

Lessons:
- An unexplained, rare, fatal start crash may be your address-space reservation.
- The title's own error handling for a failed reservation can be the actual crash site.
- Log where the stack and process blocks sit on every launch, switch on or off.

---

## 9. GPU-produced buffers and zero-content certificates

A zero-content certificate lets the translator skip or suppress an upload because "the title's data is all zero". It is dangerous. Our rules:

- **Classify the producer before you copy.** If a GPU pass wrote the buffer, a CPU shadow of zeros is not the truth. Retain generation and queue ownership. Do not refresh CPU to GPU blindly. A blanket refresh destroyed valid GPU-produced rendering data in one title.
- **A zero certificate must describe the immutable payload being submitted.** It binds to a generation and snapshot version.
- **Incomplete proofs have no authority.** If the proof did not scan every meaningful byte, it cannot suppress an upload.
- **Bound proof visits.** Charge scratch memory at admission.
- **Interior offsets in heap imports.** Base-only heap imports worked in a fixture. Tested interior offsets still failed. We did not relax the guard.
- **Stale nonzero uploads exist too.** An existing zero-source ownership protection can still admit a stale nonzero grading-volume upload. The code hole is real. No upload and readback proved it caused the washed-out colour we chased. The authority switch is default off, because it can suppress legitimate same-generation CPU reinitialisation. For the same reason we do not force a sRGB format, change a packed-float format or boost global saturation.

Related: one title kept progression (XP, inventory, wallet) in separate mutable files. That allowed partial awards. Use one writer, atomic flushed transactions, expected balances and idempotent receipt identities. Back up the native save and the ledger together. This is a state-integrity rule that also fits any translator that mediates saves.

---

## 10. Measurement pitfalls

This section is the one we would give a new team first. Our biggest wasted weeks came from numbers that did not measure what we thought.

### Stalled baselines

The faster OFF runs on two machines had been stopped at test barriers while the ON runs drove. The matched pair on a third machine did not support the headline collapse. A stalled baseline cannot measure the cost of a streaming fix. Require matched moving routes. Check that the controlled object moved for the whole route. In one case, a stopped car in two of three sweep-off route runs and in none of the sweep-on runs made race-one comparisons void. We kept those out of the table.

### Diagnostics can dominate

- A crash recorder that formatted and stored every draw under one history lock held that lock 93% of the time (56.9 s of 61.0 s). Render workers piled up 192 s of overlapping waits. One title ran at 6 fps on Vulkan, and the cause was our own diagnostics. A compact binary recorder took the same drive from 6.0 to 29.2 fps on a 6700 XT. On the 4090 with all diagnostics off, the title held its 30 fps cap. With them on it ran 9.45 fps.
- Per-visit texture diagnostics cost a lot: the same frozen binary with diagnostics off recovered much of the speed. Do not subtract historical route results as a measured lane cost.
- A first-visit log wrote 976,000 rows per run. It flooded the last 300 lines that a controller read for timing. The controller died, and the whole cohort was void.
- Pipeline-state trace recorders are also diagnostics. State the diagnostic set with every benchmark.

### Test captures create the lows

A title's 1% lows of 18-20 fps against a 30 fps cap looked like a real stutter. Our own test captures caused them. With a screenshot every 2 s, the loader stopped the present thread, waited for the GPU readback, converted the image and wrote a bitmap before Present. The median was 20.7 ms on the handheld. That turns a 33 ms frame into about 54 ms. Of 847 intervals over 45 ms, 840 followed a capture directly. Capture duration predicted the next interval with a Pearson correlation of 0.9916. Average fps was unaffected (29.9998). Measure lows with captures off, or capture asynchronously: copy to a staging buffer and write on a worker. State the capture schedule beside every benchmark.

### Frame caps and animation clocks

Titles ran well above 60 fps in routes where timers, vehicle integration and supplied particle deltas supported it. Our first particle probe mistook a per-update delta for an absolute clock, and a ratio-based jump filter biased results. Count actual updates, resets and supplied deltas without ratio-based acceptance filtering. A frame cap does not certify the animation clock. Validate each independently.

### Inverse p99 is not mean-slowest-1%

Our "1% low" was an inverse 99th-percentile frame time with conservative 0.1 ms bins. That is not the mean of the slowest 1% of frames. Two reports using the same label described different statistics. State the window, denominator, units, histogram quantisation and diagnostics for every number.

### Wrong cause from correlation

- Turning a residency probe off removed logged stalls but did not restore the old speed on the handheld. A power plateau and low clocks did not prove thermal throttling. Header-only telemetry cannot prove temperatures. We did not infer a leak or remove a visual repair from these correlations. Repeat controlled old/new routes at recorded power and session state.
- A shader pre-build helped cold-cache boots. Boot to race fell from 233 s to 138 s on a laptop, 336 s to 142 s on an AMD Windows desktop, and 159 s to 113 s on the handheld. But the first laptop "OFF" figure of about 199 s used vkd3d's warm default cache, so it understated the saving. Any A/B that toggles pipeline precompile must keep the cache path identical in both arms. Each cold-ON figure outside the handheld was one run. Warm launches were neutral (105.8 s OFF, about 108 s ON).
- Harness bugs void cohorts. One script forced a switch to 0 in every handheld session, so the arm that was meant to be ON ran OFF. A promotion based on those sessions had to be rechecked. Read the effective switch set from the run's own census, not from your plan.
- Check that your verifier does not reject your switch set. Our receipt logged only titles' own prefixes, so cores' generic switches failed the check on every try.

### Noise

Same-config runs of one title varied from 75 to 148 fps. Rules we follow:
- Interleave A and B (A B A B A), repeat A last, and record every try.
- Use latency metrics (demand to landed, median and 95th percentile) for streaming changes, not raw fps.
- Judge by the effect size against the noise floor: a +27-30% change repeated every run is real. A few percent is not.

---

## 11. Memory footprint: RSS, GTT, commit and alignment

### Which number are you reading?

- **Private (committed) bytes** counted what the sweeper changed. The working set did not move.
- **RSS** is resident host memory.
- **GTT** is system RAM mapped for GPU use. On an APU or handheld it is where "VRAM" lives, so it competes with RSS for the same physical RAM. A title using 24 GB on the handheld was RSS plus GTT plus swap, not a leak of one kind.
- **Peak** and **settled** values differ. In one comparison two builds ended at 16.1 and 16.8 GB, and sat at 13.6 and 13.7 GB at 300 s. That showed the newer build did not add memory.

Always chart RSS, GTT, committed and swap together, at 5 s sampling, for a fixed route.

### Large effects we measured

- Command-bundle table sweeping: 13-15% lower private memory in later races, equal fps (section 7).
- A tight buffer alignment option (use the native alignment instead of padding every buffer) reduced growth. Without it, one earlier estimate of +5.3 GB per extra race did not reproduce. With tight alignment on, growth from race one to race three was +1,859 MiB in total, and race one to two was about 0.
- In a focused matched pair with a repaired cycle of resource-description queries, private memory fell from 15,343 to 10,247 MiB (-33%), and working set from 11,807 to 6,958 MiB. That was a single sequential pair on one desktop. It has no handheld result and no live deployment. We treat it as promising, not proven.
- An early version of that tight-buffer path crashed after 1.9 s. The flag masked the tight option, but the native resource kept 256-byte alignment, so the footprint came out invalid. A GetDesc cycle must return what the title asked for, so the title and native views agree. We fixed it in a CPU fixture with 90 assertions, and still needed three native old-failure and fixed-success cases.
- A pending texture queue that holds a reference to each pending job added about 11 GB in one build when service starved. Bound it by bytes and by jobs.

### Things that did not help

- Pooling staging textures with a 1.3 GB cap: staging creations fell 75%, private-byte growth was unchanged.
- A 512 MiB snapshot diet: no saving.
- Allocation and memory-policy experiments did not establish a cure for a visual fault. An early laptop improvement did not reproduce. Record the lever and its confounds. Partial slab coverage and a slow run limit any broad negative conclusion.

### Alignment

Console buffers and constant buffers are not desktop-aligned. Padding every buffer to desktop alignment costs memory, and desktop padding can exhaust console-sized arenas (section 2). Padding constant-buffer offsets to 256 bytes is necessary, but do it only where reads need it. Tight alignment plus a faithful description cycle is how we recovered memory without hurting correctness.

---

## 12. DXVK and vkd3d path differences that touch memory

We kept two backends for different titles. The D3D12.X titles go through vkd3d-proton. One D3D11.X title went through DXVK. These differences mattered for memory and data:

1. **Min-LOD is ignored.** DXVK 3.1.1 made SetResourceMinLOD a no-op. A title that hid unwritten top mips with a min-LOD clamp then sampled zero blocks: black for BC data, alpha 0 for cutouts, so alpha-tested scenery vanished. The stopgap is a per-view most-detailed-mip and mip-count, with views recreated when residency changes. A shipped-DLL probe confirmed that an untouched mip 0 sampled RGBA 0. It did not cure all the black geometry on its own. Test whether any path relies on a clamp your backend may not honour.
2. **Descriptor ownership lives in the backend.** A wrapper-managed descriptor rewrite guard (section 7) has no counterpart on DXVK, which owns descriptor allocation, update and retirement itself. The in-flight descriptor stall does not apply there.
3. **GPU results do not reach title memory.** Under DXVK, CPU-side DMA ops see stale data (section 1). On vkd3d, a CPU/GPU mapping is more direct. State per backend which copy owns the truth.
4. **Staging in host-visible memory.** DXVK placed full-chain staging textures in the host-visible heap, the one that filled before a crash. Under vkd3d a pooled upload ring is cheaper. We saw 18.97 million write-watch harvests in 10 minutes on the DXVK path, two VirtualQuery walks and a per-texture query per refresh.
5. **Constant-buffer alignment is not solved by the API.** DXVK's SetConstantBuffers1 does not remove the 256-byte rule for unaligned console offsets. Use the residue method (section 2).
6. **Bundles replay differently.** vkd3d-proton replays bundles on the CPU. DXVK needs inheritance implemented in its own replay. That is why our bundle sweeper is on by default only for Vulkan.
7. **The Proton default.** Under Proton, the title's own D3D12 path already is vkd3d-proton. A wrapper that defaults to loading a bundled Windows vkd3d fails with error 126 when the bundle is not deployed, and doubles translation when it is. Detect Wine or Proton (the ntdll Wine version export) and use the system path, unless the user sets a backend. The title sat idle until a one-hour timeout because the failure was silent.
8. **NVIDIA descriptor behaviour.** NVIDIA Vulkan loses the device on in-flight descriptor rewrites that AMD tolerates. Count in-flight rewrites with a logging build before you blame anything else on NVIDIA.
9. **Cached memory flags differ per platform.** WC reads are about 4x slower on Windows than under Proton (section 5). Budget copy time per platform.

---

## Checklist for a new translator

1. Make every unimplemented slot loud. Log each call and count it.
2. Emulate DMA decompress and fill with queue order, retirement and fence waits designed first.
3. Decide per resource who owns the data: CPU or GPU. Log every change of owner.
4. Retry rejected CPU-visible textures as DEFAULT storage plus a shadow.
5. Prove layout offline with cycle tests before debugging detile.
6. Do not hash whole textures. Keep a retained exact copy or use write-watch, and test each alone and combined.
7. Never read write-combined memory with plain loops. Use streaming loads, and never leave a diagnostic scan on.
8. Replace VirtualQuery in hot paths with your own region map.
9. Size uploader caps from the measured backlog. Watch both the median and the tail of latency.
10. Treat readback, committed memory, padding and a silent probe as weak evidence. Join pixel ownership.
11. Track descriptor use per queue and fence epoch. Do not wait under a lock that submission needs.
12. Quarantine, never release, resources after a failed wait.
13. Sweep dead bundles only on proof, in small chunks, with a fence check.
14. Measure on matched moving routes with diagnostics and captures off. Record every try. State statistic definitions.
15. Chart RSS, GTT, commit and swap together. Measure what a footprint is made of before shrinking it.
16. Check for rare launch crashes from your address-space holds.
17. Detect the Proton runtime and use the system D3D12 path there.

---
[Back to the index](../README.md)
