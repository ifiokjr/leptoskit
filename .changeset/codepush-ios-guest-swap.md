---
"release": patch
---

# Swap iOS code push guests atomically

While an update was being confirmed, the iOS host mounted the new guest on top of the running one, and both stayed attached. VoiceOver and UI tests could reach controls from both guests at once. The host now does what the Android host already did: it renders the candidate underneath, invisible and hidden from accessibility, while the confirmed guest keeps the screen and stays interactive. On confirmation it swaps the two in one main-thread turn.
