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

/* Application ABI, available when the Rust `app` feature is enabled.
 * The generated library supplies leptoskit_app_create_with_context and its registrant.
 * All app operations stay on the UI thread. Stop native tasks before destroy.
 * An app's bridge is borrowed; NEVER call lk_bridge_destroy on it. */
typedef struct lk_app lk_app;
typedef struct lk_closing_app lk_closing_app;
/* Context stays owned until release, including concurrent late wakes.
 * Wake/release are thread-safe. Wake enqueues one UI turn without reentry.
 * Close runs on the UI owner, retains its supplied owner and polls until Ready.
 * A valid callback table transfers context ownership on every return path. */
typedef struct {
    void *context;
    void (*wake)(void *context);
    void (*close)(void *context, lk_closing_app *owner);
    void (*release)(void *context);
} lk_service_callbacks;
/* Existing application-private directories, borrowed for this call only.
 * Paths are UTF-8 bytes, not NUL-terminated strings. */
typedef struct lk_app_context {
    const uint8_t *data_dir;
    size_t data_dir_len;
    const uint8_t *cache_dir;
    size_t cache_dir_len;
} lk_app_context;
lk_app *leptoskit_app_create_with_context(const uint8_t *wasm, size_t len,
                                        const lk_app_context *context, lk_buffer *error);
void lk_app_destroy(lk_app *app);
/* Activate deferred Rust services before start. No-service apps succeed.
 * All callbacks must be nonnull; invalid tables retain caller ownership. */
int32_t lk_app_activate_service(lk_app *app, lk_service_callbacks callbacks);
int32_t lk_app_poll_service(lk_app *app);
/* Host text measurement. measure lays out len UTF-8 bytes at font_size logical
 * points, CSS font_weight (400-700), and rtl direction within width logical
 * points (-1 unbounded, -2 the widest word), writes the logical width and
 * height to out[0] and out[1], and returns true. It may run on any thread.
 * release runs once, after the last measurement. */
typedef struct lk_text_measurer {
    void *context;
    bool (*measure)(void *context, const uint8_t *text, size_t len, float font_size,
                    uint32_t font_weight, float width, bool rtl, float *out);
    void (*release)(void *context);
} lk_text_measurer;
/* Measure text with the host's fonts and lay the tree out again. Null
 * callbacks retain caller ownership; a null app releases the context. */
int32_t lk_app_set_text_measurer(lk_app *app, lk_text_measurer measurer);
/* Poll service first, then use this rather than draining Bridge cancellation. */
int64_t lk_app_next_cancelled(lk_app *app);
/* Consumes App. Admission/delivery stop before asynchronous joins begin. */
lk_closing_app *lk_app_begin_close(lk_app *app);
/* 0 pending, 1 closed, negative joined failure. Poll again only on wake. */
int32_t lk_app_close_poll(lk_closing_app *owner);
/* Pending release transfers cleanup back to the installed close scheduler. */
void lk_app_close_release(lk_closing_app *owner);
lk_bridge *lk_app_plugins(lk_app *app);
int32_t lk_app_start(lk_app *app, float width, float height);
int32_t lk_app_resize(lk_app *app, float width, float height, bool dark);
/* Kinds: press 0, change 1, submit 2, focus 3, blur 4, press in 8, press out 9.
 * Press in and out bracket a button's native pressed state. */
int32_t lk_app_event(lk_app *app, size_t node, uint32_t kind, const uint8_t *text, size_t len);
int32_t lk_app_wake(lk_app *app);
/* The display frame clock, for hosts with a display link: enable it before
 * starting the guest. Then 1 while the guest waits for a frame (run the display
 * link), 0 otherwise. On each display-link tick, deliver the frame, then pump
 * and render before the callback returns. */
int32_t lk_app_enable_frames(lk_app *app); /* before lk_app_start */
int32_t lk_app_frame_requested(lk_app *app);
int32_t lk_app_frame(lk_app *app, uint64_t timestamp_ns, uint64_t period_ns);
/* 1 when lk_app_render has changes, a new root or a failure to send, else 0:
 * call it first and skip rendering when nothing changed. */
int32_t lk_app_render_pending(lk_app *app);
lk_buffer lk_app_render(lk_app *app);
/* UI-thread native frames. Callbacks synchronously upload and never retain
 * pixel pointers or reenter app functions. visible: 0/1; present: 1 success.
 * Flags: 1 active/visible window, 2 reduced motion. Tick: 1 pending, 0 idle,
 * -1 diagnostic. No guest execution or JSON pixel transport occurs. */
typedef int32_t (*lk_rive_visible_fn)(void *context, size_t node);
typedef int32_t (*lk_rive_present_fn)(void *context, size_t node, uint32_t width,
    uint32_t height, const uint8_t *pixels, size_t length);
int32_t lk_app_has_rive(const lk_app *app);
int32_t lk_app_rive_tick(lk_app *app, double elapsed, float scale, uint32_t flags,
    void *context, lk_rive_visible_fn visible, lk_rive_present_fn present);
lk_buffer lk_app_error(lk_app *app);
/* 1 native request, 0 idle, 2 yield to the next UI turn, negative failure. */
int32_t lk_app_next_request(lk_app *app, lk_request *out);
/* Worker handles own their plugin/payload independently of the app. The host
 * must reserve bounded job capacity before draining a request. Retain that
 * capacity until execution finishes, even after guest cancellation. */
typedef struct lk_rust_worker lk_rust_worker;
typedef struct lk_worker_reply lk_worker_reply;
typedef struct lk_worker_cancellation lk_worker_cancellation_token;
/* UI thread: borrow request and retain its Bytes, without copying. Null means
 * a platform-native request. The original request payload still needs release. */
