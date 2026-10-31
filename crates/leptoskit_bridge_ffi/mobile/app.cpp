#include "../include/leptoskit_bridge.h"
#include <jni.h>
#include <cstdint>
#include <memory>
#include <new>
#include <cstdlib>
#include <string>

jobject leptoskit_request(JNIEnv*, lk_bridge*, lk_request);

namespace {
// Rust retains this storage until the final callback/waker has returned. It
// never borrows an App, a View, or a thread-local JNIEnv across callbacks.
struct ServiceContext {
    JavaVM* java_vm;
    jobject callback;
    jmethodID wake;
    jmethodID close;
};

class CallbackEnv {
public:
    explicit CallbackEnv(JavaVM* java_vm) : vm_(java_vm) {
        const auto status = vm_->GetEnv(reinterpret_cast<void**>(&env_), JNI_VERSION_1_6);

        if (status == JNI_OK) return;

        if (status != JNI_EDETACHED) std::abort();
        attach();
    }
    ~CallbackEnv() {
        // A callback failure cannot cross the C ABI or silently lose a join.
        if (env_->ExceptionCheck()) {
            env_->ExceptionDescribe();
            env_->FatalError("LeptosKit service callback failed");
        }

        if (attached_) vm_->DetachCurrentThread();
    }

    JNIEnv* get() const { return env_; }
private:
    void attach() {
#ifdef __ANDROID__
        const auto status = vm_->AttachCurrentThread(&env_, nullptr);
#else
        const auto status = vm_->AttachCurrentThread(reinterpret_cast<void**>(&env_), nullptr);
#endif

        if (status != JNI_OK) std::abort();
        attached_ = true;
    }

