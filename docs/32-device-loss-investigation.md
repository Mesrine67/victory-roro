# Chasing an NVIDIA device loss in a D3D12-on-Vulkan translator

This is an engineering account of our longest-running open bug. We translate an Xbox One D3D12.X title to run on PC. The translator targets D3D12 and runs on vkd3d-proton, which turns D3D12 into Vulkan. On NVIDIA GPUs, the device is lost a few minutes into play. We have not fixed it. We have learned a lot about how to look for it, and about how our own tools changed the answer.

We write this for someone building a similar translator. It covers the symptom, the crash tools, every fix we tried with numbers, what we ruled out, the theory that descriptors are volatile, the statistics traps, and what we would build on day one.

One honest summary first. After many weeks and several hundred test starts, we can say this with confidence: the GPU reads a page of texture memory after that memory was freed. We cannot yet say whose stale reference does the reading. Every cure we tried either failed or could not be shown to work.

---

## 1. The symptom and its signature

### What the player sees

The game runs normally, usually in a race. Then the picture freezes. The title notices that the device has gone. The title's own code then crashes with a null write, because it does not handle a removed device. The crash address inside the title is the same in every loss. That sameness is itself useful: it tells you two losses are the same event even when the GPU-side detail differs.

### What the logs show

Every loss has the same sequence:

1. vkd3d-proton reports `VK_ERROR_DEVICE_LOST` on a timeline-semaphore wait or a queue submit.
2. The title calls `GetDeviceRemovedReason` and gets `DXGI_ERROR_DEVICE_REMOVED` (0x887a0005).
3. The title crashes with a null write inside its own code.
4. On Windows, the NVIDIA kernel driver logs a timeout-and-reset event. On Linux the equivalent is an Xid.

The logs do not give us a fault address, a shader, or a command. No breadcrumb survives. You need extra tools for that (see section 3).

### How often, and when

Rates depended heavily on the test setup, which is part of the story (section 5). Rough figures:

- On the RTX 4090 desktop at normal speed, time to loss from launch was 195, 226, 385 and 430 s in one series. That is a median near 5 minutes. A fuller set of 4090 starts gave a median near 3.8 minutes.
- On the RTX 3080 laptop at a forced 15 fps, a control with all diagnostics off lost the device in 5 of 5 starts, at 182 to 435 s.
- On a later, quieter build, the default play configuration on the 4090 lost about once per six ten-minute runs.
- The player's own play session on the 4090 lost the device after about 37 minutes of normal play. The driver logged 43 reset events across 7 days on that machine.

Losses cluster. On the 4090, 6 of 14 fell in lap 2 or 3 at lap-seconds 7 to 14, and 5 fell early in lap 1. On the laptop they clustered at two places on the route. We think the clusters mark moments when the title frees streamed textures in bulk.

### The faulting access

Once we had good crash tools, the pattern became very consistent:

- an MMU read fault in the graphics engine;
- by a fragment shader (and once, a vertex shader);
- at an address where every resource the tools could find was already destroyed;
- in the last 4 KB page of a block-compressed streamed texture, which is the packed tail of the small mips. In 9 of 10 dumps the fault was in a mip of 64 by 64 texels or less;
- on memory that the title had released 5 to 389 seconds earlier.

The same size of texture was often created and destroyed again and again at one title address. That matters for the "freed minutes earlier" claim, as we explain in section 3.

### Which hardware

NVIDIA only, almost entirely. The same title ran for long periods on the AMD desktop under Linux/Proton with no loss. We did get two AMD events on the AMD handheld (SteamOS/Proton). One was a GPU page fault with the same title-side signature, a texture-cache read fault. The other had no page fault at all: the platform power-management firmware stopped answering about 40 seconds before the graphics ring timed out, and the machine rebooted. So AMD is not immune, but the 4090 and the 3080 laptop reproduce it far more readily.

