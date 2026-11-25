# Appendix A. Switch names used as examples
These are our environment switches, named here only to make the mechanisms concrete. Code defaults are mostly off, and a play profile turns on the proven set.

| Example switch | Mechanism |
|---|---|
| `XBOX_TEX_DRAW_EXACT` | The exact texture path (sections 4-5). |
| `XBOX_TEX_DRAW_EXACT_DETECT=trap\|watch` | Writer detection mode. |
| `XBOX_TEX_DRAW_EXACT_ARM_MERGE` | Skip re-protecting pages already armed by neighbours. |
| `XBOX_TEX_DRAW_EXACT_RETIRE_OWN` | Retire disarms only the texture's own tokens. |
| `XBOX_TEX_TRAP_EXCLUSION_FAST` | Exact-key exclusion lookup instead of a window walk. |
| `XBOX_TEX_SNAPSHOT_LEASE` | Striped mapping leases for snapshot reads. |
| `XBOX_TEX_SNAPSHOT_POOL` | Reuse snapshot reservations. |
| `XBOX_TEX_HASH_FAST` | Own SHA-256 with SHA instructions. |
| `XBOX_TEX_DETILE_FAST` | Learned detile run tables, verified against the library. |
| `XBOX_TEX_UPLOAD_BATCH` | Batched upload submission, capped. |
| `XBOX_TEX_DRAW_EXACT_HS_REFRESH` | Head-start refresh semantics. |
| `XBOX_TEX_DRAW_EXACT_HS_EAGER` | Eager head-start staging. |
| `XBOX_TEX_HS_WORKER_BOOST=2` | Second head-start worker, used only together with eager staging. |
| `XBOX_TEX_STAGE_EARLY` | Hot/cold head-start queues (no gain). |
| `XBOX_TEX_DRAW_EXACT_STAGE_FANOUT` | Fan inline staging out to helper threads. |
| `XBOX_TEX_DRAW_EXACT_FLAT` | Flat descriptor bookkeeping. |
| `XBOX_TEX_DRAW_EXACT_BUNDLE_INPLACE` | In-place bundle reach. |
| `XBOX_TEX_READ_MAP` | Title memory map instead of `VirtualQuery`. |
| `XBOX_TEX_SETTLE_WAKE` | Event-driven re-check of waiting textures (pre-exact design). |
| `XBOX_TEX_UNRESIDENT_FILL` | Debug fill for missing data. |
| `XBOX_DEFER_RELEASE` | Fence-certified deferred release. |
| `XBOX_TITLE_VA_RELAUNCH` | Relaunch when a title range hold is lost. |
| `XBOX_BUFFER_TIGHT_ALIGN` | Tighter small-buffer alignment through a vkd3d-proton private flag. |
| `XBOX_EXACT_FRAME_SPLIT`, `XBOX_EXACT_STALL_SPLIT`, `XBOX_EXACT_WAIT_SPLIT`, `XBOX_EXACT_LOCK_OWNER`, `XBOX_TEX_DRAW_EXACT_HOOK_TIMING`, `XBOX_TEX_DRAW_EXACT_MEASURE` | Observers (section 11). |

---
[Back to the index](../README.md)
