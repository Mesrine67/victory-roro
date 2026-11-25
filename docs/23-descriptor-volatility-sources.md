# Descriptor volatility: what the public D3D12 documents say

Our design rule for descriptors ([07-draws.md](07-draws.md), section 7.5) rests on public documentation. This document summarises it, with links, so you can check it yourself. Everything here is a paraphrase. Read the pages for the exact wording.

## Sources
- Microsoft Learn, *Root Signature Version 1.1*: https://learn.microsoft.com/en-us/windows/win32/direct3d12/root-signature-version-1-1
  It covers the `DESCRIPTORS_VOLATILE`, `DATA_VOLATILE`, `DATA_STATIC_WHILE_SET_AT_EXECUTE` and `DATA_STATIC` flags, the flag summary table, and the defaults.
- DirectX-Specs, *Resource Binding*: https://microsoft.github.io/DirectX-Specs/d3d/ResourceBinding.html
  See the sections on changing state pointed to by descriptor tables, on object lifetime management being separate from binding, and on `CopyDescriptors`.
- vkd3d-proton's author on descriptor handling: https://themaister.net/blog/2021/11/ (root signature 1.0 forces fully volatile, bindless-style descriptors).
- Public reports of NVIDIA device losses on newer vkd3d-proton descriptor paths (descriptor buffer and descriptor heap). These are general reports, not tied to the pattern below:
  - https://forums.developer.nvidia.com/t/vkd3d-proton-descriptor-heap-path-causes-vk-error-device-lost-on-driver-610-in-the-first-descendant/374747
  - https://forums.developer.nvidia.com/t/vk-ext-descriptor-buffer-in-vkd3d-triggers-xid-69-on-pascal-cards-since-driver-535-series/291172

## What they establish
1. **Root signature 1.0 means "descriptors volatile, data volatile".** In version 1.1's terms, that is both flags set. With volatile descriptors, the application may change a descriptor in a heap at any time *except* while a command list or bundle that binds the table has been submitted and has not finished executing. The page gives "record, then change the descriptor, then submit" as an explicitly valid example.
2. **Version 1.1 defaults are static.** Without flags, descriptors must be ready when the table is set during recording, and must not change until the list has finished executing for the last time. For SRV and CBV ranges, the data defaults to "static while set at execute". UAV data defaults to volatile.
3. **Breaking a 1.1 promise is undefined behaviour**, and the results may differ by driver and hardware.
4. **Lifetime is separate from binding.** Before freeing a resource, the application must make sure the GPU has finished referencing it. A resource whose descriptor was overwritten before submit is, by this contract, not referenced by that list, so the title may legally release it. (This is our inference from the rules above. The spec does not spell out this exact case.)
5. **`CopyDescriptors` is a CPU-immediate copy.** The title's slot changes at the moment of the call, so a translator can see the change before `ExecuteCommandLists`.
6. **Unreferenced descriptors** in a table may change even while the list is in flight. Only the ones the shaders actually read are protected.

## What this means for a translator (our inferences)
- **Do not freeze record-time copies** of 1.0 tables. If you must keep a private copy (for example to rebind into your own shader-visible heap), **refill the slots that changed at submit**.
- **A refill without a fence wait is consistent with the contract.** If the title changed a slot, no in-flight list that reads it can exist, unless the title is already breaking the rules.
- **Same list submitted twice:** refill only changed slots. Never rewrite unchanged ones, or you race with the first execution.
- **Slots no shader reads** may change legally in flight. A refill that copies *every* slot can therefore race there. Restrict refills to the footprint the shaders can read (reflection), or accept and document the risk.
- **Multiple queues:** the rule is per submission to a queue. A title using async compute must fence between queues itself, and a refill inherits that assumption.
- **Check the root signature version and flags per table.** 1.1 static descriptors *do* make record-time decisions legal.

## What we saw
An audit at submit compared our frozen copies with the title's current heap. It counted slots `rewritten`, `changed`, `dead` (resource destroyed) and `dead-changed` (the title moved on *and* the copy still points at a destroyed resource). The last class is a GPU use-after-free waiting to happen, and it matches the device-loss signature in [08-lifetime-device-loss.md](08-lifetime-device-loss.md).

---
[Back to the index](../README.md)
