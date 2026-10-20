#include "../include/leptoskit_bridge.hpp"
#include <jni.h>
#include <cstdint>

namespace {
jclass request_class = nullptr;
jmethodID request_constructor = nullptr;

lk_bridge* bridge(jlong handle) {
    return reinterpret_cast<lk_bridge*>(static_cast<uintptr_t>(handle));
}
}

extern "C" JNIEXPORT jint JNICALL JNI_OnLoad(JavaVM* vm, void*) {
    JNIEnv* env = nullptr;
    if (vm->GetEnv(reinterpret_cast<void**>(&env), JNI_VERSION_1_6) != JNI_OK) {
        return JNI_ERR;
    }
    jclass local = env->FindClass("dev/leptoskit/bridge/NativeRequest");
    if (!local) return JNI_ERR;
    request_class = static_cast<jclass>(env->NewGlobalRef(local));
    env->DeleteLocalRef(local);
    if (!request_class) return JNI_ERR;
    request_constructor = env->GetMethodID(request_class, "<init>", "(JIILjava/nio/ByteBuffer;J)V");
    if (!request_constructor) {
        env->DeleteGlobalRef(request_class);
        request_class = nullptr;
        return JNI_ERR;
    }
    return JNI_VERSION_1_6;
}

extern "C" JNIEXPORT void JNICALL JNI_OnUnload(JavaVM* vm, void*) {
    JNIEnv* env = nullptr;
    if (vm->GetEnv(reinterpret_cast<void**>(&env), JNI_VERSION_1_6) == JNI_OK && request_class) {
        env->DeleteGlobalRef(request_class);
        request_class = nullptr;
    }
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_bridge_NativeBridge_create(JNIEnv*, jclass) {
    return static_cast<jlong>(reinterpret_cast<uintptr_t>(lk_bridge_create()));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_bridge_NativeBridge_destroy(JNIEnv*, jclass, jlong handle) {
    lk_bridge_destroy(bridge(handle));
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_bridge_NativeBridge_register(
    JNIEnv* env, jclass, jlong handle, jbyteArray name, jintArray methods) {
    if (!name || !methods) return -3;
    const auto name_len = env->GetArrayLength(name);
    const auto method_count = env->GetArrayLength(methods);
    if (name_len > 256 || method_count > 4096) return -5;
    auto* name_bytes = env->GetByteArrayElements(name, nullptr);
    if (!name_bytes) return -7;
    auto* method_ids = env->GetIntArrayElements(methods, nullptr);
    if (!method_ids) {
        env->ReleaseByteArrayElements(name, name_bytes, JNI_ABORT);
        return -7;
    }
    const auto result = lk_bridge_register(
        bridge(handle), reinterpret_cast<const uint8_t*>(name_bytes), name_len,
        reinterpret_cast<const uint32_t*>(method_ids), method_count);
    env->ReleaseIntArrayElements(methods, method_ids, JNI_ABORT);
    env->ReleaseByteArrayElements(name, name_bytes, JNI_ABORT);
    return result;
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_bridge_NativeBridge_grant(
    JNIEnv*, jclass, jlong handle, jint plugin, jint method, jboolean allowed) {
    return lk_bridge_grant(bridge(handle), plugin, method, allowed == JNI_TRUE);
}

jobject leptoskit_request(JNIEnv* env, lk_bridge* host, lk_request request) {
    // The request owns the Rust allocation. The direct ByteBuffer is a view,
    // not another byte array. Kotlin closes the owner after suspend dispatch.
    jobject bytes = env->NewDirectByteBuffer(
        const_cast<uint8_t*>(request.payload.data), static_cast<jlong>(request.payload.len));
    if (!bytes) {
        lk_buffer_release(request.payload);
        lk_bridge_cancel(host, request.id);
        return nullptr;
    }
    jobject value = env->NewObject(
        request_class, request_constructor, static_cast<jlong>(request.id),
        static_cast<jint>(request.plugin), static_cast<jint>(request.method), bytes,
        static_cast<jlong>(reinterpret_cast<uintptr_t>(request.payload.owner)));
    env->DeleteLocalRef(bytes);
    if (!value) {
        lk_buffer_release(request.payload);
        lk_bridge_cancel(host, request.id);
    }
    return value;
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_leptoskit_bridge_NativeBridge_nextRequest(JNIEnv* env, jclass, jlong handle) {
    lk_request request{};
    const auto result = lk_bridge_next_request(bridge(handle), &request);
    if (result < 0) {
        jclass exception = env->FindClass("java/lang/IllegalArgumentException");
        if (exception) env->ThrowNew(exception, "Invalid LeptosKit bridge");
        return nullptr;
    }
    if (result == 0) return nullptr;
    return leptoskit_request(env, bridge(handle), request);
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_bridge_NativeBridge_complete(
    JNIEnv* env, jclass, jlong handle, jlong id, jint error, jbyteArray payload) {
    if (error != 0) return lk_bridge_complete(bridge(handle), id, error, nullptr, 0);
    if (!payload) return -3;
    const auto len = env->GetArrayLength(payload);
    if (len == 0) return lk_bridge_complete(bridge(handle), id, 0, nullptr, 0);
    if (len > 8 * 1024 * 1024) {
        return lk_bridge_complete(bridge(handle), id, 5, nullptr, 0);
    }
    auto* bytes = env->GetByteArrayElements(payload, nullptr);
    if (!bytes) return -7;
    const auto result = lk_bridge_complete(
        bridge(handle), id, 0, reinterpret_cast<const uint8_t*>(bytes), len);
    env->ReleaseByteArrayElements(payload, bytes, JNI_ABORT);
    return result;
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_bridge_NativeBridge_cancel(JNIEnv*, jclass, jlong handle, jlong id) {
    return lk_bridge_cancel(bridge(handle), id);
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_bridge_NativeBridge_nextCancelled(JNIEnv*, jclass, jlong handle) {
    return lk_bridge_next_cancelled(bridge(handle));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_bridge_NativeBridge_release(JNIEnv*, jclass, jlong owner) {
    lk_buffer_release({nullptr, 0, reinterpret_cast<void*>(static_cast<uintptr_t>(owner))});
}

extern "C" JNIEXPORT jlong JNICALL
Java_dev_leptoskit_bridge_NativeBridge_submit(
    JNIEnv* env, jclass, jlong handle, jint plugin, jint method, jbyteArray payload) {
    if (!payload) return -3;
    const auto len = env->GetArrayLength(payload);
    if (len == 0) return lk_bridge_submit(bridge(handle), plugin, method, nullptr, 0);
    if (len > 8 * 1024 * 1024) return -5;
    auto* bytes = env->GetByteArrayElements(payload, nullptr);
    if (!bytes) return -7;
    const auto result = lk_bridge_submit(
        bridge(handle), plugin, method, reinterpret_cast<const uint8_t*>(bytes), len);
    env->ReleaseByteArrayElements(payload, bytes, JNI_ABORT);
    return result;
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_dev_leptoskit_bridge_NativeBridge_takeResult(JNIEnv* env, jclass, jlong handle, jlong id) {
    lk_buffer buffer{};
    const auto result = lk_bridge_take_result(bridge(handle), id, &buffer);
    if (result == 0) return nullptr;
    if (result < 0) {
        jclass exception = env->FindClass("dev/leptoskit/bridge/PluginException");
        if (!exception) return nullptr;
        jmethodID constructor = env->GetMethodID(exception, "<init>", "(I)V");
        if (constructor) {
            jobject value = env->NewObject(exception, constructor, -result);
            if (value) {
                env->Throw(static_cast<jthrowable>(value));
                env->DeleteLocalRef(value);
            }
        }
        env->DeleteLocalRef(exception);
        return nullptr;
    }
    jbyteArray output = env->NewByteArray(static_cast<jsize>(buffer.len));
    if (output && buffer.len != 0) {
        env->SetByteArrayRegion(
            output, 0, static_cast<jsize>(buffer.len), reinterpret_cast<const jbyte*>(buffer.data));
    }
    lk_buffer_release(buffer);
    return output;
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_leptoskit_bridge_NativeBridge_emit(
    JNIEnv* env, jclass, jlong handle, jint plugin, jint kind, jbyteArray payload) {
    if (!payload) return -3;
    const auto len = env->GetArrayLength(payload);
    if (len == 0) return lk_bridge_emit(bridge(handle), plugin, kind, nullptr, 0);
    if (len > 8 * 1024 * 1024) return -5;
    auto* bytes = env->GetByteArrayElements(payload, nullptr);
    if (!bytes) return -7;
    const auto result = lk_bridge_emit(
        bridge(handle), plugin, kind, reinterpret_cast<const uint8_t*>(bytes), len);
    env->ReleaseByteArrayElements(payload, bytes, JNI_ABORT);
    return result;
}
