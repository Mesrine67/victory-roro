# Pipelines, shader caches and load times in an Xbox-to-PC translator

This chapter is for an engineer who is building a translator for Xbox Series X titles. It draws on our experience with a translator for Xbox One D3D12.X titles. That translator runs on Windows (native D3D12, or Vulkan through vkd3d-proton) and on Linux (vkd3d-proton under Proton).

The short version: pipeline state objects (PSOs) are the largest single source of boot time and hitching in a translator. Caches fix most of it, but caches also hide bugs, they invalidate themselves in surprising ways, and they ruin A/B tests if you do not control them. Read the "What failed" section before you trust any number you measure.

Throughout, "title A" is the title we tuned most. "Title B" is another title on the same shared core. We do not name them, and we publish no game data.

## 1. How pipeline creation works in such a translator

### 1.1 The path of one PSO

A console title calls a "create graphics pipeline" function on its device object. The call carries a description: shader bytecode for each stage, blend state, rasterizer state, depth state, input layout, render target formats and more. On the console this call is cheap. On a desktop it is expensive, because the driver must compile the shader to GPU machine code.

A translator sits between the title and the desktop API. For each create call it typically does this:

1. **Read the description** from title memory. Validate it. Reject pointers that do not lead to a shader.
2. **Fix each shader stage.** Some console shaders use instructions with no desktop equivalent. These must be translated to something the desktop compiler accepts. Some stages need only a small rewrite. Some need none.
3. **Look up caches.** Is there already a translated shader for these bytes? Is there already a compiled PSO for this exact state?
4. **Call the desktop API.** On native D3D12 this goes to the driver. Under vkd3d-proton it goes to the layer that turns D3D12 into Vulkan, then to the driver.
5. **Return a handle** to the title, and remember the result.

Every step can fail. Steps 2 and 5 produced most of our bugs.

### 1.2 Where the time goes

We measured the split on title B on an RTX 4090 desktop and on an AMD handheld running SteamOS with Proton. These runs were cold, so every pipeline was new to the driver.

| Part of PSO time | RTX 4090 desktop (native D3D12) | AMD handheld (Proton) |
|---|---|---|
| Driver compile | 71 % | 96.5 % |
| Our shader translation | 16 % | 3.5 % |
| Our own cache layer | 13 % | 0 % (off by default under Proton) |

The lesson is plain. The driver compile dominates. Your own translation work is the minority cost. If you spend your first month optimising the translator's shader rewrite, you will have optimised the smaller part.

A second data point from title A: a cold PSO took about 18 ms to create, and a warm one about 0.5 ms. That is a factor of about 36. A title that creates thousands of PSOs turns that factor into tens of seconds.

### 1.3 When titles create PSOs

Titles differ in when they create pipelines, and this changes what helps.

- **Title B creates everything in one burst before the first frame.** About 2,499 pipelines were created at boot and none afterwards, in any menu, load or race. Sessions ran up to 1,154 seconds. PSO work was 72 % of all hitch time on the desktop and 82 % on the handheld, and all of it sat in the boot burst. After boot it was 0 %: zero PSO-related hitches of 100 ms or more, out of 9 on the desktop and 30 on the handheld. Later scene-load stalls in that title were texture uploads.
- **Title A creates pipelines throughout play.** On the handheld, PSO creation was 86 % of hitch time, spread through menus and car selection, not only at boot.

So the first question for a new title is: does it create PSOs up front or on demand? Log every creation with a timestamp. A plot of creation time against run time answers this in one picture. Up-front titles need a warm cache and a faster boot. On-demand titles need pre-building, because no cache can help with a pipeline you have never made.

### 1.4 What vkd3d-proton does with your PSO

On Linux, and on the Vulkan path of Windows, your D3D12 calls reach vkd3d-proton. Its public source shows several behaviours you should know before you design on top of it. Links are in section 6.

