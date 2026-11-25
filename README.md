# Xbox-to-PC translator notes: memory, textures, draws and performance

What we learned building a translator that runs an Xbox One D3D12.X game on PC: Windows D3D12, and Linux through Proton/vkd3d-proton. It is written for people building the same kind of thing, in particular an **Xbox Series X** translator.

It covers how we bridged the console's unified CPU/GPU memory to PC heaps and GPU addresses, detected the title's CPU writes to GPU-visible data, staged texture bytes to the GPU *exactly* before each submission, handled draws, descriptors and lifetime, and then made all of that fast enough. It includes the dead ends, the measured costs and the tools.

**What is not here:** game data of any kind (no assets, shaders, addresses or dumps), console SDK material, or our loader's source code. The design is described, not shipped.

## Start here
- **The whole write-up as one page:** [xbox-memory-translation-notes](https://github.com/TPepperoni666/xbox-memory-translation-notes).
- **The short version:** [docs/00-about.md](docs/00-about.md), section 0.4 ("Short version").
- **If you are starting a Series X translator:** read [14-series-x.md](docs/14-series-x.md) and [15-day-one.md](docs/15-day-one.md), then come back to the mechanisms.

## The write-up (one topic per file)
| # | Topic |
|---|---|
| 0 | [About, terms, short version](docs/00-about.md) |
| 1 | [The memory model gap: console vs PC](docs/01-memory-model-gap.md) |
| 2 | [Owning the title's address space: reservations, VM hooks, size-0 frees, write-combining, the ledger](docs/02-address-space.md) |
| 3 | [GPU resources over title memory: title VA ↔ GPU VA, generations, buffers, textures](docs/03-gpu-resources.md) |
| 4 | [Detecting CPU writes: write-watch vs page traps, the fault handler, retire, the oracle](docs/04-write-detection.md) |
| 5 | [Exact texture staging: the contract, the barrier, snapshots, leases, the head-start worker](docs/05-exact-staging.md) |
| 6 | [The black pop-in history and what it taught us](docs/06-black-pop-in-history.md) |
| 7 | [Where draws meet memory: draw reach, bundles, descriptor bookkeeping, volatility, hook costs](docs/07-draws.md) |
| 8 | [Residency, lifetime and device loss](docs/08-lifetime-device-loss.md) |
| 9 | [Performance lessons: submit budget, three slow-frame classes, capture frames, workers, the burst lock, Wine](docs/09-performance.md) |
| 10 | [Worked examples: one texture's life, one streaming slow frame, one burst slow frame](docs/10-worked-examples.md) |
| 11 | [The instruments we built](docs/11-instruments.md) |
| 12 | [GPU-side profiling: timestamp layer, RGP, RenderDoc pitfalls, NVIDIA tools](docs/12-gpu-profiling.md) |
| 13 | [Measurement discipline](docs/13-measurement-discipline.md) |
| 14 | [What changes for Xbox Series X](docs/14-series-x.md) |
| 15 | [If we started again: day-one list](docs/15-day-one.md) |
| A | [Switch names used as examples](docs/A-switches.md) |
| B | [Numbers worth remembering](docs/B-numbers.md) |

Cross-references like "section 5.4" refer to the numbered headings in these files: section 5.4 is in file 5.

## Reference material
| Document | What it is |
|---|---|
| [20-tried-and-ruled-out.md](docs/20-tried-and-ruled-out.md) | The experiment ledger: nearly 100 A/B attempts, their results and what each one rules out. |
| [21-measurement-method.md](docs/21-measurement-method.md) | How we ran A/B rows, joined Linux samples to in-game marks, split frames, and avoided small-n traps. |
| [22-gpu-timestamp-layer-design.md](docs/22-gpu-timestamp-layer-design.md) | How to build a Vulkan GPU timestamp layer that needs no admin or counters. |
| [23-descriptor-volatility-sources.md](docs/23-descriptor-volatility-sources.md) | What the public D3D12 docs say about descriptor volatility, with links, and what it means for a translator. |
| [24-wine-proton-facts.md](docs/24-wine-proton-facts.md) | Wine/Proton/vkd3d-proton behaviours that changed our design. |
| [25-open-questions.md](docs/25-open-questions.md) | What we still don't know. |

## Tools
[tools/](tools/): a Linux-side `/proc` sampler for Wine processes, a headless RGP capture reader and comparer, an event-triggered RGP capture watcher, and a slow-frame classifier. See [tools/README.md](tools/README.md).

## Caveats
- All numbers are from our own machines and one title, so read them as orders of magnitude and ratios. The machines were an AMD desktop and an AMD handheld on Linux/Proton, an RTX 4090 desktop and an RTX 3080 laptop on Windows, and a Ryzen 9 5900X desktop with an RX 6700 XT.
- The Series X section uses public information only. Verify it against your title.
- This is a clean-room description of our own work. It describes no other project's internals.

## Licence
- Documentation: [CC BY 4.0](LICENSE).
- Code in `tools/`: [MIT](LICENSE-CODE).
