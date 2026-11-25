# 10. Worked examples

## 10.1 One texture's life, from title write to GPU copy
A composite of what the traces show for a streamed road texture with exact mode, traps, the head-start worker with refresh, eager staging and the lease all on. Slot numbers, generations and mip counts are illustrative; the sequence and the decisions are real.

```
t=0      Title VirtualAlloc(MEM_COMMIT) for a pool block
           hook: strip console flags, drop write-combine, commit (write-watched)
           ledger: +span {committed, RW};  epoch++ ; commit counter++
t=1      Title "places" a tiled texture at an address inside that block
           create desktop texture (DEFAULT, committed, zeroed), note {title VA, gen 41}
           register tiled state: 11 mips → 7 spans (big mips) + 1 packed tail span
           arm traps on committed spans (PAGE_READONLY); tail page shared with
           a neighbour → arm merge, no extra VirtualProtect
t=2      Title thread 3 memcpy()s mip 2 into title memory
           first store faults → handler: span 2 Armed→Handling→Dirty, page restored,
           event pushed, worker semaphore released; store retried, continues
t=2+δ    HS worker (eager): drain → mark span 2 (mark_seq=9)
           arm span 2 again, take lease (shared stripes), snapshot 64 KiB,
           SHA-256, detile into an UPLOAD block, publish ticket(seq=9, sig)
t=3      Title writes the rest of mip 2 (a second burst)
           fault again → span 2 Dirty (mark_seq=10) → refresh: wanted=10
           worker re-stages span 2 only; new ticket(seq=10)
t=4      Title creates an SRV in heap H slot 812 (flat bookkeeping: slot 812 → tex gen 41)
           Title lowers the view's most-detailed mip to 2 (mips 0-1 not committed yet)
t=5      Title records a draw that binds a table covering slot 812
           draw reach: (H, range) → attention set includes tex gen 41
t=6      Title ExecuteCommandLists on the direct queue
           barrier: drain (nothing new) → classify: span 2 ticket seq 10 == mark 10 → USED
                    lease signature recomputed == stored → accept
                    spans 3-10 trusted (landed after last mark) → nothing to do
                    mips 0-1 uncommitted → nothing to copy
           record: barrier→COPY_DEST, CopyTextureRegion(span 2), barrier back
           execute upload list, Signal(upload fence) → spans "landed"
           native ExecuteCommandLists(title lists)  ← GPU now sees current mip 2
t=7      Title streams mip 1: VirtualAlloc commit → hook marks Commit on mip 1 span
           ... same cycle as t=2..t=6; if the worker hasn't started it by the
           next barrier, the barrier STEALS it and stages inline (+0.1-0.3 ms)
t=8      Title releases the texture
           Release hook: release-flip (state → dying, generation read in one hold)
           retire-own: disarm only this texture's tokens (neighbour on the shared
           tail page stays armed and trusted)
t=9      Title VirtualFree(block, 0, MEM_RELEASE)
           hook: resolve size 0 → real allocation range; exclusive lease stripes;
           mark Free (pre and post); ledger −span; epoch++
           desktop texture freed when its references go (or retired, if deferred
           release is on)
t=10     Title places a NEW texture at the same address: gen 42
           interval map keeps gen 42 as the live entry at that base; any stale
           lookup that still carries gen 41 is classified OlderCompatible/Unknown
```

Each step has a counter. The checks we ran on every test run:
- trap misses 0;
- trusted-but-unarmed 0;
- snapshot-read breaches only on the speculative worker;
- "changed without mark" 0 in the audit;
- lease refusals counted.

## 10.2 One streaming slow frame, broken down
AMD desktop, exact path on, the second worker off, a race lap. The figures are typical values from the frame-split and Linux-side sampler rows. They come from separate instruments, so they do not add up exactly.

```
present interval ................................ ~39-40 ms   (ordinary frame ~14.8 ms)
  presenting thread on-CPU ...................... ~29-30 ms   (ordinary ~11-11.6)
  presenting thread blocked ..................... ~9.4-11.2 ms (ordinary ~3.2-3.7)
  run-queue wait ................................ ~0.3 ms     (CPU never starved)

inside the barrier (one or more submissions with needs)
  inline staging of stolen/missing spans ........ ~9.9 ms for ~60 spans (up to 13.7 ms / 73)
  upload submit (record + execute + signal) ..... ~2.8 ms
  barrier "rest" (classify, bookkeeping) ........ ~2.7 ms
  joins on running head-start passes ............ ~1.6-3.1 ms
  harvest / drain ............................... ~1 ms
  loader locks .................................. ~1.7-2.9 ms running + ~1-1.9 ms blocked

outside the barrier
  title's own code .............................. ~18.8 ms   (ordinary ~12.6 ms)
     → about +4-6 ms of title CPU we cannot yet attribute (cache pressure,
       our hooks' side effects, the title's own streaming work)
```

Reading it:
- The barrier is the biggest single block, so the head-start work helps.
- Almost as much again is extra *title* time outside the barrier. That is why perfect staging still leaves a ~31 ms frame (section 9.2).
- The next instrument has to go inside the title-side excess and inside the barrier's wait.

## 10.3 One burst slow frame, broken down
Same machine, same configuration.

```
present interval ................................ ~37-38 ms
  submissions this frame ........................ ~88   (usual ~15)
  barrier needs ................................. 0
  presenting thread on-CPU ...................... ~20.5-21 ms
  presenting thread blocked ..................... ~15.8-17.2 ms
     contended title critical section ........... ~6.2-9.8 ms
     title WaitForSingleObject .................. ~2-3.3 ms
meanwhile, another title thread
  inside our VirtualFree hook ................... ~10-16 ms (~71 calls × ~200 µs)
then, 3-5 frames later
  streaming slow frames (the textures from this step arrive) → section 10.2
```

Reading it:
- No staging work happens in this frame, so head-start changes cannot help it.
- The suspects are the title's own lock and the cost of our VM hooks under it. The ownership instrument (section 9.7) decides between them.

---
[Back to the index](../README.md)