Native D3D12 on the laptop completed three laps with no removal. So the bug needs the Vulkan path. That points at the translation layer or at our code, but it does not separate them. Native D3D12 has its own failure modes in our stack, so this was a weak control.

---

## 2. The shape of the investigation

We worked in four phases. Knowing the phases helps you plan your own.

1. **Blind setting splits (first week).** We toggled switches and watched whether the device died. Almost nothing changed the outcome. This phase produced the long "ruled out" list in section 4 and very little understanding.
2. **Vendor crash dump (day 6 onward).** An NVIDIA crash dump named a shader and an instruction. That was our first real data.
3. **Fault addresses and a memory ledger.** A Vulkan extension gave us fault addresses. A custom ledger of every allocation and free let us join an address to a freed texture.
4. **Lifetime experiments.** Hold, retain, and slot-audit tests tried to prove which stale reference is the culprit. They mostly came back inconclusive.

We also did a public-source search only after about five weeks. We should have done it on day one. It found no report that matches our signature, but it gave us useful tools and caveats, which we list in section 3.

---

## 3. The crash tools, and what each one told us

### vkd3d-proton breadcrumbs

vkd3d-proton has a breadcrumbs option that records which command was in flight. It needs a build that supports it. We ran it on both NVIDIA machines. Both lost the device (about 302 s and 219 s), and neither produced a usable checkpoint region. Verdict: inconclusive. Breadcrumbs add a checkpoint per command, so they also slow the game a lot. NVIDIA's own advice is to keep checkpoints to about 100 per frame or fewer.

### NVIDIA Nsight Aftermath crash dumps

Aftermath works through vkd3d-proton and through Vulkan. This surprised us, because the common advice is that it needs its own integration. We used the crash-dump monitor in global mode, which needs administrator action on the machine. That is the owner's decision, not a script's.

The dump gives you the faulting shader's hash, the program counter, the fault type, and recent resource records. To map the hash back to a SPIR-V module you need the SDK's shader-hash function, and you need to dump the SPIR-V from vkd3d-proton at the same time.

What the dumps told us, in order:

- **The first dump** named a vertex shader and a "shader instruction error", with no page-fault record. We mapped the instruction to a bindless texture fetch: image from the descriptor heap at a push-constant offset, sampler from another index, an explicit-LOD sample. We read that as "invalid or wrong-type descriptor in that slot". A second dump confirmed the same shader and the same instruction.
- **Later dumps**, from a different harness and a lighter capture mode, were MMU read faults in fragment shaders, not instruction errors. Five of five valid starts in one series were this class, and the resource rows at the fault address were all destroyed resources.

So two different classes of GPU report came out of what looks like one bug. That is a warning in itself. Treat the first dump you get as one sample, not as the diagnosis.

One more Aftermath warning, from NVIDIA's public release notes: the older page-fault lookup can name the wrong tracked resource when resources overlap or reuse a GPU address. The newer decoder call, `GetPageFaultResourceInfo`, returns the full history at the address. If your resource was "freed minutes earlier", check whether a newer resource lives at that address now. Our streamed textures were created and destroyed at one address over and over, so this applies to us.

### VK_EXT_device_fault

This extension reports fault addresses after a device loss. We built a collector layer for it. It had a bug on its first runs: it skipped the detail query when the vendor binary size was zero, so it never fetched the addresses. The counts query had succeeded and said there were two addresses. After we fixed the collector, both NVIDIA machines returned a READ_INVALID page with 4 KiB precision.

That gave us the first real pointer: a read of a page inside memory that had been unbound and freed 21 to 34 seconds earlier.

Lessons:

- Read the extension's counts before you trust a zero-sized binary.
- The addresses are only useful if you also log every allocation and free. We added a side ledger for that.
- Our first ledger logged every event as text. It grew to 3.3 GB and dragged the game to about 3.3 fps. The run no longer looked like a loss run. The fix was a binary ring of allocations, frees and binds of 1 MiB or more, plus a protected table of the first 4,096 frees that is never overwritten. That version cost under 5% fps in the design, though we did not measure it cleanly.
- The device-address-binding-report extension (`VK_EXT_device_address_binding_report`) gives GPU addresses in the driver's own address space. That is the space the fault addresses live in, so the join is sound.

