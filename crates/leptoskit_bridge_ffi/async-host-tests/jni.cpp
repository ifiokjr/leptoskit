#include "async_test.h"
#include <jni.h>
#include <cstdlib>

jobject leptoskit_request(JNIEnv*, lk_bridge*, lk_request) { return nullptr; }
extern "C" lk_app* leptoskit_app_create_with_context(
    const uint8_t*, size_t, const lk_app_context*, lk_buffer*) { return nullptr; }
extern "C" void lk_buffer_release(lk_buffer) {}
extern "C" void lk_app_destroy(lk_app*) {}

extern "C" lk_bridge* lk_app_plugins(lk_app*) { return nullptr; }
extern "C" int32_t lk_app_start(lk_app*, float, float) { return 0; }
extern "C" int32_t lk_app_resize(lk_app*, float, float, bool) { return 0; }
extern "C" int32_t lk_app_wake(lk_app*) { return 0; }
extern "C" int32_t lk_app_event(lk_app*, size_t, uint32_t, const uint8_t*, size_t) { return 0; }
extern "C" int32_t lk_app_render_pending(lk_app*) { return 0; }
extern "C" lk_buffer lk_app_render(lk_app*) { return {}; }
extern "C" lk_buffer lk_app_error(lk_app*) { return {}; }
extern "C" int32_t lk_app_next_request(lk_app*, lk_request*) { return 0; }

// Async callback controls cannot enter the independent application service lifecycle.
extern "C" int32_t lk_app_activate_service(lk_app*, lk_service_callbacks) { std::abort(); }
extern "C" int32_t lk_app_poll_service(lk_app*) { std::abort(); }
extern "C" int64_t lk_app_next_cancelled(lk_app*) { std::abort(); }
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
