# Viewport rendering cache integration

Parents: `1199b50311f4d3b0d567c86d9031f07b1a0746b8` and
`27c2678d52da7ba9445c7a644bde6e59af55ef10`.

Keep the bounded prepared-line cache, text measurement reuse and unchanged
Taffy assignment checks from #28. Add immutable canvas-row reuse above that
shared painter instead of keeping a second prepared-line implementation.

Content, attributes, wrapping and alignment changes invalidate both caches.
Painted rows additionally check the current width, clip and untouched destination
rows. Writes detach shared rows. The prepared layout retains at most 512 lines
and 2,048 segments; painted rows retain at most 16,384 cells per component.

Validation: 172 Windows library tests and strict Clippy pass. Downstream terminal,
browser and release-performance validation applies to the integrated revision.
