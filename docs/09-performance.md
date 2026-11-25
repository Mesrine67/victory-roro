# 9. Performance lessons

## 9.1 The submit-time budget is microseconds
The most useful experiment of all was cheap. With the exact path **off**, we inserted a synthetic busy-wait in every direct `ExecuteCommandLists`:

| Busy-wait per submission | Effect |
|---|---|
| 15 ms or 5 ms | The game froze at the first track load. |
| 1 ms | **-42 %** presents. |
| 0.2 ms | **-14 %** presents. |

The title submits many times per frame, and some of its own logic waits on those submissions. That set our budget for reach at **30 µs per submission**, and told us that inline staging in the barrier would always show up in frame time. **Run this experiment first** on any title you translate.

## 9.2 Slow frames come in three classes
With every frame split into exclusive timed parts, and each slow frame labelled, slow frames on the AMD desktop fell into three classes:

| Class | Share of real slow frames | Defining feature |
|---|---|---|
| Streaming | about two-thirds | The barrier had at least one need; barrier CPU about 5.3 ms against 0.4 ms in ordinary frames. |
| Title submission burst | 13-19 per minute | 30 or more submissions in one frame (about 88 against a usual 15), and **zero** needs. |
| Zero-need | about 3 per minute | Plain heavier title frames. |

More findings:
- The burst threshold is robust: anywhere from 22 to 40 moves the count by at most 0.4 per minute. Not one burst frame had a need (0 of 495).
- **Bursts and streaming frames are one event.** 84-95 % of heavy streaming frames fall 1-10 frames after a burst, with a peak 11-16x the base rate at +3 to +5 frames. The title's streaming step is a burst of submissions, then frames that consume new textures.
- On the 15 W handheld, bursts were the *largest* class: of 91 real slow frames per minute, 55 were bursts, 25 streaming and 12 zero-need.
- **Only streaming frames respond to head-start work.** Any fix aimed at staging can at best touch two-thirds of slow frames on the desktop, and under a third on the handheld.
- **Ceiling.** Even perfect staging would cut slow frames by 56-72 % on the desktop, but those frames would still be about 31 ms. Slow frames sit just over the line: median excess 3.4-3.9 ms, p90 13-14.5 ms.

## 9.3 Average frames lie; frame populations differ
- Only about 40 % of a test run's frames were race laps (25,310 of 64,982 on one row). Menus, loading and replays had about 4x the slow-frame rate. **Always quote one defined stretch** (race laps), never a whole run.
- Our desktop presented with vsync on, which gave about 3 ms of slack in ordinary frames. Savings smaller than that are invisible to fps there and appear only in timers.

## 9.4 Your capture harness can be half your measurement
Our test harness took a route screenshot every 2 s to track progress. Every capture frame was over 33.4 ms: 159 of 159 in one race. That made about **30 of the 84 slow frames per minute** in the headline number. A real change of 10-20 % disappeared under it, and several earlier "no change" verdicts were made on the inflated numbers.