lk_rust_worker *lk_app_worker(lk_app *app, const lk_request *request);
/* Borrow before execution; returned token has independent ownership. */
lk_worker_cancellation_token *lk_worker_cancellation(const lk_rust_worker *worker);
/* Thread-safe; suppresses invocation even while waiting for the plugin mutex.
 * Running plugin I/O is not interrupted. Token must remain live for the call. */
void lk_worker_cancel(const lk_worker_cancellation_token *token);
void lk_worker_cancellation_release(lk_worker_cancellation_token *token);
/* Background thread: consumes worker; may block. Never touches the app/bridge. */
lk_worker_reply *lk_worker_execute(lk_rust_worker *worker);
/* Release an unexecuted worker. Null is allowed. */
void lk_worker_release(lk_rust_worker *worker);
/* UI thread: consumes reply on success AND failure; zero-copy bytes transfer.
 * A null bridge is allowed after shutdown and still consumes the reply. */
int32_t lk_worker_complete(lk_bridge *bridge, uint64_t id, lk_worker_reply *reply);
/* Release a discarded reply. Null is allowed. */
void lk_worker_reply_release(lk_worker_reply *reply);

/* Installed-bundle ABI, available with the Rust `app` feature.
 * All calls, including open/destroy, run on one serialized background worker.
 * The owner NEVER contains or borrows an lk_app. Only one owner may access a
 * directory. Configuration is UTF-8 JSON; directory/receipts are UTF-8 bytes.
 * Inputs are borrowed only for the call. Status is 0 success, -1 failure;
 * lk_updates_error copies the last diagnostic. Release owned outputs once.
 * Open bounds configuration to 1 MiB, embedded to 32 MiB, directory to 32 KiB;
 * confirm/reject bound receipt to 4 KiB. Output pointers must be nonnull. */
typedef struct lk_updates lk_updates;
lk_updates *lk_updates_open(const uint8_t *configuration, size_t configuration_len,
                            const uint8_t *embedded, size_t embedded_len,
                            const uint8_t *directory, size_t directory_len,
                            lk_buffer *error);
void lk_updates_destroy(lk_updates *updates);
/* On success returns separately owned Wasm and JSON metadata:
 * {receipt:string,version:string,build:uint64,candidate:bool,recovery?:string}.
 * Outputs must be distinct. A trial marker is durable before return. The host
 * validates/starts the guest on its UI thread; on failure reject its receipt
 * and launch again. On failure outputs remain uninitialized. */
int32_t lk_updates_launch(lk_updates *updates, lk_buffer *wasm, lk_buffer *metadata);
/* Blocking HTTP + verification + atomic stage; never activates a guest.
 * Success metadata: {status:"current"|"staged",version:string,build:uint64}.
 * On failure output remains uninitialized. */
int32_t lk_updates_sync(lk_updates *updates, lk_buffer *metadata);
/* Confirm only after native first-frame attachment succeeds. Receipts identify
 * individual launches; stale callbacks cannot confirm a replacement launch. */
int32_t lk_updates_confirm(lk_updates *updates, const uint8_t *receipt, size_t len);
/* Rejection followed by launch recovers a known working bundle. */
int32_t lk_updates_reject(lk_updates *updates, const uint8_t *receipt, size_t len);
lk_buffer lk_updates_error(lk_updates *updates);
/* Copies the verified entry module out of a trusted store-shipped container,
 * for hosts running without the update store (updates disabled, or the store
 * unavailable). Any thread. Returns 0 and an owned Wasm buffer, or -1 and an
 * owned diagnostic in `error`. Bounds the container to 32 MiB; outputs must be
 * distinct and nonnull. */
int32_t lk_bundle_entry(const uint8_t *embedded, size_t len, lk_buffer *wasm,
                        lk_buffer *error);

/* Development-guest ABI for apps `leptoskit dev` launched, available with the
 * Rust `app` feature. Calls block for the server's wait (25 s), so run them on
 * one dedicated background thread; the owner never borrows an lk_app. Open
 * takes the guest URL (UTF-8, at most 4 KiB) and the Wasm already running, and
 * fails in release builds, which never run unsigned guests. Next returns 1 and
 * an owned Wasm buffer for a new guest, 0 when nothing changed, and -1 on
 * failure; lk_dev_error copies the last diagnostic. */
typedef struct lk_dev lk_dev;
lk_dev *lk_dev_open(const uint8_t *url, size_t url_len, const uint8_t *running,
                    size_t running_len, lk_buffer *error);
int32_t lk_dev_next(lk_dev *development, lk_buffer *wasm);
lk_buffer lk_dev_error(lk_dev *development);
void lk_dev_destroy(lk_dev *development);

/* Optional native async dispatch; these symbols exist without async-host too.
 * Retain both job and cancellation token before starting. A non-null job owns
 * its request Bytes independently of the app; release the original request.
 * start consumes the job and invokes completed exactly once, possibly before
 * returning or on another thread. The callback owns context and reply. It must
 * only resume the platform job, not access the app/bridge. Completion and
 * release reuse lk_worker_complete/lk_worker_reply_release on the UI thread.
 * Cancellation never owns/releases context and never suppresses completion. */
typedef struct lk_async_job lk_async_job;
typedef struct lk_async_cancellation lk_async_cancellation;
lk_async_job *lk_app_async(lk_app *app, const lk_request *request);
lk_async_cancellation *lk_async_cancellation_retain(const lk_async_job *job);
void lk_async_cancel(const lk_async_cancellation *token);
void lk_async_cancellation_release(lk_async_cancellation *token);
void lk_async_release(lk_async_job *job);
void lk_async_start(lk_async_job *job, void *context,
                    void (*completed)(void *context, lk_worker_reply *reply));

#ifdef __cplusplus
}
#endif
#endif
