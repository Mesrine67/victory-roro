# 0. About this document

## 0.1 What this is
We run an Xbox One D3D12.X game on PC: Windows D3D12, and Linux through Proton/vkd3d-proton (D3D12 on Vulkan). The loader is a shared core that more than one title uses. It provides the console's system and graphics APIs on top of the desktop ones.

Most of our hard problems came from one place: **memory**. On the console the CPU and GPU share one physical memory and one address space. On a PC they do not. This document covers:

- how we bridged that gap;
- the invariants we ended up enforcing;
- the failure modes we hit and how we found them;
- what everything cost, in measured numbers;
- what we would do differently from day one.

It is long on purpose. Every section follows the same pattern: what the mechanism does, why it exists, what we tried and threw away, its exact invariants, how it failed, what it costs, and what we'd change.

## 0.2 How to read the numbers
- All numbers are from our own hardware:
  - an AMD desktop: Ryzen 7 5800X3D, Radeon RX 9070 XT, Linux/Proton;
  - an AMD handheld: Ryzen Z2 Extreme, Radeon 890M, SteamOS/Proton, measured at about 15 W in game;
  - an RTX 4090 desktop on Windows;
  - an RTX 3080 laptop on Windows;
  - a Ryzen 9 5900X desktop with an RX 6700 XT, used as a build and test box.
- Treat them as orders of magnitude and ratios. They are not benchmarks.
- "Slow frame" means a present interval over 33.4 ms, two 60 Hz refreshes. We also use the 16.7 ms and 50 ms lines.
- "Real" slow frames exclude frames our own test harness spent capturing screenshots. Section 9.4 explains why that matters so much.
- Every comparison was made against an A/B noise band. Each arm ran at least twice, with the "off vs off" spread quoted. Anything inside that spread is called noise, however attractive it looks.

## 0.3 Terms
| Term | Meaning here |
|---|---|
| Title | The game executable and its own code. |
| Title memory | Memory the title allocates through the console's VM APIs and writes directly. On the console it is also what the GPU reads. |
| Title VA | An address in the title's view, including addresses it believes are GPU addresses. |
| Desktop resource | The real D3D12 (or vkd3d-proton) resource we create to stand in for a title resource. |
| Span | A contiguous piece of a texture's title memory we track as a unit, usually one mip or one packed-mip tail. |
| Barrier | Our check-and-copy step that runs inside `ExecuteCommandLists`, before the title's command lists reach the GPU. |
| Head-start (HS) worker | An off-thread worker that prepares copies before the barrier asks for them. |
| Need | A span the barrier must have a fresh GPU copy of before this submission can go. |
| Stolen need | A need whose head-start job was queued but not started, so the barrier took it back and staged it inline on the submitting thread. |
| Exact | The rule that the GPU never sees anything other than the bytes the title last wrote (section 5.1). |

## 0.4 Short version
If you read nothing else:

1. **Decide your correctness contract first.** Ours became *submission equivalence*: when a command list reaches the GPU, every texture it can reach holds the bytes the title had written before it called `ExecuteCommandLists`. Every shortcut we tried that broke that contract (placeholders, lower mips, deferred copies, waiting a bit longer) came back as a visible bug: black road, black walls, magenta checkers in a debug fill.
2. **Own the address space from the first instruction.** Reserve the title's fixed ranges before the graphics driver starts allocating. Hook every way memory is allocated, protected, queried or freed, including `GetProcAddress` lookups and kernel-side writers.
3. **Give every GPU resource a generation, not just an address.** The title reuses addresses within microseconds. About 1,000 resources a second were created in our game.
4. **Page-protection traps beat write-watch harvesting** for finding CPU writes, but the fault handler must be written as if it were an interrupt handler.
5. **Your submit-time budget is tiny.** A synthetic 1 ms busy-wait per direct submission cost 42 % of presents; 0.2 ms cost 14 %.
6. **Under Wine/Proton the costs move.** Virtual memory calls take a process-wide lock. `VirtualQuery` can cost hundreds of microseconds. Thread priorities do nothing. In-process `/proc` reads describe the wrong thread.
7. **Instrument before you optimise, and make every instrument an observer.** Most of our "fixes" changed nothing measurable. The ones that worked came from a timer or counter that pointed at the right place.
8. **The GPU was mostly idle** (15-20 % busy on the RTX 4090). The frame limit was the CPU path between the title and the GPU.

---
[Back to the index](../README.md)
