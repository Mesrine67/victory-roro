# 12. GPU-side profiling: what worked and what didn't

## 12.1 A Vulkan timestamp layer (worked)
Under vkd3d-proton everything becomes Vulkan, so a small **explicit Vulkan layer**, enabled only by environment variables, can time the GPU without admin rights or GPU performance counters.
- It writes timestamps at the start and end of every `vkQueueSubmit` batch and every command buffer.
- It maps presents, CPU submit times and the loader's frame marks onto the GPU clock.

**Results on the RTX 4090:**
- the GPU was busy only **15-17 %** of each frame traced (about 17-18 % untraced);
- most of the idle came *before* any work had been submitted: 8-10 ms of idle before the frame's first list of 0.2 ms or more;
- the exact path added no GPU work, but at lap start it made that idle gap 2.3 ms longer.

**Costs and cautions.**
- About 19 % fps in CPU-bound stretches. GPU times are real; frame times and idle shares are inflated.
- A traced run lost the device and the matching untraced runs did not. Keep a "batch begin/end only" light mode, and never trust a device-loss result from a traced run.

## 12.2 Radeon GPU Profiler on AMD Linux (worked, with limits)
RADV, Mesa's Vulkan driver, can write RGP thread traces through its built-in capture. We set the trace mode to RGP with a trigger file, a large trace buffer and queue events on. A small watcher touched the trigger at chosen moments of the run, for example 6 s and 14 s into each lap, to get one-frame captures.

- **Headless:** the trace file's chunks can be parsed for queue event timings and the GPU clock. That gives per-command-buffer GPU start and end per captured frame, with no GUI.
- **Per draw:** the SQTT token stream (per draw, barrier and layout transition) has no open decoder we could find. Per-draw and per-pass cost needs the RGP GUI.
- **Distortions:** capture slows the run a lot (about 44 → 21 fps), and RADV fixes a stable GPU clock state during capture. GPU busy time is valid like-for-like; CPU-side idle is inflated.

**Findings on the handheld.**
- Fix on and fix off had the same GPU busy time (22.6 against 22.5 ms) and the same command buffer count (50 against 51).
- The difference was a **9-26 ms GPU-idle gap before the title's first big render list** with the fix on.
- Lap starts carried 60-263 tiny upload command buffers per frame in both arms. Those are upload storms.

All of this said: **the GPU is not the problem; the CPU path to the GPU is.**

## 12.3 RenderDoc (useful per draw, but has pitfalls under vkd3d-proton)
RenderDoc gives per-draw timing and pixel history in a capture, which no other tool gave us. Under vkd3d-proton we hit all of these problems:

- **It changed the reported GPU vendor.** With RenderDoc loaded, our loader saw a different vendor, and our vendor-specific guards switched themselves off. The captured frames therefore ran *a different code path* from the one with the bug. Pin vendor-dependent policy explicitly in capture runs, or log it in every row.
- **It captured the wrong frames.** Presents counted with capture active did not line up with the moments we meant. Several captures were of the menu before the race, not the frames with the bug. Each captured frame took about 5.9 s. Trigger captures from an in-game event you log, not from a present number planned in advance, and check a thumbnail before analysing.
- **Replay failures with ray tracing enabled.** vkd3d-proton allocates all device memory with device-address support when DXR is requested. Captures then failed to replay with `VK_ERROR_INVALID_OPAQUE_CAPTURE_ADDRESS` on the same machine and driver. Disabling DXR in vkd3d-proton for the capture run is the thing to try.
- **Device losses.** Two of two Vulkan-level capture runs lost the device during capture.
- **Which layer you capture.** Calling the in-application capture API with null device and window captured the D3D12 layer *above* vkd3d-proton. Replaying that needed the matching D3D12 redistributable and developer mode. To capture at the Vulkan level, hand RenderDoc vkd3d-proton's `VkInstance` and device, which vkd3d-proton exposes through its device extension interface.
- **Small traps.** Over an elevated remote shell on Windows, the Vulkan loader ignores layer environment variables, so the capture layer never loads. RenderDoc's Python host blocked on its first-run analytics prompt until that was answered in its config.

## 12.4 NVIDIA tools
- **Nsight Graphics GPU Trace** fails with "GPU performance counters unavailable" unless counter access is allowed for all users. The NVIDIA Control Panel calls this *developer settings → manage GPU performance counters*. The default is admin-only. That is a machine-level setting for the owner to change, not something to script past.
- **Nsight Systems** needs admin to install and run on Windows.
- **PresentMon** and similar ETW-based frame tools need admin, or membership of the Performance Log Users group.
- **vkd3d-proton's queue profiling** is CPU-side only.

The timestamp layer (section 12.1) was our only option that avoided all of these.

## 12.5 What to take from this
- Build a **GPU busy against wall clock** measurement on day one. It tells you whether to work on the GPU or the CPU. For us it was CPU, by a wide margin, on every machine.
- Per-command-buffer timing is enough for most decisions. Use per-draw tools only once you know which pass to look at.
- Every capture tool changes timing, and some change *behaviour* (vendor ID, memory allocation flags). Write down what each changes before trusting a capture.

---
[Back to the index](../README.md)
