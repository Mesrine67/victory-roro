# A Vulkan GPU timestamp layer (design, so you can build your own)

When the vendor GPU profilers need admin rights or a counter permission, a small **explicit Vulkan layer** still gives per-command-buffer GPU times. Ours was about 550 lines of C++. Its source is not in this repository: it was a throwaway tool and was not kept. What follows is enough to rebuild it in a day.

## Why
- Under vkd3d-proton, every D3D12 command list becomes a `VkCommandBuffer`, and every `ExecuteCommandLists` becomes a `vkQueueSubmit`/`vkQueueSubmit2`. A layer below vkd3d-proton therefore sees the title's lists 1:1, plus vkd3d's own init and transition buffers.
- It needs no admin, no GPU performance counters and no driver setting. It is enabled per process by environment variables only.
- Its first answer settled a lot: the RTX 4090 was busy only 15-17 % of each frame. Our exact texture path added no GPU work. The cost was CPU time before the GPU got work.

## Enabling it (explicit layer, nothing system-wide)
```
VK_ADD_LAYER_PATH=<folder with the layer JSON manifest and DLL/.so>
VK_INSTANCE_LAYERS=VK_LAYER_YOURNAME_gputime
YOURLAYER_OUT=<output file prefix>
```
**Gotcha:** on Windows, an *elevated* process (for example one started from an elevated remote shell) ignores layer environment variables, so the layer silently doesn't load. Start the game unelevated.

## Mechanism
1. **Intercept** `vkCreateDevice` (note each queue family's capabilities and `timestampPeriod`), `vkGetDeviceQueue*`, `vkQueueSubmit`, `vkQueueSubmit2` and `vkQueuePresentKHR`.
2. **Per queue**, keep a ring of timestamp queries (we used 4,096) and a pool of tiny pre-recorded command buffers. Each tiny buffer resets one query and writes one timestamp.
3. **Rewrite each submit batch** on a graphics or compute queue as:
   ```
   [TOP_OF_PIPE ts]  cb1  [BOTTOM_OF_PIPE ts]  cb2  [BOTTOM ts] ... cbN [BOTTOM ts]
   ```
   That gives the GPU start and end of the batch, and the GPU time of each app command buffer (the difference between consecutive timestamps).
4. **Transfer-only queues** cannot reset queries in a command buffer on all drivers. Log their submits on the CPU side only.
5. **CPU-GPU clock mapping.** Every N presents (we used 32), call `vkGetCalibratedTimestampsKHR` (`VK_KHR_calibrated_timestamps`, previously the EXT) for a (GPU tick, QPC or monotonic) pair. Map CPU submit and present times onto the GPU timeline from those pairs.
6. **Join to the game's own events.** Log a cheap CPU tick (`GetTickCount64` or QPC) next to your loader's frame or lap marks. The analysis can then label each GPU frame with lap and lap-seconds.
7. **Read results back lazily.** Call `vkGetQueryPoolResults` without waiting, a few frames later, for slots whose batch fence has completed. Write plain text lines (`S` submit, `C` command buffer, `P` present, `K` calibration).
8. **Optional pass mode.** Put a TOP/BOTTOM pair around every `vkCmdBeginRendering` / `vkCmdBeginRenderPass*` *inside* vkd3d's command buffers. That gives per-render-pass GPU time. Only worth it once you know the GPU is the limit.

## Analysis
Per frame (present to present):
- **busy** = the union of command-buffer intervals;
- **span** = first start to last end;
- **idle** inside the span, and **idle before the first big list** (0.2 ms or more);
- command buffer counts by size class (under 0.05 ms, 0.05-0.2 ms, 0.2-1 ms, 1 ms and up).

## Costs and cautions (measured)
- About **19-21 % fps** in CPU-bound stretches. Likely cause: two or more extra command buffers per batch raise the per-submit cost on vkd3d's queue thread, which also presents.
- **GPU busy times are real.** Frame times and idle shares are inflated. Compare arms with both traced.
- **One traced run lost the device and its untraced twins did not.** Add a light mode (begin/end marks per batch, or only the first and last batch of each frame) before tracing configurations that are already fragile. Never treat a device loss in a traced run as evidence about the game or translator.

## Alternatives we checked
| Tool | Status for us |
|---|---|
| Nsight Graphics GPU Trace | Needs "GPU performance counters: allow all users" (NVIDIA Control Panel, developer settings). Default is admin-only. |
| Nsight Systems | Admin to install and run on Windows. |
| PresentMon / ETW frame tools | Admin or Performance Log Users. |
| vkd3d-proton queue profiling | CPU-side only. |
| RGP (AMD, Linux via RADV) | Works without root; see [12-gpu-profiling.md](12-gpu-profiling.md) and [tools/](../tools/). |
| RenderDoc | Per-draw timing, with pitfalls under vkd3d-proton; see [12-gpu-profiling.md](12-gpu-profiling.md). |

---
[Back to the index](../README.md)
