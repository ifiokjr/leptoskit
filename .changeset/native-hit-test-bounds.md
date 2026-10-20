---
"release": patch
---

# Prune native hit testing with cached subtree bounds

`NativeSurface::hit_test` skips subtrees that cannot contain the point. Each layout commit caches, per node, the union of its frame and its unclipped descendants' frames, so a query reads that dense cache instead of every node's props and layout. Results are unchanged: clipping, rounded clipping, hidden subtrees, stopped activity indicators, and decorative `Pressable` content all hit exactly as before.
