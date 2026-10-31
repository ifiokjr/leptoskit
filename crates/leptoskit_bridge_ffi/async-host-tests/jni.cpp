#include "async_test.h"
#include <jni.h>
#include <cstdlib>

jobject leptoskit_request(JNIEnv*, lk_bridge*, lk_request) { return nullptr; }
extern "C" lk_app* leptoskit_app_create_with_context(
    const uint8_t*, size_t, const lk_app_context*, lk_buffer*) { return nullptr; }
extern "C" void lk_buffer_release(lk_buffer) {}
extern "C" void lk_app_destroy(lk_app*) {}

extern "C" lk_bridge* lk_app_plugins(lk_app*) { return nullptr; }
extern "C" int32_t lk_app_enable_frames(lk_app*) { return 0; }
extern "C" lk_buffer lk_app_error(lk_app*) { return {}; }

// These callback ownership tests never run a guest thread.
extern "C" lk_guest* lk_guest_spawn(lk_app*, lk_guest_callbacks) { std::abort(); }
extern "C" int32_t lk_guest_stop(lk_guest*) { std::abort(); }
extern "C" void lk_guest_ack(const lk_guest*) { std::abort(); }
extern "C" int32_t lk_guest_start(const lk_guest*, float, float) { std::abort(); }
extern "C" int32_t lk_guest_resize(const lk_guest*, float, float, bool) { std::abort(); }
extern "C" int32_t lk_guest_event(const lk_guest*, size_t, uint32_t, const uint8_t*, size_t) { std::abort(); }
extern "C" int32_t lk_guest_frame(const lk_guest*, uint64_t, uint64_t) { std::abort(); }
extern "C" int32_t lk_guest_frame_requested(const lk_guest*) { std::abort(); }
extern "C" int32_t lk_guest_wake(const lk_guest*) { std::abort(); }
extern "C" int32_t lk_guest_complete(const lk_guest*, uint64_t, int32_t, const uint8_t*, size_t) {
    std::abort();
}
extern "C" lk_buffer lk_guest_take_render(const lk_guest*) { std::abort(); }
extern "C" int32_t lk_guest_next_request(const lk_guest*, lk_request*, lk_rust_worker**, lk_async_job**) {
    std::abort();
}
extern "C" int64_t lk_guest_next_cancelled(const lk_guest*) { std::abort(); }
extern "C" int32_t lk_guest_has_rive(const lk_guest*) { std::abort(); }
extern "C" int32_t lk_guest_rive_tick(const lk_guest*, double, float, uint32_t, const size_t*, size_t) {
    std::abort();
}
extern "C" lk_buffer lk_guest_take_rive_frame(const lk_guest*, size_t*, uint32_t*, uint32_t*) {
    std::abort();
}
extern "C" int32_t lk_guest_failed(const lk_guest*) { std::abort(); }
extern "C" lk_buffer lk_guest_error(const lk_guest*) { std::abort(); }

// Async callback controls cannot enter the independent application service lifecycle.
extern "C" int32_t lk_app_activate_service(lk_app*, lk_service_callbacks) { std::abort(); }
extern "C" int32_t lk_app_set_text_measurer(lk_app*, lk_text_measurer) { std::abort(); }
extern "C" lk_closing_app* lk_app_begin_close(lk_app*) { std::abort(); }
extern "C" int32_t lk_app_close_poll(lk_closing_app*) { std::abort(); }
extern "C" void lk_app_close_release(lk_closing_app*) { std::abort(); }

extern "C" JNIEXPORT jint JNICALL

Java_dev_leptoskit_mobile_AsyncTest_started(JNIEnv*, jclass) { return test_async_started(); }
extern "C" JNIEXPORT jint JNICALL

Java_dev_leptoskit_mobile_AsyncTest_cancelled(JNIEnv*, jclass) { return test_async_cancelled(); }
extern "C" JNIEXPORT jint JNICALL

Java_dev_leptoskit_mobile_AsyncTest_liveJobs(JNIEnv*, jclass) { return test_async_live_jobs(); }
extern "C" JNIEXPORT jint JNICALL

Java_dev_leptoskit_mobile_AsyncTest_liveReplies(JNIEnv*, jclass) { return test_async_live_replies(); }
extern "C" JNIEXPORT jint JNICALL

Java_dev_leptoskit_mobile_AsyncTest_liveTokens(JNIEnv*, jclass) { return test_async_live_tokens(); }
extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_AsyncTest_finish(JNIEnv*, jclass) { test_async_finish(); }
extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_AsyncTest_job(JNIEnv*, jclass, jint mode) {
    return static_cast<jlong>(reinterpret_cast<uintptr_t>(test_async_create(mode)));
}
