#ifndef LEPTOSKIT_BRIDGE_H
#define LEPTOSKIT_BRIDGE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ABI v1. Bridge access is single-threaded and non-reentrant.
 * A runtime may lend its Bridge for one pump; never destroy a lent pointer.
 * Native work can retain an lk_buffer (including on a worker) until release.
 * No function retains borrowed input pointers. Never use a released buffer.
 * Status errors are negative codes from leptoskit_bridge::Error.
 * These C statuses differ from the wasm statuses, which reserve -1 for pending.
 */
typedef struct lk_bridge lk_bridge;
typedef struct {
    const uint8_t *data;
    size_t len;
    void *owner;
} lk_buffer;
typedef struct {
    uint64_t id;
    uint32_t plugin;
    uint32_t method;
    lk_buffer payload;
} lk_request;

lk_bridge *lk_bridge_create(void);
void lk_bridge_destroy(lk_bridge *bridge);
void lk_buffer_release(lk_buffer buffer);
int64_t lk_bridge_register(lk_bridge *bridge, const uint8_t *name, size_t name_len,
                           const uint32_t *methods, size_t method_count);
int32_t lk_bridge_grant(lk_bridge *bridge, uint32_t plugin, uint32_t method, bool allowed);
int64_t lk_bridge_submit(lk_bridge *bridge, uint32_t plugin, uint32_t method,
                         const uint8_t *data, size_t len);
/* 1 available (release payload), 0 empty, negative error. */
int32_t lk_bridge_next_request(lk_bridge *bridge, lk_request *out);
int32_t lk_bridge_complete(lk_bridge *bridge, uint64_t id, int32_t error,
                           const uint8_t *data, size_t len);
/* Transfers an unmodified Rust buffer without copying. Consumes its owner on
 * success AND failure; do not subsequently release it or use its byte views. */
int32_t lk_bridge_complete_owned(lk_bridge *bridge, uint64_t id, lk_buffer buffer);
/* 1 available (release buffer), 0 pending, negative error (consumed). */
int32_t lk_bridge_take_result(lk_bridge *bridge, uint64_t id, lk_buffer *out);
int32_t lk_bridge_cancel(lk_bridge *bridge, uint64_t id);
/* Positive ID to cancel, 0 empty, negative error. Drain before starting new
 * native jobs; continue budgeting native resources until cancellation finishes. */
int64_t lk_bridge_next_cancelled(lk_bridge *bridge);
int32_t lk_bridge_emit(lk_bridge *bridge, uint32_t plugin, uint32_t kind,
                       const uint8_t *data, size_t len);

#ifdef __cplusplus
}
#endif
#endif