With the slim ledger, the laptop fault page mapped to a dedicated 5.6 MB allocation for one texture. The title had released it, and the native free returned 48.8 s before the loss. The game kept presenting for about 1,970 frames after the free. Nothing reused that memory. On the 4090, the page was in a 16 MiB vkd3d-proton host-visible chunk with many sub-allocations. That was the same class but could not be tied to one texture.

A subtle point a reviewer made: a READ_INVALID fault tells you the page was unmapped when the GPU touched it. If the address had been re-bound to live memory, the read would have returned wrong texels silently and never faulted. So the ledger's "no reuse" result is partly implied by the fault type.

### DRED

DRED (Device Removed Extended Data) is the D3D12 way to get breadcrumbs and page-fault output. vkd3d-proton only has stubs for its interfaces. So DRED cannot help on the Vulkan path. It would only help in a native D3D12 run, where it can name the faulting virtual address and any recently freed allocation. Our native runs did not lose the device, so we never got a DRED output.

### Debug layer and GPU-based validation

On the native D3D12 path, the debug layer found two real bugs that were not the cause: a wrong barrier before-state, and a texture used through a buffer-typed view. Both were worth fixing. Neither cured the loss. The laptop lacked the SDK layers for GPU-based validation, and installing them is a system change, so we did not do it there.

### Other tools

- A descriptor-QA mode exists in vkd3d-proton. It logs descriptor writes and instruments shaders, but it needs a build with a special flag. We built our own descriptor-ownership witness instead (section 6).
- Nsight Graphics GPU Trace failed with "counters unavailable" until the owner allowed counter access for all users.
- RenderDoc on the Vulkan path lost the device in 2 of 2 runs. It also changed the reported GPU vendor, so our vendor-specific guards switched themselves off in captured runs.

---

## 4. Every fix and theory we tried, with results

The table has what we tried, the result, and what it rules out.

### Fixes that changed nothing

| Attempt | Result | Rules out |
|---|---|---|
| vkd3d-proton deferred resource destruction | Lost at 314 s (laptop) | Short-lived in-flight lists as the cause |
| Our own deferred release behind full queue completion | Failed 3 of 3 on the laptop | The same |
| A 60 s hold of freed textures, 6 runs on the 4090 | Lost 3 of 6 | The hold as a cure. It did not prevent the loss, and the gap to the loss just stretched |
| Wait-on-reuse and descriptor-snapshot designs | No loss only at 0.5 to 10 fps. Stall arms died of CPU or memory exhaustion at 287 to 392 s | Playable wait-based fixes |
| Typed null descriptors for empty slots | Lost at 374 s despite 1,086 substitutions and 183 observed mismatches corrected | The NULL-2D-versus-3D mismatch as the only cause |
| A texture-budget cap | A halved budget lost at 743 s. VRAM stayed over budget in every arm, and several 4090 losses happened inside the budget | Over-budget memory as necessary or sufficient |
| A 1100 MiB streamed-texture budget, 4090 | 0 of 2 lost, against 4 of 4 controls | Nothing yet. It fits a stale-slot-reuse cause but n is tiny |
| Disabling VK_EXT_descriptor_buffer | The laptop lasted 462 to 490 s. The 4090 lost at 109 to 137 s | Descriptor buffers as the only cause. It only delayed the loss |
| An NVIDIA-specific descriptor guard | See below | |
| Driver 617 versus 621 | Lost on both | Driver version as a cure |

### The NVIDIA guard

We had a guard that copies descriptor tables into private heaps at draw-record time on NVIDIA. It treated a type mismatch as the likely cause. Results:

