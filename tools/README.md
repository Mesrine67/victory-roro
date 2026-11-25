# Tools

Small, self-contained tools we wrote and used. None of them contains game data, and none needs anything from a console SDK. Python 3.8+ only, standard library.

| Tool | What it does | Where it runs |
|---|---|---|
| [`linux_proc_sampler.py`](linux_proc_sampler.py) | Samples `/proc/<pid>/task/<tid>/{stat,schedstat,syscall,wchan}` of a Wine/Proton process's busiest threads at about 1 kHz, from *outside* the process. Timestamps are `CLOCK_MONOTONIC_RAW / 100`, which equals Wine's QPC, so records join in-game frame marks directly. | Linux host running the game |
| [`rgp_parse.py`](rgp_parse.py) | Headless reader for RADV `.rgp` captures. Decodes the queue-event timings and ASIC info chunks into per-command-buffer GPU start/end, GPU busy (union), span, gaps, per-queue totals and the largest buffers. Does not decode the SQTT token stream (per-draw data needs the RGP GUI). | Anywhere |
| [`rgp_compare.py`](rgp_compare.py) | Per-capture and median markdown table across arms (`label=dir`), using `rgp_parse.py`. | Anywhere |
| [`rgp_trigger_watch.sh`](rgp_trigger_watch.sh) | Takes one-frame RGP captures when a pattern appears in the game's log (plus delays), so captures land on the frames you meant. | Linux host running the game |
| [`frame_classes.py`](frame_classes.py) | Classifies slow frames into streaming / submission burst / zero-need, excludes harness capture frames, reports per-minute rates and how many streaming frames follow a burst. Input is a simple per-frame CSV you export from your own logging. | Anywhere |

## Typical use
```bash
# 1. Start the game with RADV capture enabled (see the header of rgp_trigger_watch.sh), then:
setsid ./rgp_trigger_watch.sh ~/game.log 'LAP_START' ./captures "6 14" 6 &
setsid python3 linux_proc_sampler.py ./proc_samples.txt --exe yourloader.exe --hz 1000 --top 4 --settle 60 &

# 2. After the run:
python3 rgp_compare.py off=./captures_off on=./captures_on
python3 frame_classes.py frames_on.csv --stretch race --line 33.4
```

## Sampler output format
```
H pid=<pid> hz=.. top=.. settle=.. start_qpc=..          header
P <qpc> <tid> <cpu_ns_last_5s>                            thread pick (re-picked every 5 s)
T <qpc> <tid> <state> <run_ns> <rq_ns> <slices> <sys> <wchan>   one sample
C <qpc> sampler_run_ns=..                                 sampler's own CPU so far
E <qpc>                                                   end
```
Per-frame CPU of a thread is the difference in `run_ns` between the samples that bracket each frame. Run-queue delay is the difference in `rq_ns`. `sys`/`wchan` while sleeping tell lock waits (futex) apart from wait-object waits.

The GPU timestamp layer we used is described in [../docs/22-gpu-timestamp-layer-design.md](../docs/22-gpu-timestamp-layer-design.md). Its source is not included.

Licence: MIT (see [LICENSE-CODE](../LICENSE-CODE)).
