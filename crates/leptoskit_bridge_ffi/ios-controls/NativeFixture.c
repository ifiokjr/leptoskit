#include "leptoskit_bridge.h"
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <pthread.h>
#include <stdatomic.h>

struct lk_app {
    int identifier;
    int rendered;
    int status;
    int requested;
    int service_polled;
    lk_service_callbacks callbacks;
};
struct lk_closing_app { lk_app *app; };
struct lk_updates { int launch; };
enum { owner_limit = 32, service_failure = -77, joined_failure = -88 };
static int scenario;
static int next_identifier;
static int hold;
static int releases[owner_limit];
static int polls[owner_limit];
static lk_app *owners[owner_limit];

static lk_buffer bytes(const char *text) {
    char *copy = strdup(text);

    return (lk_buffer){(const uint8_t *)copy, strlen(copy), copy};
}

void fixture_reset(int mode) {
    scenario = mode;
    next_identifier = 0;
    hold = 0;
    memset(releases, 0, sizeof(releases));
    memset(polls, 0, sizeof(polls));
    memset(owners, 0, sizeof(owners));
}

int fixture_releases(int identifier) { return releases[identifier]; }
int fixture_polls(int identifier) { return polls[identifier]; }
void fixture_hold(int value) { hold = value; }
void fixture_finish(void) {
    hold = 0;

    for (int index = 1; index < owner_limit; index++) {
        lk_app *app = owners[index];

        if (app) app->callbacks.wake(app->callbacks.context);
    }
}

lk_app *leptoskit_app_create_with_context(const uint8_t *wasm, size_t len,
    const lk_app_context *context, lk_buffer *error) {
    lk_app *app = calloc(1, sizeof(lk_app));
    app->identifier = ++next_identifier;
    owners[app->identifier] = app;

    return app;
}

int32_t lk_app_activate_service(lk_app *app, lk_service_callbacks callbacks) {
    app->callbacks = callbacks;

    return 0;
}

int32_t lk_app_poll_service(lk_app *app) {
    app->service_polled = 1;

    return scenario == 1 && app->identifier == 1 ? service_failure : 0;
}

lk_closing_app *lk_app_begin_close(lk_app *app) {
    lk_closing_app *closing = malloc(sizeof(lk_closing_app));
    closing->app = app;
    app->status = 1;

    return closing;
}

int32_t lk_app_close_poll(lk_closing_app *closing) {
    polls[closing->app->identifier]++;

    if (hold) return 0;
    return (scenario == 1 || scenario == 2) && closing->app->identifier == 1 ? joined_failure : 1;
}

void lk_app_close_release(lk_closing_app *closing) {
    lk_app *app = closing->app;
    releases[app->identifier]++;
    owners[app->identifier] = NULL;
    app->callbacks.release(app->callbacks.context);
    free(app);
    free(closing);
}

void lk_app_destroy(lk_app *app) {
    lk_closing_app *closing = lk_app_begin_close(app);
    app->callbacks.close(app->callbacks.context, closing);
}

lk_buffer lk_app_error(lk_app *app) { return bytes("candidate service failed"); }
lk_bridge *lk_app_plugins(lk_app *app) { return (lk_bridge *)app; }
int32_t lk_app_start(lk_app *app, float width, float height) { return 0; }
int32_t lk_app_resize(lk_app *app, float width, float height, bool dark) { return 0; }
int32_t lk_app_wake(lk_app *app) { return 0; }
/* The fixture guest never waits for a display frame, so the link stays paused. */
int32_t lk_app_enable_frames(lk_app *app) { return 0; }
int32_t lk_app_frame_requested(lk_app *app) { return 0; }
int32_t lk_app_frame(lk_app *app, uint64_t timestamp_ns, uint64_t period_ns) { return 0; }
int32_t lk_app_event(lk_app *app, size_t node, uint32_t kind, const uint8_t *text, size_t len) { return 0; }
int64_t lk_app_next_cancelled(lk_app *app) { return 0; }
int32_t lk_app_next_request(lk_app *app, lk_request *request) {
    if (scenario != 3 || app->requested++) return 0;

    if (!app->service_polled) abort();
    *request = (lk_request){.id = 1, .plugin = 0, .method = 1, .payload = bytes("async")};

    return 1;
}

lk_rust_worker *lk_app_worker(lk_app *app, const lk_request *request) { return NULL; }

/* Every pump renders, as before the check existed. */
int32_t lk_app_render_pending(lk_app *app) { return 1; }
lk_buffer lk_app_render(lk_app *app) {
    if (app->rendered++) return bytes("{\"version\":1,\"mutations\":[]}");
    char json[512];
    // Guest 2's root paints an opaque slate-950 (#020617) canvas and asks for
    // `rounded-full`; guest 1 has neither.
    const char *background = app->identifier == 2 ? ",\"background\":33953791,\"corner_radius\":9999" : "";
    snprintf(json, sizeof(json), "{\"version\":1,\"root\":1,\"mutations\":[{\"Create\":{\"id\":1,\"kind\":\"text\",\"props\":{\"text\":\"Guest %d\",\"accessibility_label\":\"guest-%d\"%s}}},{\"UpdateLayout\":{\"id\":1,\"x\":0,\"y\":0,\"width\":200,\"height\":80}}]}", app->identifier, app->identifier, background);

    return bytes(json);
}

