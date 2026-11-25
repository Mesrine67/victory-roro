# Wine / Proton / vkd3d-proton facts that changed our design

If your translator will run on Linux handhelds, these matter from day one. Each one cost us at least one wrong conclusion.

| Fact | Measured / observed | Consequence |
|---|---|---|
| `VirtualAlloc` / `VirtualFree` / `VirtualProtect` take a process-wide VM lock in Wine. | Snapshot scratch allocations alone cost about 2.5 ms CPU per frame on two workers. | Pool every scratch allocation. Count VM calls per frame. |
| `VirtualQuery` is a call into the wineserver. | About 250 µs per call on a sibling title; its readability scan used about 30 % of the main thread. | Keep your own title-memory ledger; use `VirtualQuery` only as a fallback. |
| Thread priorities do nothing. | Every Windows thread of the process at nice 0, `SCHED_OTHER`; max nice priority 0. Only Wine's own internal disk threads at nice 19. | Priority-based levers are placebos. Add workers, cut work, or pin cores instead. |
| In-process `/proc/thread-self` reads go through the wineserver. | They describe wineserver's thread; 0 open failures, silently wrong values. | Sample from outside the process ([tools/linux_proc_sampler.py](../tools/linux_proc_sampler.py)). |
| `GetThreadTimes` ticks at 10 ms. | "Blocked" was overstated per frame. The external sampler showed about 14-15 ms CPU plus 9 ms blocked in slow frames. | Use it for totals only; per-frame splits need an external sampler. |
| QPC is 10 MHz, from the monotonic raw clock. | `CLOCK_MONOTONIC_RAW` ns / 100 equals in-game QPC. | Free, exact join between Linux samples and in-game marks. |
| Wine's crypto provider lacks SHA hardware acceleration. | About 300 ms CPU per second of hashing. | Ship your own SHA-NI path with a bit-identity self-test. |
| Wine waits are precise; Windows default timer resolution is not. | A 34 ms wait ticks at about 21.5 Hz on Windows. | Don't port "frame cap by wait" numbers across OSes. |
| Linux maps a shared zero page for unwritten memory; Windows commits on first read. | Different cost and memory growth for "scan to see if zero". | Avoid reading untouched title memory; track writes instead. |
| Write-combined reads are much cheaper on the Linux handheld than on Windows desktops. | 0.8-1.0 vs 3.2-3.8 ms per MiB. | Allocate title memory cached if you will read it back. |
| Kernel writes (file reads) into read-only pages fail instead of faulting. | `ERROR_NOACCESS` returned to the caller. | Exclude ranges for I/O, track completions in a ledger, turn traps off if an uncovered kernel writer appears. |
| vkd3d-proton treats every root signature 1.0 descriptor as volatile (bindless). | See [23-descriptor-volatility-sources.md](23-descriptor-volatility-sources.md). | The native layer is volatility-correct; don't undo that in your own copies. |
| vkd3d-proton with DXR requested allocates all memory with device-address support. | RenderDoc replays then failed with `VK_ERROR_INVALID_OPAQUE_CAPTURE_ADDRESS`. | Disable DXR for capture runs when the title doesn't need it. |
| vkd3d-proton offers a private tight-alignment flag for small buffers. | Private memory -33 % for us. | Worth probing (hide it from `GetDesc`, fall back on failure). |
| Handheld firmware power limits are not the power actually drawn. | Limits read 35/37/45 W; measured GPU power in game was 15.0 W. | Record measured power for every handheld run. |
| Native Windows ASLR can put the stack/PEB/TEB inside the title's fixed ranges; we never saw it under Proton. | About 1.85 % of Windows launches. | Relaunch on a lost hold; don't assume Proton behaviour on Windows or the reverse. |

---
[Back to the index](../README.md)
