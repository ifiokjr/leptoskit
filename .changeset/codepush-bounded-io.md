---
"release": patch
---

# Bound code push origin work

Code push downloads now stream in 64 KiB chunks with Range and HEAD support instead of buffering entire release and delta files. Update checks use container hashes saved at ingest or import without reading release or delta bodies. Metadata work runs off async workers, and each router rejects excess requests with 503 while 16 responses are active, including downloads still in progress. Older stores without saved hashes continue to offer full downloads through the dynamic routes.
