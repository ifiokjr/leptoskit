#include "../include/leptoskit_bridge.h"
#include <jni.h>
#include <cstdint>
#include <string>

jobject leptoskit_request(JNIEnv*, lk_bridge*, lk_request);

namespace {
lk_app* app(jlong handle) {
    return reinterpret_cast<lk_app*>(static_cast<uintptr_t>(handle));
}
jbyteArray array(JNIEnv* env, lk_buffer buffer) {
    jbyteArray result = env->NewByteArray(static_cast<jsize>(buffer.len));
    if (result && buffer.len) {
        env->SetByteArrayRegion(result, 0, static_cast<jsize>(buffer.len),
                               reinterpret_cast<const jbyte*>(buffer.data));
    }
    lk_buffer_release(buffer);
    return result;
}
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeApp_create(JNIEnv* env, jclass, jbyteArray wasm) {
    if (!wasm) return 0;
    const auto size = env->GetArrayLength(wasm);
    auto* bytes = env->GetByteArrayElements(wasm, nullptr);
    if (!bytes) return 0;
    lk_buffer error{};
    auto* result = leptoskit_app_create(reinterpret_cast<uint8_t*>(bytes), size, &error);
    env->ReleaseByteArrayElements(wasm, bytes, JNI_ABORT);
    if (!result) {
        // Diagnostics are text controlled by the runtime, not modified UTF-8
        // input supplied by the guest.
        std::string message(reinterpret_cast<const char*>(error.data), error.len);
        lk_buffer_release(error);
        jclass failure = env->FindClass("java/lang/IllegalStateException");
        if (failure) env->ThrowNew(failure, message.c_str());
    }
    return static_cast<jlong>(reinterpret_cast<uintptr_t>(result));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeApp_destroy(JNIEnv*, jclass, jlong handle) {
    lk_app_destroy(app(handle));
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeApp_plugins(JNIEnv*, jclass, jlong handle) {
    return static_cast<jlong>(reinterpret_cast<uintptr_t>(lk_app_plugins(app(handle))));
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_start(JNIEnv*, jclass, jlong handle, jfloat width, jfloat height) {
    return lk_app_start(app(handle), width, height);
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_resize(JNIEnv*, jclass, jlong handle,
                                        jfloat width, jfloat height, jboolean dark) {
    return lk_app_resize(app(handle), width, height, dark == JNI_TRUE);
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_wake(JNIEnv*, jclass, jlong handle) {
    return lk_app_wake(app(handle));
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_event(JNIEnv* env, jclass, jlong handle,
                                       jlong node, jint kind, jbyteArray text) {
    if (!text || node < 0) return -1;
    const auto size = env->GetArrayLength(text);
    if (!size) return lk_app_event(app(handle), node, kind, nullptr, 0);
    auto* bytes = env->GetByteArrayElements(text, nullptr);
    if (!bytes) return -1;
    auto result = lk_app_event(app(handle), node, kind, reinterpret_cast<uint8_t*>(bytes), size);
    env->ReleaseByteArrayElements(text, bytes, JNI_ABORT);
    return result;
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_dev_leptoskit_mobile_NativeApp_render(JNIEnv* env, jclass, jlong handle) {
    return array(env, lk_app_render(app(handle)));
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_dev_leptoskit_mobile_NativeApp_error(JNIEnv* env, jclass, jlong handle) {
    return array(env, lk_app_error(app(handle)));
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_leptoskit_mobile_NativeApp_nextRequest(JNIEnv* env, jclass, jlong handle, jintArray status) {
    if (!status || env->GetArrayLength(status) != 1) return nullptr;
    lk_request request{};
    const jint result = lk_app_next_request(app(handle), &request);
    env->SetIntArrayRegion(status, 0, 1, &result);
    if (result != 1) return nullptr;
    return leptoskit_request(env, lk_app_plugins(app(handle)), request);
}
