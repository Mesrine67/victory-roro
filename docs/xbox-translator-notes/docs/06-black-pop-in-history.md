# 6. The black pop-in history, and what it taught us

This section is the story behind section 5.1's contract. We spent weeks on it, and most of that time went on attempts that could never have worked.

## 6.1 The symptom
Streamed textures, the road surface most visibly, sometimes drew **black** for 0.2-0.3 s. At scene cuts, whole regions were black for seconds, worst on lap 1 of a race. The original game shows no such thing.

## 6.2 Timeline
| Phase | What we believed | What we did | What happened |
|---|---|---|---|
| 1 | The upload pipeline was too slow. | Removed caps on the uploader (it was limited to a few uploads per present). | **Real win for lap 1**: 981 vs 231 uploads/s on the handheld, and the lap-1 road lit from 0:10 instead of 1:18. Short pops remained. |
| 2 | Upload ordering or priority was wrong. | Priority queues, "first lane" for early draws, fair queues, wake-ups, admission tuning, cancelling obsolete jobs, more drain jobs per present. | Big latency improvements (one fix cut ready-to-landed p95 from 29.0 s to 2.3 s, another cut landings over 1 s from 10.7 % to 0.1-0.2 %). **None removed the black.** One left 0 live jobs in the pipeline while 19,750 of 94,696 viewed generations never got mip 0. |
| 3 | The CPU detile was the bottleneck. | GPU detile. | Driver CPU time 49.96 s to 1.12 s. Still black. |
| 4 | Copy when needed: before the first draw. | "Demand pull", an inline copy before the draw, with a 64 MiB budget. | Hitches over 250 ms went from 2 to **115**, and 6-8 fps. Unbounded inline work on the draw path is not an option. |
| 5 | Make it not *look* black. | Clamp the descriptor's most-detailed mip to what has landed; a "floor" mip; a top-up policy. | Marginal at best. The clamp could act on only about 12-17 % of descriptor decisions, because most black samples had *nothing* resident yet. The floor made opponents' cars near-black. |
| 6 | Waiting textures are found too late. | Measured the re-check: each waiting texture was revisited only every 0.16-0.31 s. Built an event-driven *settle wake*. | Black road per lap on the AMD desktop: **6.5 / 6.3 / 5.3 s → 0 / 0 / 0.27 s**. Starving the re-check deliberately brought it back: 88 % of whole-road shots black. With the wake, 1.1 %. Shipped. Short flashes remained. |
| 7 | Remaining flashes are the title rendering black. | Painted the road draw a solid colour (*pixel ownership*: is that draw actually there?), then filled every not-yet-uploaded mip with **magenta**. | The road draw was there every frame. Magenta appeared in 69 of 73 marked moments: trees, fences, kerbs, decals. Worst after scene cuts (garage menu about 50-61 % magenta for about 3 s). **The data was simply not on the GPU when sampled.** |
| 8 | We must know exactly when the title finished writing. | The exact path (sections 4-5). | Correct images. The rest of the work was performance. |

## 6.3 Why the early fixes could not work
- **Unwritten mips read zero.** Committed resources are zeroed, draws never wait, and there was no placeholder. A late upload is black, whatever the queue order.
- **"Late" was mostly "not noticed".** Data reached title memory by CPU stores. Our polling noticed it fractions of a second later. Faster *uploading* cannot fix slow *noticing*.
- **Lowering the mip only helps when a lower mip is resident.** Most black samples had none.
- **The title asked for its data correctly.** Changing what or how much the title requested (texture detail, budgets) made no difference. Peak texture memory stayed at about 1.3 GiB with a budget of 2 GiB or 3.5 GiB.

## 6.4 Debugging methods that worked
1. **Prove pixel ownership before analysing shaders.** Tint the suspect draw a flat colour. If the tint is solid every frame, the draw is fine and its *inputs* are wrong. This removed a whole class of theories in one short run.
2. **Fill missing data with an impossible colour.** Magenta checkers, filled at registration time, made "not uploaded yet" visible everywhere at once. A control run showed 25 of 25 probe pixels at exact zero. Be aware that the fill cannot cover render targets, depth or UAV textures, and that it changes timing.
3. **Starve the suspected mechanism on purpose.** If shrinking the re-check budget makes the bug worse in proportion, you have the mechanism.
4. **Trust eyes over scorers.** Automated "percent black in a region" scorers were blind to some cases: cars and shadows covered the road, and the scorer compared different shots. When a human viewer saw it and the scorer did not, the viewer was right every time.
5. **Keep a "tried / ruled out" table** for each problem, with the switch, machine, result and what it rules out. Ours grew to over 40 rows. It stopped us retrying ordering variants of the same failed idea.

## 6.5 The frame-clocked world
When the exact path was slow (1-5 fps in early builds), the world on a long straight vanished for about 10 seconds. That was not our bug in the usual sense. The title's own streaming and our wall-clock uploader both fell behind because frames were slow. Moving our base uploader to a wall clock (60 Hz, below-normal priority) and making the drain O(changes) fixed it. **Measure performance and correctness together. A fix that halves frame rate changes what the title streams.**

## 6.6 Exactness bugs we found in our own exact path
Each of these would have brought black textures back silently:

- **Stale readability cache.** A long proof run found 27,830 events where a span was cached as unreadable while its memory was readable, so its copy could be skipped. Cause: a birth race, where readability was computed before the state was inserted, so a commit in between was never seen. Fixed, and the count went to 0.
- **Snapshot reads of memory being decommitted** (section 5.4).
- **Bundle template shortcut.** A proposed optimisation recorded a bundle's descriptor-table use once and reused it. A caller that inherits a table set *inside* a bundle would then record the old table: silent under-inclusion. Blocked in review before it was built.
- **"No title backing" rename.** Three menu textures are filled every frame by *buffer copies*, which are not CPU stores, so the barrier never saw them. A proposed rename would have dropped them. We added copy sources to the census instead.
- **Size-zero free** (section 2.3) was a cost bug, not a correctness bug. But it shows how one console shorthand can dominate everything.

---
[Back to the index](../README.md)
