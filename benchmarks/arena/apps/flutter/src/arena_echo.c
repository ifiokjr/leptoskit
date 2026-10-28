// The arena's ArenaEcho module for the sync tier: plain C functions that
// dart:ffi calls directly. Each echo copies its input into a buffer the caller
// owns, so a call does the same work as returning a fresh value.

#include <stddef.h>
#include <stdint.h>
#include <string.h>

#if defined(_WIN32)
#define ARENA_EXPORT __declspec(dllexport)
#else
#define ARENA_EXPORT __attribute__((visibility("default"))) __attribute__((used))
#endif

ARENA_EXPORT void arena_echo_none(void) {}

ARENA_EXPORT int64_t arena_add_one(int64_t value) { return value + 1; }

ARENA_EXPORT void arena_echo_bytes(const uint8_t* input, size_t length,
                                   uint8_t* output) {
  memcpy(output, input, length);
}

ARENA_EXPORT void arena_echo_int64s(const int64_t* input, size_t count,
                                    int64_t* output) {
  memcpy(output, input, count * sizeof(int64_t));
}