- The UAV guard on 4090: three runs survived 540 s, then a fourth lost at 227 s.
- Split tests on the laptop: guard fully off lost at 79 s during loading, SRV-only lost, UAV-only survived 3 of 3. But a later 4090 policy arm lost at 281 s.
- A guard-off loss existed, but those losses were during loading and were never attributed. A later review corrected our early claim that the guard was excluded. Every loss we attributed to a texture sample ran with the guard at its default, so the guard is not excluded.

The guard is also a descriptor-volatility violation (section 6).

### The retention patch

This was our best apparent result, and our best lesson in statistics. We built a private vkd3d-proton variant that keeps freed memory mapped and reuses chunks later, with a cap and a dwell time. Results:

- 0 of 20 racing runs lost the device with retention on.
- 0 of 3 matched controls lost it.
- The historical rate at the time was about 6 losses in 31 runs. If you assume independent runs and a steady 6-in-31 rate, twenty clean runs would be 1.35% likely. That sounds decisive.
- But the 6-in-31 rate came from a different mix of runs, and a corrected rate of about 1 in 29 for the clean-condition runs makes twenty clean runs about 50% likely by chance. Zero of twenty meant very little.
- We then tried to get a proper baseline. Fifteen retention-off runs in a matched harness gave 0 losses in 15 racing exposures. The baseline never reproduced the loss, so the retention effect could not be tested.
- Cost: about +265 MiB of VRAM and +1.8 GiB of host memory.
- It also changes three things at once: it keeps chunks, redirects dedicated images, and enlarges chunks. Even a clean result would not say which one mattered.

We also built a poison-fill detector to catch stale reads under retention: fill retained memory with a marker, then look for the marker in the frame. It only covers BC1 and BC3 formats. The first fixtures failed on the wrong fill pattern, and the first real race produced zero reuse, so a stale read would never have shown up. We needed a minimum-dwell provider (390 s) to get any reuse, and even that left several block formats outside the detector.

### Slot audit and draw witness

Two measurement experiments tried to classify the stale reference.

- **A loss-time slot audit.** It kept a slot-owner ledger and classified each loss as: the title's live table names a released texture (A1), a stale copy made by us or vkd3d (A2), or a slot-reuse race (R). In 10 real runs there were 2 losses, and no slot named the freed texture. Definition of done was 3 classified losses. It got 0.
- **A draw witness.** It recorded the slot inputs at submit time. It reproduced the loss once in 8 racing runs. The faulting shader read its texture index from a material word at a fixed offset, added to a table base. The witness's reduced observer had sampled a different shader family, so its records could not identify the descriptor the GPU read.

### A bisect of our own instrumentation

On the laptop at 15 fps we could reproduce the loss in nearly every 600 s start (180 to 435 s). We then bisected which of our own diagnostics changed the odds. Final tally: diagnostics off 0 of 6 survive; recorder only 2 of 4; RT scaffolding 0 of 2; full RT 2 of 3; sync only 2 of 3. Survivals looked like chance or timing. We stopped the bisect, but it gave us a fast reproducer for a fault-capture run.

---

## 5. Statistics pitfalls

Our biggest wasted effort came from reading small samples as results. Here are the specific traps we hit.

