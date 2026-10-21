#include "worker_test.h"
#include <atomic>
#include <condition_variable>
#include <mutex>

struct lk_rust_worker {};
struct lk_worker_reply {};
struct lk_worker_cancellation { std::atomic<bool> cancelled{false}; };
namespace {
std::atomic<int> started{0};
std::atomic<int> released{0};
std::mutex gate_mutex;
std::condition_variable gate_changed;
bool unblocked = false;
}

extern "C" lk_rust_worker *test_worker_create() { return new lk_rust_worker; }
extern "C" lk_worker_cancellation_token *lk_worker_cancellation(const lk_rust_worker *) {
    return new lk_worker_cancellation_token;
}
extern "C" void lk_worker_cancel(const lk_worker_cancellation_token *token) {
    // The scheduler test exercises the Swift pre-execution cancellation gate.
    // Rust integration tests exercise cancellation while waiting on its mutex.
    (void)token;
}
extern "C" void lk_worker_cancellation_release(lk_worker_cancellation_token *token) { delete token; }
extern "C" int test_worker_started() { return started.load(); }
extern "C" int test_worker_released() { return released.load(); }
extern "C" void test_worker_unblock() {
    {
        std::lock_guard<std::mutex> lock(gate_mutex);
        unblocked = true;
    }
    gate_changed.notify_all();
}

extern "C" lk_worker_reply *lk_worker_execute(lk_rust_worker *worker) {
    started.fetch_add(1);
    {
        std::unique_lock<std::mutex> lock(gate_mutex);
        gate_changed.wait(lock, [] { return unblocked; });
    }
    lk_worker_release(worker);
    return new lk_worker_reply;
}

extern "C" void lk_worker_release(lk_rust_worker *worker) {
    if (worker) {
        released.fetch_add(1);
        delete worker;
    }
}

extern "C" void lk_worker_reply_release(lk_worker_reply *reply) { delete reply; }
extern "C" int32_t lk_worker_complete(lk_bridge *, uint64_t, lk_worker_reply *reply) {
    delete reply;
    return 0;
}
