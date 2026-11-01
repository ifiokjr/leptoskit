#include "../include/leptoskit_bridge.h"
#include <jni.h>
#include <cstdint>
#include <memory>
#include <new>
#include <pthread.h>
#include <vector>
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

// A thread attached here stays attached until it exits, then detaches. The
// guest thread measures text and notifies through here thousands of times a
// screen, and attaching costs more than the calls themselves.
pthread_key_t attached_threads;
pthread_once_t attached_threads_once = PTHREAD_ONCE_INIT;

void detach_thread(void* vm) { static_cast<JavaVM*>(vm)->DetachCurrentThread(); }

void create_attached_threads() {
    if (pthread_key_create(&attached_threads, detach_thread) != 0) std::abort();
}

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

        if (pthread_once(&attached_threads_once, create_attached_threads) != 0 ||
            pthread_setspecific(attached_threads, vm_) != 0) {
            std::abort();
        }
    }

    JavaVM* vm_;
    JNIEnv* env_ = nullptr;
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
Java_dev_leptoskit_mobile_NativeApp_enableFrames(JNIEnv*, jclass, jlong handle) {
    return lk_app_enable_frames(app(handle));
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_dev_leptoskit_mobile_NativeApp_error(JNIEnv* env, jclass, jlong handle) {
    return array(env, lk_app_error(app(handle)));
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
Java_dev_leptoskit_mobile_NativeApp_executeWorker(JNIEnv*, jclass, jlong handle) {
    auto* worker = reinterpret_cast<lk_rust_worker*>(static_cast<uintptr_t>(handle));

    return static_cast<jlong>(reinterpret_cast<uintptr_t>(lk_worker_execute(worker)));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeApp_releaseWorker(JNIEnv*, jclass, jlong handle) {
    lk_worker_release(reinterpret_cast<lk_rust_worker*>(static_cast<uintptr_t>(handle)));
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

// 0 once apps created afterwards report to the hub, -1 when this build has no
// DevTools or the hub is not on loopback.
extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeDevtools_hub(JNIEnv* env, jclass, jbyteArray url,
    jbyteArray app) {
    BorrowedArray address(env, url);
    BorrowedArray name(env, app);

    if (!address.valid() || !name.valid()) return -1;

    return lk_devtools_hub(address.data(), address.size(), name.data(), name.size());
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

namespace {
// leptoskit_bridge::Error::Native, for a call the platform cannot take.
constexpr jint native_error = 7;

// Retained by the guest until it stops; its notifications reach Kotlin here.
struct GuestContext {
    JavaVM* java_vm;
    jobject notifier;
    jmethodID notified;
};

void guest_notify(void* opaque) {
    auto* context = static_cast<GuestContext*>(opaque);
    CallbackEnv env(context->java_vm);
    env.get()->CallVoidMethod(context->notifier, context->notified);
}

void guest_release(void* opaque) {
    auto* context = static_cast<GuestContext*>(opaque);
    CallbackEnv env(context->java_vm);
    env.get()->DeleteGlobalRef(context->notifier);
    delete context;
}

lk_guest* guest(jlong handle) {
    return reinterpret_cast<lk_guest*>(static_cast<uintptr_t>(handle));
}
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestSpawn(JNIEnv* env, jclass, jlong handle, jobject notifier) {
    if (!notifier) return 0;
    JavaVM* java_vm = nullptr;

    if (env->GetJavaVM(&java_vm) != JNI_OK) return 0;
    jclass type = env->GetObjectClass(notifier);

    if (!type) return 0;
    const auto notified = env->GetMethodID(type, "notified", "()V");
    env->DeleteLocalRef(type);

    if (!notified) return 0;
    jobject retained = env->NewGlobalRef(notifier);

    if (!retained) return 0;
    auto* context = new GuestContext{java_vm, retained, notified};
    auto* result = lk_guest_spawn(app(handle), {context, guest_notify, guest_release});

    // A guest that cannot start leaves the context with us.
    if (!result) {
        env->DeleteGlobalRef(retained);
        delete context;
    }

    return static_cast<jlong>(reinterpret_cast<uintptr_t>(result));
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestStop(JNIEnv*, jclass, jlong handle) {
    return lk_guest_stop(guest(handle));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestAck(JNIEnv*, jclass, jlong handle) {
    lk_guest_ack(guest(handle));
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestStart(JNIEnv*, jclass, jlong handle, jfloat width, jfloat height) {
    return lk_guest_start(guest(handle), width, height);
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestResize(JNIEnv*, jclass, jlong handle,
    jfloat width, jfloat height, jboolean dark) {
    return lk_guest_resize(guest(handle), width, height, dark == JNI_TRUE);
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestEvent(JNIEnv* env, jclass, jlong handle,
    jlong node, jint kind, jbyteArray text) {
    BorrowedArray value(env, text);

    if (!value.valid() || node < 0) return -1;

    return lk_guest_event(guest(handle), static_cast<size_t>(node), static_cast<uint32_t>(kind),
        value.data(), value.size());
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestFrame(JNIEnv*, jclass, jlong handle,
    jlong timestamp, jlong period) {
    if (timestamp < 0 || period < 0) return -1;

    return lk_guest_frame(guest(handle), static_cast<uint64_t>(timestamp), static_cast<uint64_t>(period));
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestFrameRequested(JNIEnv*, jclass, jlong handle) {
    return lk_guest_frame_requested(guest(handle));
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestWake(JNIEnv*, jclass, jlong handle) {
    return lk_guest_wake(guest(handle));
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestComplete(JNIEnv* env, jclass, jlong handle,
    jlong id, jint error, jbyteArray payload) {
    if (error != 0) return lk_guest_complete(guest(handle), static_cast<uint64_t>(id), error, nullptr, 0);
    BorrowedArray value(env, payload);

    if (!value.valid()) return -1;

    return lk_guest_complete(guest(handle), static_cast<uint64_t>(id), 0, value.data(), value.size());
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestCompleteWorker(JNIEnv*, jclass, jlong handle, jlong id, jlong reply) {
    return lk_guest_complete_worker(guest(handle), static_cast<uint64_t>(id),
        reinterpret_cast<lk_worker_reply*>(static_cast<uintptr_t>(reply)));
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestTakeRender(JNIEnv* env, jclass, jlong handle) {
    return array(env, lk_guest_take_render(guest(handle)));
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestNextRequest(JNIEnv* env, jclass, jlong handle, jlongArray jobs) {
    if (!jobs || env->GetArrayLength(jobs) != 2) return nullptr;
    lk_request request{};
    lk_rust_worker* worker = nullptr;
    lk_async_job* job = nullptr;

    if (lk_guest_next_request(guest(handle), &request, &worker, &job) != 1) return nullptr;
    const auto id = request.id;
    // The request releases its own payload if it cannot be created.
    jobject value = leptoskit_request(env, nullptr, request);

    if (!value) {
        // Answer the call so the guest does not wait for it; an exception is pending.
        lk_worker_release(worker);
        lk_async_release(job);
        lk_guest_complete(guest(handle), id, native_error, nullptr, 0);

        return nullptr;
    }

    const jlong handles[2] = {
        static_cast<jlong>(reinterpret_cast<uintptr_t>(worker)),
        static_cast<jlong>(reinterpret_cast<uintptr_t>(job)),
    };
    env->SetLongArrayRegion(jobs, 0, 2, handles);

    return value;
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestNextCancelled(JNIEnv*, jclass, jlong handle) {
    return lk_guest_next_cancelled(guest(handle));
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestHasRive(JNIEnv*, jclass, jlong handle) {
    return lk_guest_has_rive(guest(handle)) == 1;
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestRiveTick(JNIEnv* env, jclass, jlong handle,
    jdouble elapsed, jfloat scale, jint flags, jlongArray visible) {
    if (!visible) return -1;
    const auto count = env->GetArrayLength(visible);
    std::vector<jlong> ids(static_cast<size_t>(count));
    env->GetLongArrayRegion(visible, 0, count, ids.data());

    if (env->ExceptionCheck()) return -1;
    std::vector<size_t> nodes(ids.begin(), ids.end());

    return lk_guest_rive_tick(guest(handle), elapsed, scale, static_cast<uint32_t>(flags),
        nodes.data(), nodes.size());
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestTakeRiveFrame(JNIEnv* env, jclass, jlong handle, jlongArray frame) {
    if (!frame || env->GetArrayLength(frame) != 3) return nullptr;
    size_t node = 0;
    uint32_t width = 0;
    uint32_t height = 0;
    const auto pixels = lk_guest_take_rive_frame(guest(handle), &node, &width, &height);

    if (pixels.len) {
        const jlong values[3] = {static_cast<jlong>(node), width, height};
        env->SetLongArrayRegion(frame, 0, 3, values);
    }

    return array(env, pixels);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestFailed(JNIEnv*, jclass, jlong handle) {
    return lk_guest_failed(guest(handle)) == 1;
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_dev_leptoskit_mobile_NativeApp_guestError(JNIEnv* env, jclass, jlong handle) {
    return array(env, lk_guest_error(guest(handle)));
}