1. **"Very rare" was a harness artefact.** A set of figures (1 loss in 29 runs, 0 of 15) led us to park the bug. Then one harness produced 5 losses in 5 valid starts, and the earlier pop-in build lost 3 of 8. The rare figure came from a different route and different runs.
2. **A hidden confound in the 5-of-5.** Those starts ran at 11.4 fps. The display was off at launch, and the game ran in a slow mode. With the display held on, the first 15 s ran at 48 fps. Test machines need a display-state check in every row.
3. **A false correlation with a setting.** An eager-staging switch was on in 5 of 8 lost runs and 0 of 17 clean ones. A paired A/B of 6 pairs then gave 1 loss with it on and 1 with it off. The earlier split was confounded by other differences between those runs.
4. **The corrected rate.** One calculation used a 1-in-5 rate and said twenty clean runs had 1.35% probability. At 1 in 30, twenty clean runs happen about half the time. State the rate you use and where it came from.
5. **Run length.** Fifteen overnight runs held only 74 minutes of moving racing. About 5 minutes each. Use driving time, not run count, and subtract stuck or parked time. One series used "loss per 1,000 s of driving".
6. **Unequal exposure.** One series raced in 6 of 11 starts. The others stuck at a wall. Report losses per driving second.
7. **Fisher on tiny counts.** A p of 0.07 to 0.14 on 0 of 2 against 4 of 4 is a lead, not a result.
8. **Many arms, one loss.** A set of 9 arms had 3 losses among 6 ON runs and 0 among 3 OFF. That looks like "the hold causes it". A later 15-run OFF set also had 0. Mixed rates on different build days broke the comparison.
9. **Environment noise looks like crashes.** Host commit exhaustion killed one start. The Windows commit limit swung between 78 and 92 GB because the pagefile was manual, and builds on the same machine took 3 to 9 GB each. The user's desktop activity stole OS focus in four starts and failed two routes. Control the machine, or at least log its state.
10. **Success of a run is not correctness.** A stale read can silently return a replacement texture. "Survived" does not mean "correct texels", because the driver reuses GPU addresses.

Practical rules we now apply:

- Same build, same machine, same route, three controls before you trust one arm.
- Record the display state, focus, commit headroom and driver in every row.
- Count a run only if it reached the loss-prone stretch.
- Never promote a cure from zero-of-n unless the baseline rate makes n clean runs unlikely, and say so.
- A negative result needs a positive control. Our poison detector had one for BC1 and BC3 only.

---

## 6. The descriptor-volatility theory

### The contract

D3D12 root signature version 1.0 treats descriptors in a table as volatile. Microsoft's public documentation for version 1.1 describes the flag and says it is "the only supported behavior of Root Signature version 1.0". The app may change a descriptor any time except while a command list that binds the table is submitted and not yet finished.

So a conformant title can do this:

1. Record a command list that uses slot S.
2. Rewrite S to point at texture B.
3. Release texture A, the old content of S. No submitted list references it.
4. Submit.

The GPU must read what the heap holds when the list executes. A translator that copies the descriptor at record time and binds the copy is not conformant. The copy still points at A, which is now destroyed.

Public sources:
- Root Signature Version 1.1 (flags and the 1.0 equivalence): https://learn.microsoft.com/en-us/windows/win32/direct3d12/root-signature-version-1-1
- D3D12 Resource Binding spec (restrictions on changing state pointed to by descriptor tables; object lifetime): https://microsoft.github.io/DirectX-Specs/d3d/ResourceBinding.html

vkd3d-proton treats every 1.0 descriptor as volatile, using bindless and update-after-bind features. Hans-Kristian Arntzen says this in his public write-up: https://themaister.net/blog/2021/11/ . In recent vkd3d-proton, descriptors are written directly into descriptor-buffer memory, so the shader reads the heap at execution time. Volatility is native there.

### Why we suspected our own guard

Our NVIDIA guard copies bound tables into private heaps at record time. A CPU fixture driving the real guard code proved it: a UAV-PSO draw rewrote slot t0 from A to B, released A, and the guard's copy still named A at execute. Nothing refreshed the copy at submit. Variants also froze: lists executed twice, cached re-binds, and the SRV-guard path.

A laptop log showed 23,674 in-flight slot rewrites in one run before we looked closely, which proved the hazard exists but not that it faults.

### The fix, specified and built

Refill at submit. At each submission, for every copied slot whose source slot changed since the copy, write the copy again by the same rule. Never touch unchanged slots. Add no wait.

Why no fence is needed: if the title changed a slot since our copy was filled, the title obeyed the rule. So no earlier submitted list that reads that slot is still executing. Our private copy of that slot has no in-flight reader, and an unwaited CPU write is safe.