- It compiles a graphics PSO's base pipeline at creation time only when the state is complete enough. It skips early compile when, for example, the primitive topology type is undefined or the tessellation patch size is unknown.
- Other variants are keyed by topology, depth format, view mask and sample count. Those variants compile late, at the first draw that needs them. The log then says it is compiling a fallback pipeline late.
- This has a consequence for pre-building. If you replay only the PSO descriptions, you warm the base variant. You do not warm every draw-time variant. Count the "late fallback" log lines per session to measure the gap that remains.
- A compile that the driver defers is treated as non-fatal and retried with a late compile. A real failure returns null and dumps the state to the log.
- Its cache stores the conversion from DXBC to SPIR-V. That is the cheap step. It does not store the driver's machine-code compile, which is the expensive step. A shader-only cache therefore does not remove the dominant cost.
- It uses graphics pipeline libraries (the Vulkan feature that lets a driver compile parts of a pipeline separately) only in narrower cases than you might hope. We found no public switch that forces library mode or asynchronous compile. We could not confirm what it does by default for an ordinary, fully specified PSO.

## 2. The caching layers

A translator has up to five caches stacked on top of each other. You must know which are active in every test.

### 2.1 The title's own cache

Some titles keep a pipeline cache of their own, or use the D3D12 pipeline library interface. Do not assume such blobs are valid on a desktop. Under vkd3d-proton with its internal cache on, the pipeline library object the title sees is effectively a dummy. With the internal cache off, or with a specific config option, the library stores SPIR-V and the driver blob instead.

Microsoft's documentation says a cached PSO can be rejected as driver-incompatible. Your translator must handle rejection as a normal event: evict the entry and create the PSO from scratch.

### 2.2 Your own translation cache

This caches the result of your shader rewrite, keyed by the input bytes. It saves translation time, which is the smaller cost. It is also where one of our worst bugs lived (section 4.2). Key it by a "translation revision" so that a code change invalidates it, and validate every entry on load.

### 2.3 Your own PSO cache

We built a cache of whole PSO results on top of the translator. The outcome was blunt: we closed it as not promoted, because it gave no gain on any backend. Under Proton it made a cold boot about 5 seconds slower on title B (about 36 s against about 31 s, with roughly 7 s of cache cost) and gave no extra warm gain (7.7 s against 7.4 s). The driver and vkd3d-proton caches already do the useful work. Measure before you build this layer.

### 2.4 The vkd3d-proton cache

vkd3d-proton keeps its own cache file. Public documentation and source say:

- By default the file goes in the current working directory. An environment variable sets another directory, and a value of zero disables the cache.
- The file header holds a format version, vendor and device IDs, a build number and a "shader interface key" made of config flags and available extensions. A mismatch rejects the whole file and the library starts empty. The source we read has no driver-version field.
- Entries carry checksums. A truncated entry stops parsing but earlier entries survive. Writes happen on a separate worker thread, and the write file is merged into the read file at the next start.

The default directory matters more than it looks. Under Proton the game folder is the working directory, so a naive setup can put the cache somewhere that is wiped or read-only. Point it at a stable, writable, per-title directory.

### 2.5 The driver caches

Each driver keeps its own disk cache, and these are the layers that matter most for load time.

- **NVIDIA:** the shader disk cache is tied to the driver version. A driver update means a full recompile. Its path and size are set by environment variables, and an oversized cache may be wiped by a run that lacks the variable.
- **Mesa (AMD on Linux):** the cache lives under the user's cache directory with a default maximum size of 1 GB. A large pre-build can exceed that and evict itself. Raise the size limit for titles with thousands of pipelines.
- **Windows drivers** keep their own caches in places you do not control.

Every driver or Proton update can make the next launch cold. This is normal, and your design must expect it.

### 2.6 What the layers did for us

The cleanest measurement of caching came from title B, launched twice with the same private cache directories kept between launches.

