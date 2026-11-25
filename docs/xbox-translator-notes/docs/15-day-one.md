# 15. If we started again: day-one list

**Architecture**
1. Write the correctness contract (section 5.1) and the list of things it forbids before writing code.
2. Reserve all title ranges at process start. Log every reservation result and relaunch on failure.
3. One hook layer for every VM change (alloc, free, protect, query, `GetProcAddress`, heap allocator, I/O completions, DMA and decompression completions). One mutation lock, pre- and post-marks, `GetLastError` preserved.
4. A title memory ledger as the only source of "what is this memory", with a debug compare against the OS.
5. Generation-tagged identity for every resource, descriptor and address lookup.
6. Page traps as the primary writer detector, a second detector in test builds, and an explicit producer ledger for every non-CPU writer.
7. Leases for any read of title memory the title can unmap concurrently.
8. Copies on the same queue, ahead of the title's lists. No fence waits, no placeholders.
9. A staging worker pool with refresh semantics and eager start, sized from measured throughput. Under Proton, more workers, never higher priority.
10. Descriptor handling that is volatility-correct: refill changed slots at submit, never freeze record-time copies.
11. Fail-closed lifetime code: leak rather than free early, and stop with a report rather than guess.

**Costs**

12. Budget the submit path at tens of microseconds. Run the busy-wait experiment on day one.
13. Every hook O(change). No registry-wide walks, ever, on a path the title can call thousands of times.
14. Pool every scratch allocation. Under Wine every `VirtualAlloc`/`VirtualFree` takes a process-wide lock.
15. Use cached pages for title memory you will read back. Measure write-combined read cost on each OS.
16. Check what your platform's crypto and system libraries actually do. Ours lacked SHA instructions under Wine.

**Observability**

17. Observer-only instruments with exclusive frame splits, a health block in every run, and a switch census at startup.
18. Frame classes from the start: streaming, burst, other.
19. An external Linux sampler for CPU against blocked time, a GPU timestamp layer for busy against idle, and descriptor/resource history rings dumped on device removal.
20. A "tried / ruled out" table for every problem, updated after every result.

**Process**

21. Two passes per arm plus off-against-off, quoted spreads, defined stretches, harness frames excluded, machine state recorded.
22. A second reviewer for every non-trivial exactness or performance claim, whose job is to break it. Our reviewers caught wrong attributions, a refuted numbering rule, a non-exact "lean" path and a bundle shortcut that would have brought black textures back.
23. Eyes on the screen before calling anything fixed.

---
[Back to the index](../README.md)