/* lk_app_render's transactions in lk_app_render_binary's layout. */
typedef struct {
    uint8_t data[256];
    size_t len;
} transaction;

static void put(transaction *out, const void *data, size_t len) {
    memcpy(out->data + out->len, data, len);
    out->len += len;
}

static void put_u32(transaction *out, uint32_t value) { put(out, &value, sizeof(value)); }

static void put_text(transaction *out, const char *text) {
    put_u32(out, (uint32_t)strlen(text));
    put(out, text, strlen(text));
}

lk_buffer lk_app_render_binary(lk_app *app) {
    transaction out = {.len = 0};
    put(&out, "LKB1", 4);

    if (app->rendered++) {
        int64_t none = -1;
        put(&out, &none, sizeof(none));
        put_u32(&out, 0);
    } else {
        int64_t root = 1;
        put(&out, &root, sizeof(root));
        put_u32(&out, 2);
        // Create: text 0, accessibility label 4; guest 2 adds background 6
        // and corner radius 11.
        put(&out, (const uint8_t[]){1}, 1);
        put_u32(&out, 1);
        put(&out, (const uint8_t[]){4}, 1);
        put(&out, "text", 4);
        int painted = app->identifier == 2;
        put_u32(&out, (1u << 0) | (1u << 4) | (painted ? (1u << 6) | (1u << 11) : 0));
        char text[32], label[32];
        snprintf(text, sizeof(text), "Guest %d", app->identifier);
        snprintf(label, sizeof(label), "guest-%d", app->identifier);
        put_text(&out, text);
        put_text(&out, label);

        if (painted) {
            put_u32(&out, 33953791u);
            float radius = 9999;
            put(&out, &radius, sizeof(radius));
        }

        // UpdateLayout 1: 0, 0, 200, 80.
        put(&out, (const uint8_t[]){3}, 1);
        put_u32(&out, 1);
        float frame[4] = {0, 0, 200, 80};
        put(&out, frame, sizeof(frame));
    }

    uint8_t *copy = malloc(out.len);
    memcpy(copy, out.data, out.len);

    return (lk_buffer){copy, out.len, copy};
}

lk_updates *lk_updates_open(const uint8_t *config, size_t config_len,
    const uint8_t *embedded, size_t embedded_len,
    const uint8_t *directory, size_t directory_len, lk_buffer *error) {
    return calloc(1, sizeof(lk_updates));
}
void lk_updates_destroy(lk_updates *updates) { free(updates); }
int32_t lk_updates_launch(lk_updates *updates, lk_buffer *wasm, lk_buffer *metadata) {
    updates->launch++;
    *wasm = bytes("fixture");
    *metadata = bytes(updates->launch == 1
        ? "{\"receipt\":\"first\",\"version\":\"1\",\"build\":1,\"candidate\":true}"
        : "{\"receipt\":\"next\",\"version\":\"2\",\"build\":2,\"candidate\":false}");

    return 0;
}
int32_t lk_updates_sync(lk_updates *updates, lk_buffer *metadata) {
    *metadata = bytes("{\"status\":\"staged\",\"version\":\"2\",\"build\":2}");

    return 0;
}

int32_t lk_updates_confirm(lk_updates *updates, const uint8_t *receipt, size_t len) { return 0; }
int32_t lk_updates_reject(lk_updates *updates, const uint8_t *receipt, size_t len) { return 0; }
lk_buffer lk_updates_error(lk_updates *updates) { return bytes("release fixture failure"); }
int32_t lk_bundle_entry(const uint8_t *embedded, size_t len, lk_buffer *wasm, lk_buffer *error) {
    *wasm = bytes("fixture");

    return 0;
}
void lk_buffer_release(lk_buffer buffer) { free(buffer.owner); }

struct lk_async_cancellation { atomic_int references; atomic_int cancelled; };
struct lk_async_job { lk_async_cancellation *token; };
static lk_async_job *pending_job;
static void *pending_context;
static void (*pending_callback)(void *, lk_worker_reply *);
static atomic_int async_started;
static atomic_int async_cancelled;

int fixture_async_started(void) { return atomic_load(&async_started); }
int fixture_async_cancelled(void) { return atomic_load(&async_cancelled); }

lk_async_job *lk_app_async(lk_app *app, const lk_request *request) {
    if (scenario != 3 || !app->service_polled || request->id != 1) abort();
    lk_async_job *job = calloc(1, sizeof(lk_async_job));
    job->token = calloc(1, sizeof(lk_async_cancellation));
    atomic_store(&job->token->references, 1);

    return job;
}

