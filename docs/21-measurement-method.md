# How we measured: the A/B method in practice

[13-measurement-discipline.md](13-measurement-discipline.md) lists the rules. This document shows how we put them into practice, so you can copy the setup.

## 1. A test "row"
One row is one unattended game run with a fixed configuration. Each row records:
- **build identity**: executable hash, and the effective value and source of every switch, from a census the loader writes at startup;
- **machine identity**: GPU and driver; Proton or Wine version; power limit *and* measured power on handhelds; display on or off; whether remote-desktop software is running; free commit charge;
- **the route**: a scripted, repeatable drive (menus, race start, a fixed number of laps). Every frame is logged with a lap number and lap seconds;
- **health**: trap counters, breaches, lease refusals, device-loss checks and fence timeouts. A row whose health block is not clean is not used for performance;
- **progress**: a lap 1 → 2 → 3 progression check. A row with a stuck car once passed a simple "reached lap 1" check.

## 2. Arms and passes
- An **arm** is one configuration, for example "second worker plus eager staging" or "off".
- Each comparison runs at least **two passes per arm**, ideally interleaved (A, B, A, B) on the same machine and session.
- An extra **off vs off** pair gives the noise band. Typical noise was 2-8 real slow frames a minute, sometimes as much as the effect.
- **Promotion gate:** the effect beats the off/off spread in both passes, on at least two machine types, with health clean. Only then does a switch's default change. We also record *which* gate it passed, next to the default.

## 3. What we quote
- **Real slow frames per minute** at 33.4 ms (and 16.7/50 ms), over race laps only, with harness capture frames excluded. A frame where our tool took a screenshot is not the game's frame.
- **Per class** (streaming, burst, zero-need), because a fix usually helps one class and the mix differs between machines. See [tools/frame_classes.py](../tools/frame_classes.py).
- **fps and p99**, as a secondary check.
- **The spread**, always next to the delta.

## 4. Joining CPU-side samples to in-process frame marks under Wine
Wine implements `QueryPerformanceCounter` at 10 MHz, from the Linux monotonic raw clock. So a Linux-side tool that reads `CLOCK_MONOTONIC_RAW` and divides nanoseconds by 100 gets *the same timebase* as the game's QPC timestamps. Frame marks logged in-process then line up directly with `/proc` samples taken outside. [tools/linux_proc_sampler.py](../tools/linux_proc_sampler.py) does this.

To check the join, compare the sampler's per-frame CPU for the presenting thread with an in-process total. Ours matched (14.90 vs 14.90 ms per frame, correlation 0.90).

What the sampler gives per thread, at about 1 kHz:
- **state**: R (running or runnable), S (sleeping), D (uninterruptible);
- **schedstat**: on-CPU ns, run-queue wait ns, timeslices;
- **syscall and wchan** while sleeping. These tell a futex (a contended critical section under Wine) from an ntsync or other wait object.

That is how we found that burst frames were blocked on a contended title lock, and that run-queue delay was negligible (the CPU was not starved).

## 5. Splitting a frame
In-process, the presenting thread keeps an *exclusive* timer stack. Each slow frame's excess over the line is attributed to these parts:
- title run;
- hooked waits;
- pipeline creation;
- trap faults;
- the barrier's harvest, drain, enumerate, classify, staging wait, stage and submit;
- a "rest" part.

Parts must sum exactly to the frame. A remainder that is not inside any timer is reported as such, and it is often the most important number: it tells you where to put the next timer.

Rules we kept:
- Timers on the presenting thread take no lock, allocate nothing and use no TLS.
- Writes go to fixed per-thread slots; a reader thread aggregates them.
- Anything that costs real time is a test-build-only feature.

## 6. GPU side
- **Busy vs wall clock per frame**, from a timestamp layer ([22-gpu-timestamp-layer-design.md](22-gpu-timestamp-layer-design.md)) or RGP captures parsed headless ([tools/rgp_parse.py](../tools/rgp_parse.py)).
- Compare arms **like for like** (both traced). Tracing inflates idle time, but GPU busy time is real.
- Look for **idle before the first big list**. On our machines that gap *was* the cost of our CPU work.

## 7. Visual correctness
- **Eyes first.** When a person sees a problem that a scorer does not, the person is right.
- **Prove pixel ownership** (tint the suspect draw) before analysing shaders.
- **Fill missing data with an impossible colour** to see lateness everywhere at once.
- **Pair automated scores with frames from the same moment**, never different shots of "the same place".

## 8. Small-n traps we fell into
- 5 of 8 against 0 of 17 looked decisive for a device-loss cause. A paired 6x6 A/B then gave 1 of 6 either way.
- One review averaged worker CPU over a run and concluded the worker was idle. It was saturated in exactly the frames that mattered.
- One review's correlation used a convenient subset of burst frames (31 of 140). On all usable frames it fell from 0.76 to 0.53, and in the other race from 0.90 to 0.06.

Have someone whose job is to break each claim re-derive the load-bearing numbers from the raw rows.

---
[Back to the index](../README.md)