Caveats we wrote down from the public spec:

- The guarantee covers only descriptors a shader can actually reference. Unreferenced descriptors may change while a list is in flight. If our copy of such a slot is ever read (for example by a bounds-based prefetch), a refill can race.
- A list can be submitted twice. Refill only changed slots.
- Bundles: the same submitted-and-not-finished window applies to the calling list. We refuse UAV-PSO draws in bundles anyway.
- Multiple queues: the rule is per submission to a queue. A title that uses async compute must itself fence between queues.
- A case where the rule would make us drop a draw cannot be refilled at submit. The refill writes the title's own descriptor and counts it.

A CPU fixture shows that with refill on, every case that previously froze now reads the title's current slot. A second audit mode counts mismatches after refill, and must read zero.

### How it fared

Unproven for the loss. A count-only submit audit ran on the laptop. It saw the copy path taken (83 thousand snapshots, 516 thousand records, 447 thousand executes) and zero rewritten copied slots. The lead we called "D2" (guard copy handed the GPU a dead texture) was not seen. The refill fix was parked. The same audit on the 4090, where the loss happens most readily, was still pending when we wrote this.

Two other things weaken the guard theory: the 4090 loses with the guard at its default and sometimes in places the guard does not cover, and the descriptors in a faulting draw came straight from the title's table plus a material word.

### A public-source lead

No public report matches our exact signature. Partial analogues exist:

- An NVIDIA forum thread on an MMU read fault in a vkd3d game, which went away under a newer vkd3d: https://forums.developer.nvidia.com/t/fault-pde-access-type-read-bug-still-not-fixed/199123
- Device-lost reports with the descriptor-heap and descriptor-buffer paths on specific driver branches: https://forums.developer.nvidia.com/t/vk-ext-descriptor-buffer-in-vkd3d-triggers-xid-69-on-pascal-cards-since-driver-535-series/291172 and https://forums.developer.nvidia.com/t/vkd3d-proton-descriptor-heap-path-causes-vk-error-device-lost-on-driver-610-in-the-first-descendant/374747
- A vkd3d-proton issue on Blackwell hangs, with the descriptor-heap path as a way out: https://github.com/HansKristian-Work/vkd3d-proton/issues/2914

None of these shows "descriptor changed after record, before submit" as a device-loss cause. We found no public statement of why AMD tolerates stale descriptors that NVIDIA faults on.

---

## 7. How diagnostic tools changed the odds

This section may be the most useful for another team. Our tools were not neutral.

- **The timestamp layer.** A GPU timing layer was the prime suspect in one traced loss: the traced row lost the device and untraced rows did not. Treat every layer as a possible change to the bug.
- **The text ledger.** It slowed the game by about a factor of ten and produced runs unlike loss runs.
- **The Aftermath capture mode.** Capture on lost 3 of 3 at normal speed. One capture-off start lost at 382 s with the same title crash address. A later series showed capture is not needed for a loss. Whether it raises the rate is still open.
- **The slot audit.** It lost 2 of 10. The earlier rate was about 50%, from an unmatched control. A note in the record says observer timing may lower the rate.
- **The lifetime witness.** It halved fps on the 4090 (about 14 fps). It flagged CPU-observed reads of destroyed backings in every start (108 to 911 rows), mostly in minute 1, with or without a loss. The count did not predict a loss. Of 9 valid starts, 2 lost the device, at about 751 s and 772 s, and the witness was silent near both.
- **Retention.** It changed memory placement and chunk size at the same time.
- **RenderDoc.** It changed the reported vendor and turned our NVIDIA guard off, so the capture ran a different code path.
- **Instrumentation bisect.** It showed that no single diagnostic ingredient reliably prevents the loss. Survival looked like timing.
- **Slow modes.** Every wait-based design ran at 10 fps or less. Survival there says nothing, because at low rates the title may never reach the problematic stretch of lap time.
- **QA shaders in a descriptor observer.** One tool reported "destroyed" descriptor rows. A reviewer re-checked and found these were in-flight rewrites of live views (153 rows, none after the native death). So those counts were not evidence of stale reads. Always check that your own detector does not report something legal as a violation.

