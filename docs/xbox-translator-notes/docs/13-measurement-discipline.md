# 13. Measurement discipline we learned the hard way

1. **Two passes per arm, always, plus an off-against-off pass.** Quote the spread next to every delta. Our noise on slow frames per minute was often 2-8 a minute, as large as many "improvements".
2. **Define the stretch:** race laps only, with a lap-progression check. One run had a stuck car that passed a simple "reached lap 1" test.
3. **Exclude your own capture frames** (section 9.4), and make the exclusion live in shared reader code.
4. **Record the machine's state:**
   - display on or off;
   - measured power, not the firmware limit;
   - remote desktop software (one remote-access tool on the 4090 cut a control from 88 fps to 57);
   - which GPU a hybrid laptop actually used (some early laptop runs were on the integrated GPU);
   - commit limit headroom.
5. **Separate frame classes** before averaging (section 9.2).
6. **Per-frame CPU splits need fine clocks.** Under Wine use an external sampler (section 9.8).
7. **Test builds are not play builds.** Some exactness checks ran only in test builds and cost time play builds never pay. The reverse also happened: a per-span log row in test builds alone (85,504 rows in one present, written under the barrier's locks) caused a 12-13 s level-load freeze that play builds never had.
8. **Promote a switch to default only after it passes a recorded gate.** Our play profile table records, for each switch, the gate it passed. For example, cached title memory: "4090 hitches about 27 → about 1 per minute, audit clean". An explicit environment value always overrides.
9. **Eyes over scorers** for visual correctness (section 6.4).

---
[Back to the index](../README.md)
