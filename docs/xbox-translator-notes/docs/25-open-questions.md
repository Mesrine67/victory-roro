# Open questions (what we still don't know)

These are honest gaps. If your translator answers any of them, we would like to hear it.

1. **Who holds the title lock in burst frames?** In burst slow frames the presenting thread waits 6-10 ms on a contended title critical section. Another title thread spends 10-16 ms in our `VirtualFree` hook in the same frames. The correlation is weak on the full data set, and contention also happens in frames with no `VirtualFree` at all. We have built an observer that records the lock owner and whether it is inside one of our hooks. Until it reports, "our VM hook stretches a title lock hold" stays a hypothesis. ([09-performance.md](09-performance.md), section 9.7.)
2. **The unattributed title-side CPU.** In streaming and burst slow frames the title's own code runs about 4-7 ms longer than in ordinary frames, outside every timer we have. A static estimate says that removing it alone would clear 40-67 % of streaming slow frames. Candidates: cache and TLB pressure from our hooks, side effects of our descriptor bookkeeping, or the title's own streaming work.
3. **Why does a second head-start worker *alone* add slow frames?** It adds 10-13 burst/zero-need slow frames per minute. With eager staging as well, it is a clear gain. We keep the pair together and do not know why.
4. **The NVIDIA device loss.** The signature points at a stale descriptor reading a destroyed small-mip tail at a reused address. Fence-based holds don't help. Record-time table copies are a known volatility violation. A refill-at-submit design is specified but not proven to cure it. ([08-lifetime-device-loss.md](08-lifetime-device-loss.md).)
5. **The ceiling.** Perfect staging would still leave slow frames at about 31 ms. The next big gain is not in texture staging. It is somewhere in the title's own submission bursts and our VM hooks.
6. **Zero-copy buffers over title memory** (`OpenExistingHeapFromAddress`) work in isolation. We have not proven them together with write traps and under Wine.
7. **GPU-side detection.** We never tried detecting changes on the GPU, for example by hashing on a compute queue. With the GPU 80 % idle that might be cheaper than CPU hashing, but ordering against title writes is hard.

---
[Back to the index](../README.md)