What we do now: every diagnostic is off by default. Every row records which diagnostics were on. Controls run with the same set of diagnostics except the one under test.

---

## 8. What we ruled out, and why

Plainly ruled out, with the reason:

- **A short-lived in-flight list.** The gap between free and fault is tens of seconds (up to 389 s). Fence and retirement designs cover milliseconds. The hold test then showed the measured gap tracks time since the free, not reference age.
- **Re-allocation of the freed range.** The ledger was contiguous with no reuse, and the fault type implies the page was unmapped.
- **The PSO cache, the vkd3d shader cache, shader pre-build.** Each lost with and without.
- **Frame pacing and FPS caps.** Lost at 30 fps and at 11 fps.
- **Driver version.** Lost on 617 and 621. We recorded the exact driver in every row afterward because NVIDIA reported a bug on the 616 and 617 branches for a different cause.
- **The NVIDIA overlay.** Lost with it off.
- **Loader-side destruction of the texture.** A review refuted it. The title itself releases the texture.
- **A min-LOD or mip-view theory.** Refuted by review.
- **A loader-initiated read of a decommitted mip.** Open, not refuted: the title had decommitted mip 0 of one faulting texture before releasing it, and the fault lay inside mip 0. A loader-side view that still exposes the decommitted mip is a live alternative. So "legal on Xbox" has not been shown.
- **vkd3d-proton 3.1.** Lost at 335 s.
- **Bundle allocator reset timing and the presenter's assumed barrier state.** Both were real correctness bugs. We fixed them as default-off switches. Neither cured the loss.

Not ruled out:

- **Raw handles in bundles and re-executed lists** (barriers, copies, render targets) that bypass descriptors. The title uses bundles.
- **Root CBV/SRV and vertex or index addresses.** Nulling stale slots covers only descriptors written through the API.
- **The guard as a contributor.** See section 4.

---

## 9. The current best theory

We state it with the evidence for and against.

**Theory.** The title frees a streamed texture. Our translator frees the native memory, because the title's own release says nothing else will use it. Later, tens or hundreds of seconds afterward, a shader samples a bindless texture slot that still names that memory. The sample reads an unmapped page and the device is lost. The index the shader uses is a table base plus a material word, so one of those holds a stale texture index.

**For it.**
- Fault addresses sit in destroyed, un-reused ranges across both NVIDIA machines.
- The shader reads its index from a material word plus a table base.
- Dumps and ledgers agree across different build days.
- Faults concentrate in small-mip tail pages that are touched only at the edges of mip selection. That explains why the read happens rarely, long after the free.

**Against it, or missing.**
- Whose reference is stale is unknown. The candidates are the title's own live material table (legal on Xbox, where memory would not be unmapped), a stale copy we made, or a slot-reuse race. A review of the slot path found no defect in the loader or in vkd3d-proton along it.
- The hold test (60 s) should have prevented a short-gap fault, and did not. This fits "long gap" but also means the hold cannot isolate the bug.
- The slot audit and witness both came back unclassified.
- The refill audit saw no frozen-rewritten slots on the laptop.
- The "freed minutes earlier" attribution may be partly an address-reuse artefact (section 3).

**The two fix designs we have specified, both unproven.**

1. **Native lifetime keyed to the title's address range.** The native texture would live until the title decommits or releases that range, or until a newer wrapper takes the same range. Stale slots would be redirected, not nulled, so a legal reuse never becomes a black texture that reads as survival. It matches Xbox semantics. It covers buffers as well as textures. It can reuse the title-memory tracking we already have. We have not built it as a default-off arm because step one (telemetry that tells us which class the faults belong to) never completed.
2. **Refill at submit** for the guard's copies (section 6). Built and fixture-proven. The audit does not currently show the hazard on the laptop.

