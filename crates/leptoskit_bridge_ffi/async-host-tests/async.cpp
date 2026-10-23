#include "async_test.h"
#include <atomic>
#include <cassert>
#include <cstdlib>
#include <memory>
#include <mutex>
#include <thread>
#include <vector>

struct lk_async_job {
    int mode;
    std::shared_ptr<std::atomic<bool>> cancelled;
};
struct lk_async_cancellation { std::shared_ptr<std::atomic<bool>> cancelled; };
struct lk_worker_reply {};
namespace {
struct Pending {
    lk_async_job* job;
    void* context;
    void (*completed)(void*, lk_worker_reply*);
};
std::atomic<int> started{0};
std::atomic<int> cancelled{0};
std::atomic<int> live_jobs{0};
std::atomic<int> live_replies{0};
std::atomic<int> live_tokens{0};
std::mutex pending_mutex;
std::vector<Pending> pending;
void complete(Pending item) {
    lk_async_release(item.job);
    live_replies.fetch_add(1);
    item.completed(item.context, new lk_worker_reply);
}
}
extern "C" lk_async_job* test_async_create(int mode) {
    live_jobs.fetch_add(1);

    return new lk_async_job{mode, std::make_shared<std::atomic<bool>>(false)};
}

extern "C" int test_async_started() { return started.load(); }
extern "C" int test_async_cancelled() { return cancelled.load(); }
extern "C" int test_async_live_jobs() { return live_jobs.load(); }
extern "C" int test_async_live_replies() { return live_replies.load(); }
extern "C" int test_async_live_tokens() { return live_tokens.load(); }
extern "C" void test_async_finish() {
    std::vector<Pending> drained;
    {
        std::lock_guard<std::mutex> lock(pending_mutex);
        drained.swap(pending);
    }

    for (auto item : drained) std::thread([item] { complete(item); }).join();
}
extern "C" lk_async_job* lk_app_async(lk_app*, const lk_request* request) {
    assert(request->id == 1); // JNI must preserve the call ID used for permission lookup.

    return test_async_create(request->method);
}
extern "C" lk_async_cancellation* lk_async_cancellation_retain(const lk_async_job* job) {
    live_tokens.fetch_add(1);

    return new lk_async_cancellation{job->cancelled};
}
extern "C" void lk_async_cancel(const lk_async_cancellation* token) {
    if (token && !token->cancelled->exchange(true)) cancelled.fetch_add(1);
}
extern "C" void lk_async_cancellation_release(lk_async_cancellation* token) {
    if (token) {
        live_tokens.fetch_sub(1);
        delete token;
    }
}
extern "C" void lk_async_release(lk_async_job* job) {
    if (job) {
        live_jobs.fetch_sub(1);
        delete job;
    }
}
extern "C" void lk_async_start(
    lk_async_job* job, void* context, void (*completed)(void*, lk_worker_reply*)) {
    started.fetch_add(1);
    Pending item{job, context, completed};

    if (job->mode == 0) {
        complete(item); // Exercise callback before start returns.
    } else {
        std::lock_guard<std::mutex> lock(pending_mutex);
        pending.push_back(item);
    }
}
extern "C" void lk_worker_reply_release(lk_worker_reply* reply) {
    if (reply) {
        live_replies.fetch_sub(1);
        delete reply;
    }
}
extern "C" int32_t lk_worker_complete(lk_bridge*, uint64_t, lk_worker_reply* reply) {
    lk_worker_reply_release(reply);

    return 0;
}
// Unrelated entry points are not invoked by these callback ownership tests.
extern "C" lk_rust_worker* lk_app_worker(lk_app*, const lk_request*) { return nullptr; }
extern "C" lk_worker_cancellation_token* lk_worker_cancellation(const lk_rust_worker*) { return nullptr; }
extern "C" void lk_worker_cancel(const lk_worker_cancellation_token*) {}
extern "C" void lk_worker_cancellation_release(lk_worker_cancellation_token*) {}

extern "C" lk_worker_reply* lk_worker_execute(lk_rust_worker*) { return nullptr; }
extern "C" void lk_worker_release(lk_rust_worker*) {}

// The shared JNI translation unit references updates, but these tests must
// never enter that unrelated API. Abort if the callback fixtures reach it.
extern "C" lk_updates* lk_updates_open(
    const uint8_t*, size_t, const uint8_t*, size_t, const uint8_t*, size_t, lk_buffer*) {
    std::abort();
}
extern "C" void lk_updates_destroy(lk_updates*) { std::abort(); }
extern "C" int32_t lk_updates_launch(lk_updates*, lk_buffer*, lk_buffer*) { std::abort(); }
extern "C" int32_t lk_updates_sync(lk_updates*, lk_buffer*) { std::abort(); }
extern "C" int32_t lk_updates_confirm(lk_updates*, const uint8_t*, size_t) { std::abort(); }
extern "C" int32_t lk_updates_reject(lk_updates*, const uint8_t*, size_t) { std::abort(); }
extern "C" lk_buffer lk_updates_error(lk_updates*) { std::abort(); }
extern "C" int32_t lk_bundle_entry(const uint8_t*, size_t, lk_buffer*, lk_buffer*) { std::abort(); }