- **AMD handheld, Proton:** cold boot, PSO burst 31.1 s (26.0 s of it driver). Warm boot, burst 0.8 to 1.2 s. Time to first steady frame fell from about 38 s to about 7.4 s. That is about 97 % less PSO time and about 31 s faster to first frame. All of it came from the vkd3d-proton and Mesa caches (4,018 of 4,018 shader hits).
- **RTX 4090 desktop, native D3D12:** PSO burst 69.3 s cold, 5.5 s warm (-92 %). Boot to first steady frame about 73 s cold, about 10 s warm.
- **RTX 4090 desktop, Vulkan through vkd3d-proton on Windows:** 83.3 s cold, 76.1 s warm (-9 %). The driver compile stayed at about 73 to 79 s even warm. We did not explain this. The NVIDIA Vulkan driver cache was not ours to clear, so we could not isolate it.

Read those numbers with care. The desktop cold figures were about six times larger than earlier cold boots of the same title (11.6 s), because another compile job ran on the same machine and the windows were occluded. We trust the warm-to-cold ratio, not the absolute seconds.

The practical result: on the handheld, simply keeping the vkd3d-proton and Mesa caches between launches gave nearly all the available benefit, with no pre-build at all. Do this first. It costs nothing.

## 3. Pre-building PSOs before they are needed

For titles that create pipelines during play, a warm cache from the second launch is not enough. The first time a player reaches a new car, track or weather state, the title creates PSOs the cache has never seen. The fix is to create them early, at launch, before the title asks.

### 3.1 The recorder and replayer design

We built two cooperating pieces in the shared core.

1. **A recorder.** While the title runs, it writes a journal of every PSO creation: the description, the shader hashes, and enough state to recreate the call. It writes a "begin" line before the driver call and an "end" line with the result after it.
2. **A replayer.** At launch, it reads the journal and creates each recorded PSO on a pool of worker threads, with no window and no draw. The driver compiles each pipeline and fills its own cache. When the title later asks for the same pipeline, the driver serves it warm.

Properties we ended up needing:

- **Identity check.** The journal is tied to the translator revision, the GPU, the driver and the cache directory. If the identity and the corpus are unchanged, the replay is skipped. A growing journal replays only the new part (0.06 s warm in our test). A different cache directory counts as a different identity.
- **Checkpoint and resume.** We cancelled a replay at 40 % (1,790 done), relaunched in the same cache directory, and it resumed at 1,790 and replayed the other 2,648 in 0.64 s.
- **Cross-machine journals.** A journal recorded on one machine replayed on another with zero failures. The records are device-independent: we found 3,748 identical records between an NVIDIA desktop and an AMD handheld. The driver does the device-specific compile.
- **A crash guard.** A hang inside the driver cannot be killed from within the process. We persist a "pass started" marker. If two passes end unfinished, replay turns off for seven days, then gets exactly one retry. The relaunch counter covers the hang case.
- **Corpus hygiene.** Orphan journals from older translator revisions are pruned, and a size cap of 1 GiB applies. The journal directory is per title.
- **A progress window only for real work.** We show it only when at least 128 recipes are still running after half a second. A warm launch shows nothing.

### 3.2 Measured effect

All of these measure process start to race start, with the prepare step included, on the same save and a scripted route. Each arm ran on one device.

| Device | Pre-build OFF (cold) | Pre-build ON (cold) | Change |
|---|---|---|---|
| AMD handheld (Proton), final build | about 159 s | about 113 s (three runs) | about -29 % |
| RTX 3080 laptop (Windows, Vulkan), earlier build | 233.2 s | 138.4 s (prepare 32.4 s, 4,416 of 4,416) | about -41 % |
| AMD desktop (Windows, Vulkan), earlier build | 336.4 s (mean of 341.6 and 331.1) | 141.6 s (prepare 39.2 s, 4,416 of 4,416) | about -58 % |

Other facts from the same work:

