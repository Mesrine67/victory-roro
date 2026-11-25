#!/usr/bin/env python3
"""Linux-side /proc sampler for a Wine/Proton game process (runs next to the game; read-only, outside the game process).

Why: under Wine, files are opened by wineserver, so in-process /proc/thread-self reads (through Wine's Z: drive) describe wineserver's
thread, not the calling thread. This sampler reads /proc/<pid>/task/<tid>/{stat,schedstat,syscall,wchan} of the busiest threads of the process.
Time base: CLOCK_MONOTONIC_RAW in 100 ns units = Wine's QueryPerformanceCounter (qpf 10 MHz), so records join the in-process frame marks.

usage: linux_proc_sampler.py <out file> --exe <name.exe> [--hz 1000] [--top 4] [--settle 60] [--max 3000]
  waits for the <name.exe> process with the most threads, waits --settle s, then every 5 s re-picks the --top threads by CPU over
  the last 5 s (all tasks' schedstat at 0.2 Hz) and samples those at --hz until the process exits or --max s.
Output (text, one record per line): 'T <qpc> <tid> <state> <run_ns> <rq_ns> <slices> <sys> <wchan>' and 'P <qpc> <tid> <cpu_ns_5s>'
(the pick), 'H pid=<pid> hz=.. top=..' header, 'C <qpc> sampler_run_ns=..' (own cost) every 5 s.
"""
import os, sys, time

def arg(name, default):
    return type(default)(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default

if len(sys.argv) < 2 or sys.argv[1].startswith('-'):
    print(__doc__); sys.exit(2)
out_path = sys.argv[1]; EXE = arg('--exe', 'game.exe').encode(); HZ = arg('--hz', 1000); TOP = arg('--top', 4); SETTLE = arg('--settle', 60); MAX = arg('--max', 3000)
RAW = time.CLOCK_MONOTONIC_RAW
def qpc(): return time.clock_gettime_ns(RAW) // 100

def find_pid():
    best = None
    for p in os.listdir('/proc'):
        if not p.isdigit(): continue
        try:
            cmd = open('/proc/%s/cmdline' % p, 'rb').read()
            if EXE not in cmd or b'proton' in cmd.lower(): continue
            n = len(os.listdir('/proc/%s/task' % p))
        except OSError: continue
        if n >= 20 and (best is None or n > best[1]): best = (p, n)
    return best[0] if best else None

def rd(fd):
    try: return os.pread(fd, 1024, 0).decode('ascii', 'replace')
    except OSError: return ''

def main():
    pid = None
    while pid is None:
        pid = find_pid()
        if pid is None: time.sleep(2)
    out = open(out_path, 'w', buffering=1 << 20)
    out.write('H pid=%s hz=%d top=%d settle=%d start_qpc=%d\n' % (pid, HZ, TOP, SETTLE, qpc()))
    time.sleep(SETTLE)
    selfsched = os.open('/proc/thread-self/schedstat', os.O_RDONLY)
    tracked = {}  # tid -> dict of fds
    last_all = {}; next_pick = 0.0; period = 1.0 / HZ; t_end = time.monotonic() + MAX; nxt = time.monotonic()
    while time.monotonic() < t_end and os.path.exists('/proc/%s' % pid):
        now = time.monotonic()
        if now >= next_pick:
            next_pick = now + 5.0; cur = {}
            try: tids = os.listdir('/proc/%s/task' % pid)
            except OSError: break
            for t in tids:
                try: cur[t] = int(open('/proc/%s/task/%s/schedstat' % (pid, t)).read().split()[0])
                except (OSError, ValueError, IndexError): pass
            d = sorted(((cur[t] - last_all.get(t, cur[t]), t) for t in cur), reverse=True)
            if last_all:
                pick = [t for _, t in d[:TOP]]
                q = qpc()
                for dv, t in d[:TOP]: out.write('P %d %s %d\n' % (q, t, dv))
                for t in list(tracked):
                    if t not in pick:
                        for fd in tracked.pop(t).values(): os.close(fd)
                for t in pick:
                    if t not in tracked:
                        fds = {}
                        for f in ('stat', 'schedstat', 'syscall', 'wchan'):
                            try: fds[f] = os.open('/proc/%s/task/%s/%s' % (pid, t, f), os.O_RDONLY)
                            except OSError: pass
                        tracked[t] = fds
            last_all = cur
            out.write('C %d sampler_run_ns=%s\n' % (qpc(), rd(selfsched).split()[0] if rd(selfsched) else '?'))
        for t, fds in tracked.items():
            q = qpc(); st = rd(fds['stat']) if 'stat' in fds else ''
            r = st.rfind(')'); state = st[r + 2] if r > 0 and len(st) > r + 2 else '?'
            ss = rd(fds['schedstat']).split() if 'schedstat' in fds else []
            sysn, wch = '-', '-'
            if state in ('S', 'D'):
                sc = rd(fds['syscall']).split() if 'syscall' in fds else []
                sysn = sc[0] if sc else '?'
                wch = (rd(fds['wchan']) if 'wchan' in fds else '').strip() or '0'
            out.write('T %d %s %s %s %s %s %s %s\n' % (q, t, state, ss[0] if ss else '?', ss[1] if len(ss) > 1 else '?', ss[2] if len(ss) > 2 else '?', sysn, wch))
        nxt += period
        sl = nxt - time.monotonic()
        if sl > 0: time.sleep(sl)
        else: nxt = time.monotonic()
    out.write('E %d\n' % qpc()); out.close()

if __name__ == '__main__':
    main()
