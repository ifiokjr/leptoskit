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
    return env->ExceptionCheck() ? nullptr : result;
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

namespace {
lk_updates* updates(jlong handle) {
    return reinterpret_cast<lk_updates*>(static_cast<uintptr_t>(handle));
}

void update_failure(JNIEnv* env, lk_buffer diagnostic) {
    std::string message(reinterpret_cast<const char*>(diagnostic.data), diagnostic.len);
    lk_buffer_release(diagnostic);
    jclass failure = env->FindClass("java/lang/IllegalStateException");
    if (failure) {
        env->ThrowNew(failure, message.c_str());
        env->DeleteLocalRef(failure);
    }
}

// JNI borrows never survive a call. Rust performs bounded copies/validation
// before creating its owner; no native renderer handle reaches this worker.
class BorrowedArray {
public:
    BorrowedArray(JNIEnv* env, jbyteArray value) : env_(env), value_(value) {
        if (value_) {
            size_ = env_->GetArrayLength(value_);
            if (size_) bytes_ = env_->GetByteArrayElements(value_, nullptr);
        }
    }
    ~BorrowedArray() {
        if (bytes_) env_->ReleaseByteArrayElements(value_, bytes_, JNI_ABORT);
    }
    BorrowedArray(const BorrowedArray&) = delete;
    BorrowedArray& operator=(const BorrowedArray&) = delete;
    bool valid() const { return value_ && (size_ == 0 || bytes_); }
    const uint8_t* data() const { return reinterpret_cast<const uint8_t*>(bytes_); }
    size_t size() const { return static_cast<size_t>(size_); }
private:
    JNIEnv* env_;
    jbyteArray value_;
    jbyte* bytes_ = nullptr;
    jsize size_ = 0;
};

void invalid_update_argument(JNIEnv* env) {
    if (env->ExceptionCheck()) return;
    jclass failure = env->FindClass("java/lang/IllegalStateException");
    if (failure) {
        env->ThrowNew(failure, "update inputs must be nonnull byte arrays");
        env->DeleteLocalRef(failure);
    }
}
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeUpdates_open(JNIEnv* env, jclass, jbyteArray configuration,
                                          jbyteArray embedded, jbyteArray directory) {
    BorrowedArray config(env, configuration);
    if (!config.valid()) {
        invalid_update_argument(env);
        return 0;
    }
    BorrowedArray bundle(env, embedded);
    if (!bundle.valid()) {
        invalid_update_argument(env);
        return 0;
    }
    BorrowedArray path(env, directory);
    if (!path.valid()) {
        invalid_update_argument(env);
        return 0;
    }
    lk_buffer error{};
    auto* result = lk_updates_open(config.data(), config.size(), bundle.data(), bundle.size(),
                                   path.data(), path.size(), &error);
    if (!result) update_failure(env, error);
    return static_cast<jlong>(reinterpret_cast<uintptr_t>(result));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeUpdates_destroy(JNIEnv*, jclass, jlong handle) {
    lk_updates_destroy(updates(handle));
}

extern "C" JNIEXPORT jobjectArray JNICALL
Java_dev_leptoskit_mobile_NativeUpdates_launch(JNIEnv* env, jclass, jlong handle) {
    lk_buffer wasm{};
    lk_buffer metadata{};
    if (lk_updates_launch(updates(handle), &wasm, &metadata) != 0) {
        update_failure(env, lk_updates_error(updates(handle)));
        return nullptr;
    }
    jclass byte_array = env->FindClass("[B");
    if (!byte_array) {
        lk_buffer_release(wasm);
        lk_buffer_release(metadata);
        return nullptr;
    }
    jobjectArray result = env->NewObjectArray(2, byte_array, nullptr);
    env->DeleteLocalRef(byte_array);
    if (!result) {
        lk_buffer_release(wasm);
        lk_buffer_release(metadata);
        return nullptr;
    }
    jbyteArray wasm_array = array(env, wasm);
    if (!wasm_array) {
        lk_buffer_release(metadata);
        return nullptr;
    }
    jbyteArray metadata_array = array(env, metadata);
    if (!metadata_array) {
        env->DeleteLocalRef(wasm_array);
        return nullptr;
    }
    env->SetObjectArrayElement(result, 0, wasm_array);
    if (!env->ExceptionCheck()) env->SetObjectArrayElement(result, 1, metadata_array);
    env->DeleteLocalRef(wasm_array);
    env->DeleteLocalRef(metadata_array);
    return result;
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_dev_leptoskit_mobile_NativeUpdates_sync(JNIEnv* env, jclass, jlong handle) {
    lk_buffer metadata{};
    if (lk_updates_sync(updates(handle), &metadata) != 0) {
        update_failure(env, lk_updates_error(updates(handle)));
        return nullptr;
    }
    return array(env, metadata);
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeUpdates_confirm(JNIEnv* env, jclass, jlong handle,
                                             jbyteArray receipt) {
    BorrowedArray value(env, receipt);
    if (!value.valid()) {
        invalid_update_argument(env);
        return;
    }
    if (lk_updates_confirm(updates(handle), value.data(), value.size()) != 0) {
        update_failure(env, lk_updates_error(updates(handle)));
    }
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeUpdates_reject(JNIEnv* env, jclass, jlong handle,
                                            jbyteArray receipt) {
    BorrowedArray value(env, receipt);
    if (!value.valid()) {
        invalid_update_argument(env);
        return;
    }
    if (lk_updates_reject(updates(handle), value.data(), value.size()) != 0) {
        update_failure(env, lk_updates_error(updates(handle)));
    }
}