- On the handheld the final build replayed 6,867 of 6,867 recipes in about 21.4 to 21.8 seconds with zero failures, in all three runs.
- A recorder-only arm (recording, with an empty replay list) took about 164 s on the handheld, against 159 s for OFF. The recorder costs about 5 seconds in that setup. This was a single arm.
- Warm launches are neutral. On the laptop, warm OFF took 105.8 s and warm ON took 108.3 s and 107.8 s.
- After a pre-build, only about 7 s of a roughly 100 s warm boot-to-race was attributable to our loader. The rest was the title's own timeline and the scripted route. No cheap loader-side load-time fix was left in that measurement.
- On the real default play path under Proton, a "preparing" window showed for about 17 s (9.1 s to 26.3 s) for a 6,867-recipe journal. The relaunch skipped the replay and showed no window.
- An earlier forced replay on the handheld created 5,096 PSOs in 15.1 s with no failures, from a journal of 4,470 compatible recipes.

Caveats that matter. Each cold-ON figure outside the handheld is one run on an earlier build. Every journal came from the same scripted route, which is the best case. A real first launch has no list, and a player's own play covers different pipelines. For a real player, the first launch is a plain cold launch. The benefit starts on the second.

We shipped it default ON on the Vulkan renderer only. Native D3D12, test builds and other titles stay OFF until they have their own corpus. One switch turns it off, and the off switch also stops the recorder.

### 3.3 Pre-building without recording a play session

Recording needs a person or a script to play. For a title with many cars, tracks and weather states, one route covers a small fraction of the space. We studied one title and found about 20,800 PSOs and 78,000 shaders in total, with one route covering about 15 % of the pixel shaders. Static guessing from the data files recovered only about 9 % of the used pipelines.

We therefore asked: does any public system get a complete list of pipelines without recording play? Our survey found none.

- **Microsoft Advanced Shader Delivery** captures a title's pipeline state into a database by recording a play session through a recorder tool, compiles it offline with vendor plug-ins, and ships the precompiled result with the game. A published example drops a load from about 1.5 minutes to about 4 seconds. It depends on drivers that ingest those databases, so it does not help a vkd3d path.
- **Steam's shader pre-caching** uses Fossilize recordings that real players made. The list comes from play.
- **Dolphin** removed compile stalls with "ubershaders": one generic shader that handles any configuration, used while the specialised shader compiles. It works because the GameCube GPU is fixed-function, so the state space is small. A title with arbitrary shaders cannot use that trick.
- **RPCS3** added shader interpreters that render while the real shader compiles. Its cache fills as you play.

The pattern is consistent. Full coverage without play comes from only two sources: a fallback that hides the miss, or recordings shared by other players. No public system enumerates pipelines from the data files. Microsoft's design (record once into a hardware-independent database, compile later per GPU) is the same shape as our journals.

The practical routes, ranked by our review:

1. **An unattended crawler.** A script steps through the car store or garage, loads each track once with a short dwell, and cycles weather and time of day, with the recorder on. It never drives a lap. Scripted laps proved unreliable (cars hit barriers), so the design uses menus, car selection and free-play loading only. Split the work across machines, then merge into one deduplicated, versioned list. We designed this and ran smoke tests. The first smoke test failed because of harness faults: zero cars verified, zero track loads, and the recorded recipes were quarantined. It is not yet proven. A tolerant text matcher for the car catalogue did match all 18 confirmed cars in an offline replay.
2. **A fallback while a PSO builds.** Skip or substitute rather than stall. This is a safety net, not a replacement, and it can show wrong visuals.
3. **Vulkan-level recording** with a layer, replayed with a Fossilize-style tool. It duplicates D3D12-level journals and couples to the vkd3d version. We rate it low.
4. **One cheap experiment:** give recorded PSOs an unknown depth format and see whether vkd3d-proton then routes them through pipeline libraries that compile faster. We have not run it.