Any keep-alive interim must be sized against bursts of about 1.8 GB freed within 3 seconds on the laptop (1.6 GB on the 4090) at race load. A lease sized for 256 MiB would have terminated the process. Never abort the process in a diagnostic.

**What would change our mind.** A fault address outside every freed and un-reused range. Or a fault with a slot join showing no stale reference. Then look at bundles, raw handles, and loader views of decommitted mips.

---

## 10. What we would build from day one

If you are starting a D3D12-on-Vulkan translator, build these before you have a bug.

1. **A descriptor write ring.** Record `{heap, slot, resource, generation, time}` for every descriptor write and copy. About 16 bytes per write. Keep it lock-free and bounded. Dump it on device removal. Our late-added slot dump was the most useful single artefact for this class of bug.
2. **A resource lifetime ring.** Record every create and destroy with the GPU virtual address, size, kind and a generation counter. Use the driver's address-binding report so the addresses match what the driver faults on. Add a protected table of the most recent frees that is never overwritten.
3. **A device-removal hook.** Check device removal on a fixed cadence (for example every sixteenth submit, and every queue signal). Log it once, then dump both rings, the last N submits, and the fault info. Do not let the title's own crash handler be the first thing that runs.
4. **VK_EXT_device_fault on from the start.** Query counts, then the details, then keep the vendor binary. Test the collector with a deliberate fault before you need it.
5. **Aftermath wired in.** Enable the crash-dump monitor, store the SPIR-V for every shader (with the Aftermath hash), and keep a one-command decoder. Always run the newer page-fault resource lookup.
6. **A way to find the faulting shader.** Shader hash to SPIR-V to source line. We did this by hand once and it took a day.
7. **A native D3D12 reference build with DRED and the debug layer.** Keep GPU-based validation working. It found two real bugs, and DRED would name the address if native ever lost the device.
8. **Strict-mode renderer selection.** An explicit request for Vulkan never falls back. It exits.
9. **A test harness that records state.** Display on or off, focus, driver, commit headroom, driving seconds, lap reached. Reject rows that fail the state check instead of counting them.
10. **A deterministic route and a loss-prone fast reproducer.** Our laptop at 15 fps reproduced in nearly every start. Find yours early and use it for every A/B.
11. **Default-off diagnostics with row tags.** Each run records every switch. Controls match arms except for one change.
12. **Poison fills with positive controls.** If you detect stale reads by filling freed memory with a marker, test the detector first on every format the title uses.
13. **The descriptor-volatility check on day one.** Audit every place the translator copies a descriptor, record-time or otherwise. If a copy is made at record time, give it a submit-time refill. Count how often the title rewrites a slot between record and submit.
14. **A public-source search before the second week.** We found the Aftermath address-reuse caveat, the vkd3d-proton debug options and the descriptor-path driver bugs late.

### Useful public references

- NVIDIA Nsight Aftermath: https://developer.nvidia.com/nsight-aftermath
- VK_EXT_device_fault: https://registry.khronos.org/vulkan/specs/latest/man/html/VK_EXT_device_fault.html
- VK_EXT_device_address_binding_report: https://registry.khronos.org/vulkan/specs/latest/man/html/VK_EXT_device_address_binding_report.html
- D3D12 DRED: https://learn.microsoft.com/en-us/windows/win32/direct3d12/use-dred
- vkd3d-proton README (debug options such as breadcrumbs and descriptor QA): https://github.com/HansKristian-Work/vkd3d-proton
- Root signature 1.1 and volatility: https://learn.microsoft.com/en-us/windows/win32/direct3d12/root-signature-version-1-1
- D3D12 resource binding spec: https://microsoft.github.io/DirectX-Specs/d3d/ResourceBinding.html

---

---
[Back to the index](../README.md)
