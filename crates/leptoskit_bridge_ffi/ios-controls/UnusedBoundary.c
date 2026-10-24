#include "leptoskit_bridge.h"
#include <stdlib.h>

lk_bridge *lk_bridge_create(void) { abort(); }
void lk_bridge_destroy(lk_bridge *bridge) { abort(); }
int64_t lk_bridge_register(lk_bridge *bridge, const uint8_t *name, size_t name_len,
    const uint32_t *methods, size_t method_count) { abort(); }
int32_t lk_bridge_grant(lk_bridge *bridge, uint32_t plugin, uint32_t method, bool allowed) { abort(); }
int64_t lk_bridge_submit(lk_bridge *bridge, uint32_t plugin, uint32_t method,
    const uint8_t *data, size_t len) { abort(); }
int32_t lk_bridge_next_request(lk_bridge *bridge, lk_request *out) { abort(); }
int32_t lk_bridge_complete(lk_bridge *bridge, uint64_t id, int32_t error,
    const uint8_t *data, size_t len) { abort(); }
int32_t lk_bridge_complete_owned(lk_bridge *bridge, uint64_t id, lk_buffer buffer) { abort(); }
int32_t lk_bridge_take_result(lk_bridge *bridge, uint64_t id, lk_buffer *out) { abort(); }
int32_t lk_bridge_cancel(lk_bridge *bridge, uint64_t id) { abort(); }
int64_t lk_bridge_next_cancelled(lk_bridge *bridge) { abort(); }
int32_t lk_bridge_emit(lk_bridge *bridge, uint32_t plugin, uint32_t kind,
                       const uint8_t *data, size_t len) { abort(); }
lk_worker_cancellation_token *lk_worker_cancellation(const lk_rust_worker *worker) { abort(); }
void lk_worker_cancel(const lk_worker_cancellation_token *token) { abort(); }
void lk_worker_cancellation_release(lk_worker_cancellation_token *token) { abort(); }

lk_worker_reply *lk_worker_execute(lk_rust_worker *worker) { abort(); }
void lk_worker_release(lk_rust_worker *worker) { abort(); }

int32_t lk_worker_complete(lk_bridge *bridge, uint64_t id, lk_worker_reply *reply) { abort(); }
void lk_worker_reply_release(lk_worker_reply *reply) { abort(); }