One privacy rule for any shipped corpus: the merged journal contains game-derived data, so it stays private. Each player records their own.

## 4. What failed

This section matters most. Most of these cost us weeks, because the symptom pointed somewhere else.

### 4.1 A stub that returned success with a null result

A console device function that creates a "cached PSO" was implemented as a stub that returned success and wrote a null handle. One title's cached factory stored that null. Its bind helper then skipped the native setter because the object was null, so the previous compute pipeline stayed bound at the next draw. The AMD driver faulted inside that draw after about 255 seconds. After the fix the same run stayed clean for 280 seconds.

The symptom is the point. A null PSO does not crash at creation. It crashes later, in a draw, inside a driver, with a stale pipeline. Log every call to every stub. If a title uses an entry point you stubbed, you silently get null objects. We fixed that title with a title-specific switch that routes to the title's other full creation path. A generic fix (build the PSO from the cached blob) was not written.

### 4.2 "Could not parse" cached as success

In our translation path, the function that rewrites console-specific instructions returned zero for two different cases: "nothing needed rewriting" and "this is not a shader at all". The caller treated zero as success and wrote a positive cache record. Of 3,666 kept records, 1,178 were such non-shader payloads. Elsewhere in the same title, 672 of 703 unique captured failure blobs lacked a valid shader container header, and 121 of 254 saved shader files in a shared failure folder were not valid containers.

We still do not know where those bytes come from. They match nothing in the shader archive we hold, and no driver pipeline blob explains them. The fix was to validate at the handoff, so bad bytes stop at the driver call. Draws that use them render nothing, under a placeholder policy.

The general rule: a translator that cannot tell "nothing to do" from "not valid input" will cache garbage as valid. Return distinct results. Treat bad bytes as a bug in whoever handed them over.

A related trap: a desktop shader container can parse and disassemble cleanly yet contain no executable program, because removing custom chunks left only declarations. Count functional programs separately from container hits. Do not count byte patterns found inside compressed data as shaders.

### 4.3 Failed PSOs retried again and again

A test run of a Direct3D 11 console title on the native D3D12 backend drove at only 1.4 to 6.6 fps. The log showed 2,716 failed PSO creations that were retried on demand, 42,142 placement-buffer fallbacks, a whole-buffer compare and copy on every submit, synchronous submits, and about 13 MB of hot-path logging in five minutes. The failed PSOs probably explain the missing shadows and ambient occlusion the player saw, but we did not prove that link.

The fix idea was to cache failure keys so a known-bad PSO is not retried. The general advice: count failed creations per run, check that failures are cached as failures, grep logs for per-draw lines, and check that the launcher does not run the game at below-normal priority.

### 4.4 Cold start crash, warm start fine

On a native D3D12 backend, a rebuilt run with empty caches exited with an access violation after 29 seconds. It had made 1,381 PSO begin lines and 58 failed ends, with one unmatched graphics creation. The warm retry made 1,948 PSO calls with zero failed ends and ran to its planned 320-second timeout. We never diagnosed it, and we made no speculative repair. The lesson: cold and warm are different code paths in practice, and a warm-only test suite will miss shader-path bugs.

### 4.5 A warm driver cache produced a black picture

On the RTX 4090 desktop, native D3D12 with a warm disk cache (228 hits) drew a black picture after loading. A cold cache, or the cache switched off, rendered. AMD was unaffected. Later 4090 runs were black even with the cache off. So the cache correlation explained a morning of runs and not the later ones. The cause stayed unproven. The cache files passed header and checksum tests, so we call them "unproven-corrupt", not corrupt. We advise against clearing driver caches or rolling back code on that evidence alone.

### 4.6 Hidden cold versus warm differences ruined measurements

This is the failure we most want you to avoid. Three separate episodes:

- **A bisect that blamed the wrong change.** The first old-versus-new comparison had a hidden difference: the first run populated a local cache. A half-split bisect that also reused the warmed cache was black and pointed at the wrong code. Changing only the cache selection on the full binary then showed the real driver: one build was black when warm and fine when cold. Rule: state the cache temperature of each arm and use identical environment files before you blame a code change.
- **A pre-build A/B that understated the saving.** Our first draft of the laptop OFF figure was about 199 to 201 s. It used vkd3d-proton's default cache, which was warm. With an explicit, identical, fresh cache path in both arms, OFF was 233.2 s. The saving grew from about 30 % to about 41 % once the hidden warm cache was controlled. Rule: set the cache path explicitly in both arms and wipe it for cold arms.
- **A recorder cost that was not real.** We saw an apparent recorder cost and tried batching the journal flush. It had no effect, and the apparent cost turned out to be the same cache-path confound.

The same lesson applies to title B's earlier boot numbers: some of its "cold" logs were probably partly warm.

### 4.7 A scheduled skip mistaken for a natural failure

During a geometry investigation, a piece of scenery vanished in one frame, and we used it as evidence of a missing draw. It was a programmed diagnostic skip: our own test code had deliberately skipped a shader at that frame. Exclude diagnostic-skip frames from "natural" evidence. Log every injected skip, with its frame number, in the same file as the observations.

### 4.8 Bookkeeping inside a time budget starved the real work

A texture and pipeline service loop had a per-frame time allowance. Inside that allowance, it scanned the entire pending list. As the list grew, fewer items fit inside the allowance and top-ups lost service. Use indexed membership and service only due work. A later review found this defect remained after an earlier latency gain.

### 4.9 A carriage return that turned switches off

A launcher script for a Linux handheld was generated by a Windows tool. Windows output turned every newline into a carriage-return pair, so every exported value ended with a carriage return. On/off switches compared to "1" read as off. Paths pointed nowhere. In a cold pre-build test, the recorder and the replay both read as off. The private cache directories stayed empty and the log file was never written, so the run measured nothing about the feature. The loader's startup census showed the recorder switch holding "1" followed by a carriage return.

Rule: print the effective value of every switch at startup, with quotes, and check the printout against the intended configuration before reading any result.

### 4.10 Silent invalidation

Journals recorded by other translator builds loaded as zero records with no log line, because the translation revision had changed. A 240 MB journal of 9,342 recipes was silently ignored. We now prune such orphans and log them. Any cache keyed on a revision must say so loudly when it discards entries.

### 4.11 Layouts that must be proved independently

A first review of sampler state confused an internal enumeration with a public border colour. Release and debug builds of one title used different record strides and type numbers for an extension block in the PSO description, and partial logging omitted the semantic bytes. When you parse a title's extended state, prove each layout against the build you are running, and include the semantic bytes in the effective identity of the PSO. Two PSOs that differ only in those bytes are different pipelines.

### 4.12 Optional shared-core policies that regressed another title

A set of optional scheduling policies that helped one title gave no readiness benefit in another, and worsened some low-percentile frame times. Strict backing checks failed boot before a route window. Carry per-title defaults with each title, and compare more than one scenario. Single samples support compatibility decisions, not claims of small performance wins.

### 4.13 GPU events we cannot attribute

On the AMD handheld, three GPU events occurred during pre-build testing: one with pre-build OFF (device removed at about 897 s), and two with recording and pre-build on, one ending in a full reboot and one with a page fault and ring timeout at race start. Each was a single event. Four later cold-ON runs with a 6,867-recipe journal did not reproduce them, and no arm in the final-build matrix showed a fault. One run each is not proof of absence.

Public reports point the same way. We found no public report that blames parallel pre-compilation itself for page faults or device loss. There are open reports of page faults during heavy streaming on recent AMD parts, and a driver-vendor-acknowledged bug where creating an unused compute pipeline can break an earlier, correct dispatch on some NVIDIA GPUs.

## 5. Offline tools that helped

Three techniques found problems without running the title.