Since then:
- "real" slow frames exclude capture frames;
- every reader script must apply the exclusion (one of ours didn't);
- we accept that 0.4-1.9 real hitches a minute hide inside capture frames, in every arm alike.

Related trap: with the display off, presents throttled to about 4 Hz on one machine, and the slow starts this caused looked like a performance regression *and* a pop-in regression. Keep the display on, and record its state.

## 9.5 The head-start worker: throughput, not lead time
- A design review read worker CPU at about 12 % and concluded the worker was idle, so the limit had to be *lead time*.
- A second reviewer found that most of the "first view" needs behind that reading were *disarm-only re-checks*: neighbour tokens disarmed by another texture's retire (section 4.7). Those are drained by the barrier microseconds before its own take.
- In burst-adjacent frames, with 17 or more takes, one slow-waking worker was saturated.
- The lead-time histogram (section 5.5) settled it: most stolen jobs were queued 16 ms or more ahead.

**Lessons.**
- An "idle worker" average can hide saturation inside the frames that matter. Measure per frame class, not per run.
- Before blaming a queue's order, histogram how long before the need each item was queued.
- Remove false work (the retire-own fix) before reading what the remaining work tells you.

## 9.6 The first measured gain: a second worker plus eager staging
A second head-start worker *alone* cut stolen needs by 85 % and did not help: level on the AMD desktop, worse on the handheld. With eager staging as well, real slow frames per minute (capture frames excluded) were:

| Machine | Race 1 off → on | Race 2 off → on | Off/off spread |
|---|---|---|---|
| AMD desktop, first test | 52.3 → 40.2 (-23 %) | 61.5 → 50.7 (-18 %) | 1.9 / 2.7 |
| AMD desktop, confirmation | 53.8 → 42.7 | 65.5 → 54.8 | |
| Handheld at a measured 15 W, two passes | 106 → 98, and 88 → 70 | | |
| Handheld, frames over 50 ms | 15.3 → 12.8, and 15.8 → 11.6 per minute | | |

There was no fps cost, and the change went into the play configuration. The rule we keep: **the second worker only together with eager staging.** The second worker alone added 10-13 burst or zero-need slow frames a minute, and we never found out why.

Why removing stealing helped so little: each streaming slow frame saved about 1.8 ms of barrier time, but about 1.1 ms of that came back as extra *title* run time in the same frame. Frame-time savings are not additive when the title's own threads compete for the same cores and locks.

## 9.7 The burst lock
In burst slow frames the presenting thread is **blocked** for 16-17 ms:
- 6-10 ms is a contended title critical section;
- 2-3 ms is a title `WaitForSingleObject`.

At the same time, another title thread spends **10-16 ms per burst frame inside our `VirtualFree` hook**: about 71 calls at about 200 µs each. The tempting theory: a title thread holds that critical section while calling `VirtualFree`, and our slow hook stretches the hold.

What a second reviewer found:
- The correlation drops from 0.76/0.90 to 0.53/0.06 when *all* usable burst frames are counted, not a convenient subset.
- Two other title threads contend 42-61 times at 90-109 µs even in burst frames with **zero** `VirtualFree` calls.
- One presenter wait spans 7-9 hook calls.

The proposed fix was a *lean allocation path*: skip empty trap walks, reuse one allocation walk per free, and skip the protect-back. Its last part was **not exact**. It left a page's "restorer" set, so it could later un-protect a shared page that another token still armed: a silent trap miss. That part was dropped. The cheaper exclusion lookup, tested against the burst lock, shortened the other threads' `VirtualFree` hook time by only 5-20 % and did not shorten the presenter's wait. So the test was not decisive.

Our next instrument records the *owner* of a contended critical section and whether it is inside one of our hooks. Decide from data, not correlation.

**Lessons.**
- **Your hooks run inside the title's locks.** A hook that is "only" 200 µs becomes a 10 ms stall for whichever thread waits on the lock the caller holds. Make VM hooks O(change) and lock-light. Never do registry-wide walks there.
- Every "skip work" optimisation in the trap layer needs a written exactness proof and a sampled re-check in test builds. Ours re-walks every 64th skipped walk and raises a breach on a miss.

## 9.8 Measuring under Wine/Proton
- **Thread priorities do nothing.** Under Proton, every Windows thread of the process ran at nice 0, `SCHED_OTHER`, rtprio 0, and the process's maximum nice priority was 0. `SetThreadPriority` to below-normal, normal or a temporary boost had no effect on the Linux scheduler. Only Wine's own internal disk threads ran at nice 19. Any design lever that relies on priority is a placebo there unless the session gets a nice rlimit.
- **In-process `/proc` reads describe the wrong thread.** Our in-process wait sampler opened `/proc/thread-self/...` through Wine's Unix path mapping. Under Wine, regular files are opened by the wineserver, so "thread-self" resolved to **wineserver's** thread. The CPU, run-queue, syscall and wait-channel fields were silently wrong, with 0 open failures in 130 status rows. Use a **Linux-side sampler** that reads `/proc/<pid>/task/<tid>/...` from outside the process. Ours cost about 8 % of one core. It matched the in-process presenter CPU per frame exactly (14.90 against 14.90 ms, correlation 0.90).
- **`GetThreadTimes` ticks at 10 ms under Wine.** Totals over many frames are fine. Per-frame CPU-versus-blocked splits are not. The external sampler showed the slow-frame excess was about **14-15 ms CPU plus 9 ms blocked**, not mostly blocked as the 10 ms ticks had suggested.
- **Run-queue delay was negligible** (under 0.35 ms per frame in every class). The CPU was not starved. The cost was real work and real lock waits.
- **Timer resolution.** On Windows with the default timer resolution, a 34 ms `WaitForSingleObject` ticks at about 21.5 Hz, so a "30 fps floor" was really about 21 Hz. Wine's waits are precise.
- **Power is not what the firmware says.** The handheld's firmware power limits read 35/37/45 W, but the GPU's measured power in game was 15.0 W. Record *measured* power on every handheld run.
- **The VM lock and `VirtualQuery`** (sections 2.5 and 2.6). Wine-specific costs drove several of our features: snapshot pooling, our own SHA-256, the title memory map, and the Wine-dependent defaults for detection mode.

---
[Back to the index](../README.md)