lk_async_cancellation *lk_async_cancellation_retain(const lk_async_job *job) {
    atomic_fetch_add(&job->token->references, 1);

    return job->token;
}

void lk_async_cancel(const lk_async_cancellation *token) {
    lk_async_cancellation *owned = (lk_async_cancellation *)token;

    if (owned && !atomic_exchange(&owned->cancelled, 1)) atomic_fetch_add(&async_cancelled, 1);
}

void lk_async_cancellation_release(lk_async_cancellation *token) {
    if (token && atomic_fetch_sub(&token->references, 1) == 1) free(token);
}

void lk_async_release(lk_async_job *job) {
    if (!job) return;
    lk_async_cancellation_release(job->token);
    free(job);
}

void lk_async_start(lk_async_job *job, void *context, void (*completed)(void *, lk_worker_reply *)) {
    pending_job = job;
    pending_context = context;
    pending_callback = completed;
    atomic_fetch_add(&async_started, 1);
}

static void *finish_async(void *unused) {
    lk_async_release(pending_job);
    pending_job = NULL;
    pending_callback(pending_context, NULL);

    return NULL;
}

void fixture_async_finish(void) {
    pthread_t thread;

    if (pthread_create(&thread, NULL, finish_async, NULL) != 0) abort();

    if (pthread_join(thread, NULL) != 0) abort();
}

/* The guest-thread ABI over the fixture app, run in place: each input does
 * its step at once, then notifies, as the guest thread does when a step ends.
 * The notification only posts a main-queue turn, so calling it here is safe. */
struct lk_guest {
    lk_app *app;
    lk_guest_callbacks callbacks;
    int render;
    int failed;
};

static int32_t guest_step(const lk_guest *guest) {
    lk_guest *owned = (lk_guest *)guest;

    if (!owned->failed && lk_app_poll_service(owned->app) < 0) owned->failed = 1;
    owned->render = 1;
    owned->callbacks.notify(owned->callbacks.context);

    return 0;
}

lk_guest *lk_guest_spawn(lk_app *app, lk_guest_callbacks callbacks) {
    lk_guest *guest = calloc(1, sizeof(lk_guest));
    guest->app = app;
    guest->callbacks = callbacks;

    return guest;
}

int32_t lk_guest_stop(lk_guest *guest) {
    guest->callbacks.release(guest->callbacks.context);
    free(guest);

    return 0;
}

void lk_guest_ack(const lk_guest *guest) {}
int32_t lk_guest_start(const lk_guest *guest, float width, float height) { return guest_step(guest); }
int32_t lk_guest_resize(const lk_guest *guest, float width, float height, bool dark) { return guest_step(guest); }
int32_t lk_guest_event(const lk_guest *guest, size_t node, uint32_t kind, const uint8_t *text, size_t len) {
    return 0;
}
/* The fixture guest never waits for a display frame, so the link stays paused. */
int32_t lk_guest_frame(const lk_guest *guest, uint64_t timestamp_ns, uint64_t period_ns) { return 0; }
int32_t lk_guest_frame_requested(const lk_guest *guest) { return 0; }
int32_t lk_guest_wake(const lk_guest *guest) { return guest_step(guest); }
int32_t lk_guest_complete(const lk_guest *guest, uint64_t id, int32_t error, const uint8_t *data, size_t len) {
    abort();
}
int32_t lk_guest_complete_worker(const lk_guest *guest, uint64_t id, lk_worker_reply *reply) { abort(); }

lk_buffer lk_guest_take_render(const lk_guest *guest) {
    lk_guest *owned = (lk_guest *)guest;

    if (!owned->render || owned->failed) return (lk_buffer){NULL, 0, NULL};
    owned->render = 0;

    return lk_app_render_binary(owned->app);
}

int32_t lk_guest_next_request(const lk_guest *guest, lk_request *out, lk_rust_worker **worker,
    lk_async_job **job) {
    if (guest->failed || lk_app_next_request(guest->app, out) != 1) return 0;
    *worker = lk_app_worker(guest->app, out);
    *job = lk_app_async(guest->app, out);

    return 1;
}

int64_t lk_guest_next_cancelled(const lk_guest *guest) { return lk_app_next_cancelled(guest->app); }
int32_t lk_guest_has_rive(const lk_guest *guest) { return 0; }
int32_t lk_guest_rive_tick(const lk_guest *guest, double elapsed, float scale, uint32_t flags,
    const size_t *visible, size_t count) {
    abort();
}
lk_buffer lk_guest_take_rive_frame(const lk_guest *guest, size_t *node, uint32_t *width, uint32_t *height) {
    return (lk_buffer){NULL, 0, NULL};
}
int32_t lk_guest_failed(const lk_guest *guest) { return guest->failed; }
lk_buffer lk_guest_error(const lk_guest *guest) { return lk_app_error(guest->app); }
