---
"release": patch
---

# Block executable Pressable link targets

Pressable links accept relative URLs, fragments, and HTTP(S), mailto, tel, and sms URLs. Executable or unknown schemes and targets containing whitespace, controls, or backslashes render as ordinary buttons, preserving their press handlers. The same policy applies to server rendering, browser updates, and directly constructed or deserialized props.