1. **An offline harness on the software rasterizer (WARP).** It takes captured shader bytes, runs the translator's span over them, and creates the PSO on the WARP adapter with no queue and no dispatch. Run one shader per process, because a fail-fast inside the D3D12 core kills the process. It reproduced a native crash, a generic failure and an invalid-argument error on integer-target blending, and showed the fixed versions create cleanly.
2. **A PSO flight recorder.** It writes a begin line (root signature hash, flags, cache hit or miss, a hash of each stage) before the driver call and an end line with the result after it. A true fail-fast bypasses structured exception handling, so the unmatched begin names the crashing PSO. Use creation id and thread id, because concurrent creations leave several unmatched begins.
3. **Isolated-stage graphics replay.** Useful, but a failure here is a probe failure, not proof that the shader is bad. In one run, 133 invalid-argument results came from the probe's limits.

## 6. Public sources worth reading

We checked each link. All returned HTTP 200 except one marked.

- vkd3d-proton README, cache source, state source and changelog:
  - https://raw.githubusercontent.com/HansKristian-Work/vkd3d-proton/master/README.md
  - https://raw.githubusercontent.com/HansKristian-Work/vkd3d-proton/master/libs/vkd3d/cache.c
  - https://raw.githubusercontent.com/HansKristian-Work/vkd3d-proton/master/libs/vkd3d/state.c
  - https://raw.githubusercontent.com/HansKristian-Work/vkd3d-proton/master/CHANGELOG.md