    JavaVM* vm_;
    JNIEnv* env_ = nullptr;
    bool attached_ = false;
};

void service_wake(void* opaque) {
    auto* context = static_cast<ServiceContext*>(opaque);
    CallbackEnv env(context->java_vm);
    env.get()->CallVoidMethod(context->callback, context->wake);
}

void service_close(void* opaque, lk_closing_app* owner) {
    auto* context = static_cast<ServiceContext*>(opaque);
    CallbackEnv env(context->java_vm);
    env.get()->CallVoidMethod(context->callback, context->close,
        static_cast<jlong>(reinterpret_cast<uintptr_t>(owner)));
}

void service_release(void* opaque) {
    auto* context = static_cast<ServiceContext*>(opaque);
    CallbackEnv env(context->java_vm);
    env.get()->DeleteGlobalRef(context->callback);
    delete context;
}

struct TextContext {
    JavaVM* java_vm;
    jobject measurer;
    jmethodID measure;
};

bool text_measure(void* opaque, const uint8_t* text, size_t len, float font_size,
    uint32_t font_weight, float width, bool rtl, float* out) {
    auto* context = static_cast<TextContext*>(opaque);
    CallbackEnv scope(context->java_vm);
    JNIEnv* env = scope.get();
    bool measured = false;
    // Bytes rather than a jstring: NewStringUTF expects modified UTF-8.
    jbyteArray bytes = env->NewByteArray(static_cast<jsize>(len));

    if (bytes) {
        env->SetByteArrayRegion(bytes, 0, static_cast<jsize>(len), reinterpret_cast<const jbyte*>(text));
        auto size = static_cast<jfloatArray>(env->CallObjectMethod(context->measurer, context->measure,
            bytes, font_size, static_cast<jint>(font_weight), width, static_cast<jboolean>(rtl)));

        if (size && !env->ExceptionCheck() && env->GetArrayLength(size) == 2) {
            env->GetFloatArrayRegion(size, 0, 2, out);
            measured = !env->ExceptionCheck();
        }

        if (size) env->DeleteLocalRef(size);
        env->DeleteLocalRef(bytes);
    }

    if (env->ExceptionCheck()) {
        // Layout approximates this label instead; the exception is logged.
        env->ExceptionDescribe();
        env->ExceptionClear();
        measured = false;
    }

    return measured;
}

void text_release(void* opaque) {
    auto* context = static_cast<TextContext*>(opaque);
    CallbackEnv env(context->java_vm);
    env.get()->DeleteGlobalRef(context->measurer);
    delete context;
}

lk_app* app(jlong handle) {
    return reinterpret_cast<lk_app*>(static_cast<uintptr_t>(handle));
}

struct RiveContext {
    JNIEnv* env;
    jobject renderer;
    jmethodID visible;
    jmethodID present;
};

int32_t rive_visible(void* opaque, size_t node) {
    auto* context = static_cast<RiveContext*>(opaque);
    auto* env = context->env;
    const auto visible = env->CallBooleanMethod(context->renderer, context->visible, static_cast<jlong>(node));
    return env->ExceptionCheck() ? -1 : (visible ? 1 : 0);
}

int32_t rive_present(void* opaque, size_t node, uint32_t width, uint32_t height,
                     const uint8_t* pixels, size_t length) {
    auto* context = static_cast<RiveContext*>(opaque);
    auto* env = context->env;
    // Java receives only a read-only view and copies synchronously. These local
    // references are deleted before Rust is allowed to reuse the frame buffer.
    auto buffer = env->NewDirectByteBuffer(const_cast<uint8_t*>(pixels), static_cast<jlong>(length));
    if (!buffer) return -1;
    auto type = env->GetObjectClass(buffer);
    if (!type) { env->DeleteLocalRef(buffer); return -1; }
    auto readonly_method = env->GetMethodID(type, "asReadOnlyBuffer", "()Ljava/nio/ByteBuffer;");
    auto readonly = readonly_method ? env->CallObjectMethod(buffer, readonly_method) : nullptr;
    jboolean success = false;
    if (readonly && !env->ExceptionCheck()) {
        success = env->CallBooleanMethod(context->renderer, context->present,
            static_cast<jlong>(node), static_cast<jint>(width), static_cast<jint>(height), readonly);
    }
    if (readonly) env->DeleteLocalRef(readonly);
    env->DeleteLocalRef(type);
    env->DeleteLocalRef(buffer);
    return env->ExceptionCheck() ? -1 : (success ? 1 : -1);
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

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_leptoskit_mobile_NativeApp_hasRive(JNIEnv*, jclass, jlong handle) {
    return lk_app_has_rive(app(handle)) == 1;
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_riveTick(JNIEnv* env, jclass, jlong handle,
    jdouble elapsed, jfloat scale, jint flags, jobject renderer) {
    if (!renderer) return -1;
    auto type = env->GetObjectClass(renderer);
    if (!type) return -1;
    auto visible = env->GetMethodID(type, "riveVisible", "(J)Z");
    auto present = visible ? env->GetMethodID(type, "presentRive", "(JIILjava/nio/ByteBuffer;)Z") : nullptr;
    env->DeleteLocalRef(type);
    if (!visible || !present) return -1;
    RiveContext context{env, renderer, visible, present};
    return lk_app_rive_tick(app(handle), elapsed, scale, static_cast<uint32_t>(flags),
        &context, rive_visible, rive_present);
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeApp_create(JNIEnv* env, jclass, jbyteArray wasm,
    jbyteArray data_path, jbyteArray cache_path) {
    if (!wasm || !data_path || !cache_path) return 0;
    const auto size = env->GetArrayLength(wasm);
    auto* bytes = env->GetByteArrayElements(wasm, nullptr);

    if (!bytes) return 0;
    const auto data_size = env->GetArrayLength(data_path);
    const auto cache_size = env->GetArrayLength(cache_path);

    std::string data(data_size, '\0');
    std::string cache(cache_size, '\0');
    env->GetByteArrayRegion(data_path, 0, data_size, reinterpret_cast<jbyte*>(data.data()));

    if (env->ExceptionCheck()) {
        env->ReleaseByteArrayElements(wasm, bytes, JNI_ABORT);

        return 0;
    }

    env->GetByteArrayRegion(cache_path, 0, cache_size, reinterpret_cast<jbyte*>(cache.data()));

    if (env->ExceptionCheck()) {
        env->ReleaseByteArrayElements(wasm, bytes, JNI_ABORT);

        return 0;
    }

    lk_app_context context{
        reinterpret_cast<const uint8_t*>(data.data()), data.size(),
        reinterpret_cast<const uint8_t*>(cache.data()), cache.size()
    };
    lk_buffer error{};
    auto* result = leptoskit_app_create_with_context(
        reinterpret_cast<uint8_t*>(bytes), size, &context, &error);
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

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_activateService(JNIEnv* env, jclass, jlong handle, jobject callback) {
    if (!callback) return -1;
    JavaVM* java_vm = nullptr;

    if (env->GetJavaVM(&java_vm) != JNI_OK) return -1;
    jclass type = env->GetObjectClass(callback);

    if (!type) return -1;
    const auto wake = env->GetMethodID(type, "wake", "()V");
    const auto close = env->ExceptionCheck() ? nullptr : env->GetMethodID(type, "close", "(J)V");
    env->DeleteLocalRef(type);

    if (!wake || !close) return -1;
    jobject retained = env->NewGlobalRef(callback);

    if (!retained) return -1;
    auto* context = new ServiceContext{java_vm, retained, wake, close};
    // This always-valid table consumes context on every return path.
    return lk_app_activate_service(app(handle), {context, service_wake, service_close, service_release});
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_setTextMeasurer(JNIEnv* env, jclass, jlong handle, jobject measurer) {
    if (!measurer) return -1;
    JavaVM* java_vm = nullptr;

    if (env->GetJavaVM(&java_vm) != JNI_OK) return -1;
    jclass type = env->GetObjectClass(measurer);

    if (!type) return -1;
    const auto measure = env->GetMethodID(type, "measure", "([BFIFZ)[F");
    env->DeleteLocalRef(type);

    if (!measure) return -1;
    jobject retained = env->NewGlobalRef(measurer);

    if (!retained) return -1;
    auto* context = new TextContext{java_vm, retained, measure};
    // The table is valid, so the app owns context from here on.
    return lk_app_set_text_measurer(app(handle), {context, text_measure, text_release});
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_pollService(JNIEnv*, jclass, jlong handle) {
    return lk_app_poll_service(app(handle));
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeApp_nextCancelled(JNIEnv*, jclass, jlong handle) {
    return lk_app_next_cancelled(app(handle));
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeApp_beginClose(JNIEnv*, jclass, jlong handle) {
    return static_cast<jlong>(reinterpret_cast<uintptr_t>(lk_app_begin_close(app(handle))));
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_pollClose(JNIEnv*, jclass, jlong handle) {
    return lk_app_close_poll(reinterpret_cast<lk_closing_app*>(static_cast<uintptr_t>(handle)));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeApp_releaseClose(JNIEnv*, jclass, jlong handle) {
    lk_app_close_release(reinterpret_cast<lk_closing_app*>(static_cast<uintptr_t>(handle)));
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
Java_dev_leptoskit_mobile_NativeApp_enableFrames(JNIEnv*, jclass, jlong handle) {
    return lk_app_enable_frames(app(handle));
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_frameRequested(JNIEnv*, jclass, jlong handle) {
    return lk_app_frame_requested(app(handle));
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_frame(JNIEnv*, jclass, jlong handle,
    jlong timestamp, jlong period) {
    if (timestamp < 0 || period < 0) return -1;
    return lk_app_frame(app(handle), static_cast<uint64_t>(timestamp), static_cast<uint64_t>(period));
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
Java_dev_leptoskit_mobile_NativeApp_renderBinary(JNIEnv* env, jclass, jlong handle) {
    return array(env, lk_app_render_binary(app(handle)));
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_renderPending(JNIEnv*, jclass, jlong handle) {
    return lk_app_render_pending(app(handle));
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
        if (!value_) return;
        size_ = env_->GetArrayLength(value_);

        if (size_) bytes_ = env_->GetByteArrayElements(value_, nullptr);
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

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeApp_worker(
    JNIEnv*, jclass, jlong handle, jint plugin, jint method, jlong owner) {
    lk_request request{};
    request.plugin = static_cast<uint32_t>(plugin);
    request.method = static_cast<uint32_t>(method);
    request.payload.owner = reinterpret_cast<void*>(static_cast<uintptr_t>(owner));

    return static_cast<jlong>(reinterpret_cast<uintptr_t>(lk_app_worker(app(handle), &request)));
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeApp_executeWorker(JNIEnv*, jclass, jlong handle) {
    auto* worker = reinterpret_cast<lk_rust_worker*>(static_cast<uintptr_t>(handle));

    return static_cast<jlong>(reinterpret_cast<uintptr_t>(lk_worker_execute(worker)));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeApp_releaseWorker(JNIEnv*, jclass, jlong handle) {
    lk_worker_release(reinterpret_cast<lk_rust_worker*>(static_cast<uintptr_t>(handle)));
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_completeWorker(
    JNIEnv*, jclass, jlong bridge, jlong id, jlong reply) {
    return lk_worker_complete(
        reinterpret_cast<lk_bridge*>(static_cast<uintptr_t>(bridge)),
        static_cast<uint64_t>(id),
        reinterpret_cast<lk_worker_reply*>(static_cast<uintptr_t>(reply)));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeApp_releaseWorkerReply(JNIEnv*, jclass, jlong handle) {
    lk_worker_reply_release(reinterpret_cast<lk_worker_reply*>(static_cast<uintptr_t>(handle)));
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeApp_workerCancellation(JNIEnv*, jclass, jlong handle) {
    auto* worker = reinterpret_cast<lk_rust_worker*>(static_cast<uintptr_t>(handle));

    return static_cast<jlong>(reinterpret_cast<uintptr_t>(lk_worker_cancellation(worker)));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeApp_cancelWorker(JNIEnv*, jclass, jlong handle) {
    lk_worker_cancel(reinterpret_cast<lk_worker_cancellation_token*>(static_cast<uintptr_t>(handle)));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeApp_releaseWorkerCancellation(JNIEnv*, jclass, jlong handle) {
    lk_worker_cancellation_release(
        reinterpret_cast<lk_worker_cancellation_token*>(static_cast<uintptr_t>(handle)));
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_dev_leptoskit_mobile_NativeUpdates_entry(JNIEnv* env, jclass, jbyteArray embedded) {
    BorrowedArray bundle(env, embedded);

    if (!bundle.valid()) {
        invalid_update_argument(env);

        return nullptr;
    }

    lk_buffer wasm{};
    lk_buffer error{};

    if (lk_bundle_entry(bundle.data(), bundle.size(), &wasm, &error) != 0) {
        update_failure(env, error);

        return nullptr;
    }

    return array(env, wasm);
}

namespace {
lk_dev* development(jlong handle) {
    return reinterpret_cast<lk_dev*>(static_cast<uintptr_t>(handle));
}
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeDevelopment_open(JNIEnv* env, jclass, jbyteArray url,
    jbyteArray running) {
    BorrowedArray address(env, url);

    if (!address.valid()) {
        invalid_update_argument(env);

        return 0;
    }

    BorrowedArray guest(env, running);

    if (!guest.valid()) {
        invalid_update_argument(env);

        return 0;
    }

    lk_buffer error{};
    auto* result = lk_dev_open(address.data(), address.size(), guest.data(), guest.size(), &error);

    if (!result) update_failure(env, error);

    return static_cast<jlong>(reinterpret_cast<uintptr_t>(result));
}

// A new guest's bytes, or null when nothing changed within the server's wait.
extern "C" JNIEXPORT jbyteArray JNICALL
Java_dev_leptoskit_mobile_NativeDevelopment_next(JNIEnv* env, jclass, jlong handle) {
    lk_buffer wasm{};

    switch (lk_dev_next(development(handle), &wasm)) {
    case 1:
        return array(env, wasm);
    case 0:
        return nullptr;
    default:
        update_failure(env, lk_dev_error(development(handle)));

        return nullptr;
    }
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeDevelopment_destroy(JNIEnv*, jclass, jlong handle) {
    lk_dev_destroy(development(handle));
}

namespace {
struct AsyncCompletion {
    JavaVM* vm;
    jobject receiver;
    jmethodID completed;
};

void async_completed(void* raw, lk_worker_reply* reply) {
    std::unique_ptr<AsyncCompletion> completion(static_cast<AsyncCompletion*>(raw));
    JNIEnv* env = nullptr;
    const auto status = completion->vm->GetEnv(reinterpret_cast<void**>(&env), JNI_VERSION_1_6);
    const bool detached = status == JNI_EDETACHED;

    if (detached) {
#ifdef __ANDROID__
        const auto attached = completion->vm->AttachCurrentThread(&env, nullptr);
#else

        const auto attached = completion->vm->AttachCurrentThread(reinterpret_cast<void**>(&env), nullptr);
#endif
        // Without an attached JVM there is no valid way to release its global
        // reference or resume its continuation. Do not silently strand the job.
        if (attached != JNI_OK) std::terminate();
    } else if (status != JNI_OK) {
        std::terminate();
    }

    env->CallVoidMethod(completion->receiver, completion->completed,
                        static_cast<jlong>(reinterpret_cast<uintptr_t>(reply)));

    if (env->ExceptionCheck()) {
        env->ExceptionDescribe();
        env->FatalError("LeptosKit async completion callback threw");
    }

    env->DeleteGlobalRef(completion->receiver);

    if (detached) completion->vm->DetachCurrentThread();
}
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeApp_asyncJob(
    JNIEnv*, jclass, jlong handle, jlong id, jint plugin, jint method, jlong owner) {
    lk_request request{};
    request.id = static_cast<uint64_t>(id);
    request.plugin = static_cast<uint32_t>(plugin);
    request.method = static_cast<uint32_t>(method);
    request.payload.owner = reinterpret_cast<void*>(static_cast<uintptr_t>(owner));

    return static_cast<jlong>(reinterpret_cast<uintptr_t>(lk_app_async(app(handle), &request)));
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeApp_asyncCancellation(JNIEnv*, jclass, jlong handle) {
    return static_cast<jlong>(reinterpret_cast<uintptr_t>(
        lk_async_cancellation_retain(reinterpret_cast<lk_async_job*>(static_cast<uintptr_t>(handle)))));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeApp_cancelAsync(JNIEnv*, jclass, jlong handle) {
    lk_async_cancel(reinterpret_cast<lk_async_cancellation*>(static_cast<uintptr_t>(handle)));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeApp_releaseAsyncCancellation(JNIEnv*, jclass, jlong handle) {
    lk_async_cancellation_release(
        reinterpret_cast<lk_async_cancellation*>(static_cast<uintptr_t>(handle)));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeApp_releaseAsync(JNIEnv*, jclass, jlong handle) {
    lk_async_release(reinterpret_cast<lk_async_job*>(static_cast<uintptr_t>(handle)));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeApp_startAsync(JNIEnv* env, jclass, jlong handle, jobject receiver) {
    auto* job = reinterpret_cast<lk_async_job*>(static_cast<uintptr_t>(handle));
    JavaVM* vm = nullptr;
    jclass type = env->GetObjectClass(receiver);
    jmethodID method = type ? env->GetMethodID(type, "completed", "(J)V") : nullptr;

    if (type) env->DeleteLocalRef(type);

    if (!method || env->GetJavaVM(&vm) != JNI_OK) {
        lk_async_release(job);

        if (!env->ExceptionCheck()) env->FatalError("LeptosKit async callback lookup failed");

        return;
    }

    jobject global = env->NewGlobalRef(receiver);

    if (!global) {
        lk_async_release(job);

        return; // NewGlobalRef leaves OutOfMemoryError pending.
    }

    auto* completion = new (std::nothrow) AsyncCompletion{vm, global, method};

    if (!completion) {
        env->DeleteGlobalRef(global);
        lk_async_release(job);
        jclass failure = env->FindClass("java/lang/OutOfMemoryError");

        if (failure) env->ThrowNew(failure, "Cannot allocate async callback context");

        return;
    }

    // Rust now owns job and completion. Callback may run before this returns.
    lk_async_start(job, completion, async_completed);
}