- Fossilize (Valve's pipeline recorder and replayer), its Linux replay source, and a memory-use issue:
  - https://github.com/ValveSoftware/Fossilize
  - https://raw.githubusercontent.com/ValveSoftware/Fossilize/master/cli/fossilize_replay_linux.hpp
  - https://github.com/ValveSoftware/Fossilize/issues/312
- Microsoft Advanced Shader Delivery and PSO return values:
  - https://devblogs.microsoft.com/directx/agility-sdk-1-618/
  - https://developer.microsoft.com/games/articles/2026/03/gdc-2026-advanced-shader-delivery-on-windows
  - https://github.com/MicrosoftDocs/win32/blob/docs/desktop-src/direct3d12/d3d12-graphics-reference-returnvalues.md
- Robust pipeline cache validation: https://about.roblox.com/newsroom/2021/02/creating-a-robust-pipeline-cache-with-vulkan
- Unreal's PSO precompile conventions, and a forum thread on why its "remaining" count is unreliable:
  - https://docs.unrealengine.com/4.26/en-US/SharingAndReleasing/PSOCaching/PSOReference
  - https://forums.unrealengine.com/t/pso-precache-loading-screen/2658405
- Mesa environment variables and GPU hang debugging:
  - https://docs.mesa3d.org/envvars.html
  - https://docs.mesa3d.org/drivers/amd/hang-debugging.html
- Emulator approaches to compile stalls: https://github.com/RPCS3/rpcs3/pull/8106 and the Dolphin ubershader write-up at https://dolphin-emu.org/blog/2017/07/30/ubershaders/ (403 to our bot check; not auto-verified).
- Open reports relevant to risk:
  - https://github.com/HansKristian-Work/vkd3d-proton/issues/2906
  - https://forums.developer.nvidia.com/t/windows-vulkan-unused-compute-pipeline-causes-device-loss-on-rtx-4090-4070-super/383848
- DXVK README: https://raw.githubusercontent.com/doitsujin/dxvk/master/README.md
- Steam's shader pre-caching beta coverage: https://gamingonlinux.com/2020/05/steam-beta-adds-vulkan-shader-processing

## 7. Design lessons from public pre-compilers

We read how Fossilize replay works, because it is the closest public design to a translator's pre-build.

- It runs replay as separate child processes, one thread each. The parent restarts a crashed child past the pipeline that crashed it, and passes a ban list of crashing shaders to later children. Children die with the parent.
- On Linux the parent also throttles. About once a second it reads I/O pressure and dirty pages. When dirty pages rise by 50 MiB or I/O stall passes 30 %, it pauses one child. It resumes one when dirty pages fall by 10 MiB or the stall drops under 10 %.
- It has no overall memory cap. Public reports show workers using 2 to 2.4 GB each. One report has 19 workers using about 51 GiB and killing a desktop session. Another has a game on a 32 GB machine running out of memory.
- Steam's own default is a quarter of the logical cores at idle priority.

The same thought applies to your replayer. Cap threads by free memory, not only by core count. On a 16 to 32 GB handheld, start at about four threads and adapt. A child process isolates a CPU crash but not a GPU reset, because a GPU reset on AMD hits every context. That is why the replay runs before the window opens, so the loader can recover and relaunch.

Unreal's convention is useful for user experience: compile fast behind a loading screen, compile quietly behind menus, pause during gameplay, resume on menus. Its forum also shows that the "remaining count" never reaches zero reliably. So use your own manifest to decide whether to run the pre-build. Do not infer it from the engine.

## 8. Day-one advice

1. **Log every PSO creation from day one.** Timestamp, thread, shader hashes, result, cache hit or miss, and duration. This tells you whether creation is a one-off boot burst or spread across the session.
2. **Make the cache path explicit in every run, and print it.** Wipe it for cold arms. Keep it for warm arms. Put the cache temperature in the test name.
3. **Before any A/B, run OFF against OFF three times.** If two identical arms differ, you have a hidden variable. Fix that first.
4. **Print the effective value of every switch at startup,** quoted. This catches carriage returns, wrong defaults and silent overrides.
5. **Never let one return value mean two things.** Separate "unchanged", "translated" and "invalid input". Validate shader containers at the handoff and stop bad input before the driver.
6. **Log every stubbed device function.** A stub that returns success with a null output is the worst kind of bug: it fails later, elsewhere.
7. **Cache failures as failures,** with a limit on retries. Count failed creations per run.
8. **Do the zero-code test first.** Launch twice with one retained driver and vkd3d-proton cache directory. On our handheld this removed about 97 % of PSO time in a boot-burst title. Only then build a pre-build system.
9. **Decide by creation pattern.** A boot burst needs warm caches. On-demand creation needs a recorder and a replayer.
10. **Tie journals to a revision, an adapter and a driver.** Prune orphans loudly. Cap their size.
11. **Make replay survivable.** Log the PSO identity to disk before each create. Persist a "pass started" marker. Disable after repeated unfinished passes. Resume from a checkpoint. Cap threads by memory.
12. **Ship pre-build default ON only where you have proved it,** and keep a one-line rollback. We proved it on one renderer path and kept the others OFF.
13. **Expect a cold launch after every driver or Proton update.** Show a progress window only when real work remains, and offer skip.
14. **Do not build a PSO cache of your own until you have measured the gain.** Ours gave none.
15. **Mark every diagnostic skip in your logs,** so a scheduled skip is never mistaken for a natural fault.
16. **Count the late fallback compiles** in vkd3d-proton per session. They show how much a pre-build still misses.
17. **Keep corpus data private.** A recorded journal holds game-derived shader data. Do not commit or ship it. Let each player record their own.

## 9. Open questions we did not close

- Whether the NVIDIA Vulkan path really stays slow when warm, or whether our setup hid a cache we did not control.
- Whether a real first launch with an empty list, followed by a second launch, matches the scripted-route gains. We measured the scripted case.
- Whether pre-building is safe on every GPU. The device-loss events we saw were single events and are unexplained either way.
- Whether a crawler can reach full coverage of cars, tracks and weather without a human.
- Which of the public pipeline-library behaviours apply in the version of vkd3d-proton you ship.

If you take one idea from this chapter, take this: measure cold and warm separately, and prove that your two arms differ only in the thing you changed.

---
[Back to the index](../README.md)
