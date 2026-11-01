// Every runtime belongs to one ArkTS object on the UI thread. Native pointers
// never cross the JS boundary; napi_wrap supplies ownership and GC cleanup.
#include "napi/native_api.h"
#include "leptoskit_bridge.h"
#include <multimedia/image_framework/image/pixelmap_native.h>

#include <algorithm>
#include <cmath>
#include <cstring>
#include <exception>
#include <limits>
#include <memory>
#include <stdexcept>
#include <string>

#include <thread>
#include <unordered_set>
#include <vector>

namespace {
// Requests and cancellations drained per batch before yielding to the UI.
constexpr uint32_t kMaxBatch = 64;
// Bridge call IDs are exact JavaScript numbers below 2^53.
constexpr double kMaxSafeInteger = 9007199254740991.0;
// leptoskit_bridge::Error::InvalidCall: the guest cancelled first.
constexpr int32_t kInvalidCall = -8;

void Check(napi_status status) {
    if (status != napi_ok) throw std::runtime_error("HarmonyOS Node-API operation failed");
}

std::string Take(lk_buffer buffer) {
    struct Release {
        lk_buffer value;
        ~Release() { lk_buffer_release(value); }
    } release{buffer};

    return buffer.len == 0 ? std::string{} :
        std::string(reinterpret_cast<const char *>(buffer.data), buffer.len);
}

std::string String(napi_env env, napi_value value) {
    size_t size = 0;
    Check(napi_get_value_string_utf8(env, value, nullptr, 0, &size));

    if (size > 32 * 1024) throw std::runtime_error("Native string exceeds 32 KiB");
    std::vector<char> bytes(size + 1);
    Check(napi_get_value_string_utf8(env, value, bytes.data(), bytes.size(), &size));

    return {bytes.data(), size};
}

double Number(napi_env env, napi_value value) {
    double result = 0;
    Check(napi_get_value_double(env, value, &result));

    if (!std::isfinite(result)) throw std::runtime_error("Expected a finite number");

    return result;
}

double Integer(napi_env env, napi_value value, double low, double high) {
    const double result = Number(env, value);

    if (result < low || result > high || std::floor(result) != result) {
        throw std::runtime_error("Expected an integer in range");
    }

    return result;
}

napi_value Unsigned(napi_env env, double value) {
    napi_value result;
    Check(napi_create_double(env, value, &result));

    return result;
}

// Copies a Rust request payload into an ArkTS-owned ArrayBuffer.
napi_value Copy(napi_env env, lk_buffer buffer) {
    struct Release {
        lk_buffer value;
        ~Release() { lk_buffer_release(value); }
    } release{buffer};

    void *data = nullptr;
    napi_value result;
    Check(napi_create_arraybuffer(env, buffer.len, &data, &result));

    if (buffer.len > 0) std::memcpy(data, buffer.data, buffer.len);

    return result;
}

void ImageStatus(Image_ErrorCode status) {
    if (status != IMAGE_SUCCESS) throw std::runtime_error("Native Rive PixelMap operation failed");
}

struct RiveImage {
    OH_PixelmapNative* pixels = nullptr;

    RiveImage(uint32_t width, uint32_t height) {
        OH_Pixelmap_InitializationOptions* raw = nullptr;
        ImageStatus(OH_PixelmapInitializationOptions_Create(&raw));
        std::unique_ptr<OH_Pixelmap_InitializationOptions,
            decltype(&OH_PixelmapInitializationOptions_Release)> options(raw, OH_PixelmapInitializationOptions_Release);
        ImageStatus(OH_PixelmapInitializationOptions_SetWidth(raw, width));
        ImageStatus(OH_PixelmapInitializationOptions_SetHeight(raw, height));
        ImageStatus(OH_PixelmapInitializationOptions_SetPixelFormat(raw, PIXEL_FORMAT_RGBA_8888));
        ImageStatus(OH_PixelmapInitializationOptions_SetAlphaType(raw, PIXELMAP_ALPHA_TYPE_PREMULTIPLIED));

        // API 12 creates editable empty maps. WritePixels copies raw bytes,
        // avoiding source-format conversion or premultiplying them a second time.
        const auto status = OH_PixelmapNative_CreateEmptyPixelmap(raw, &pixels);

        if (status != IMAGE_SUCCESS) {
            if (pixels) OH_PixelmapNative_Release(pixels);
            pixels = nullptr;
            ImageStatus(status);
        }
    }

    ~RiveImage() { if (pixels) OH_PixelmapNative_Release(pixels); }
};

struct Runtime {
    lk_app *app = nullptr;
    // Plugins registered from ArkTS; every other request fails closed.
    std::unordered_set<uint32_t> plugins;
    const std::thread::id thread = std::this_thread::get_id();
    ~Runtime() { lk_app_destroy(app); }

    void Live() const {
        if (thread != std::this_thread::get_id()) {
            throw std::runtime_error("Runtime must stay on its UI thread");
        }

        if (!app) throw std::runtime_error("Runtime is closed");
    }

    void Status(int32_t status) const {
        if (status < 0) throw std::runtime_error(Take(lk_app_error(app)));
    }

    // Drains cancellations, then requests, before rendering, so a page
    // dispatches a whole UI turn's plugin work with the tree it produced.
    napi_value Render(napi_env env) {
        napi_value cancelled, requests;
        Check(napi_create_array(env, &cancelled));
        Check(napi_create_array(env, &requests));
        bool pending = false;
        uint32_t count = 0;

        for (; count < kMaxBatch; ++count) {
            const int64_t id = lk_app_next_cancelled(app);

            if (id < 0) throw std::runtime_error("Cannot read native plugin cancellations");

            if (id == 0) break;
            Check(napi_set_element(env, cancelled, count, Unsigned(env, static_cast<double>(id))));
        }

        pending = count == kMaxBatch;

        for (count = 0; count < kMaxBatch; ++count) {
            lk_request request{};
            const int32_t status = lk_app_next_request(app, &request);
            Status(status);

            if (status == 0) break;

            if (status == 2) {
                pending = true;
                break;
            }

            if (!plugins.count(request.plugin) ||
                static_cast<double>(request.id) > kMaxSafeInteger) {
                // Selection rejects worker plugins. Fail closed if a mismatched
                // installed guest nevertheless requests native work.
                lk_buffer_release(request.payload);
                lk_bridge_cancel(lk_app_plugins(app), request.id);
                throw std::runtime_error("HarmonyOS host cannot dispatch this native plugin");
            }

            napi_value payload = Copy(env, request.payload), item;
            Check(napi_create_object(env, &item));
            Check(napi_set_named_property(env, item, "payload", payload));
            Check(napi_set_named_property(
                env, item, "id", Unsigned(env, static_cast<double>(request.id))));
            Check(napi_set_named_property(env, item, "plugin", Unsigned(env, request.plugin)));
            Check(napi_set_named_property(env, item, "method", Unsigned(env, request.method)));
            Check(napi_set_element(env, requests, count, item));
        }

        pending = pending || count == kMaxBatch;

        const std::string json = std::string("{\"pending\":") + (pending ? "true" : "false") +
            ",\"render\":" + Take(lk_app_render(app)) + "}";

        napi_value text, global, jsonObject, parse, result;
        Check(napi_create_string_utf8(env, json.data(), json.size(), &text));
        Check(napi_get_global(env, &global));
        Check(napi_get_named_property(env, global, "JSON", &jsonObject));
        Check(napi_get_named_property(env, jsonObject, "parse", &parse));
        Check(napi_call_function(env, jsonObject, parse, 1, &text, &result));
        Check(napi_set_named_property(env, result, "requests", requests));
        Check(napi_set_named_property(env, result, "cancelled", cancelled));

        return result;
    }
};

struct RiveContext {
    napi_env env;
    std::vector<size_t> visible;
    napi_value frames;
    uint32_t count = 0;
    std::exception_ptr failure;
};

int32_t RiveVisible(void* opaque, size_t node) {
    auto& context = *static_cast<RiveContext*>(opaque);

    return std::find(context.visible.begin(), context.visible.end(), node) != context.visible.end() ? 1 : 0;
}

int32_t RivePresent(void* opaque, size_t node, uint32_t width, uint32_t height,
                    const uint8_t* pixels, size_t length) {
    auto& context = *static_cast<RiveContext*>(opaque);
    try {
        if (width == 0 || height == 0 || width > 4096 || height > 4096 ||
            uint64_t(width) * height > 4194304 || uint64_t(width) * height * 4 != length) {
            throw std::runtime_error("Invalid native Rive frame");
        }
        // Canvas records a shared PixelMap for later rendering. Each uploaded
        // frame is immutable; never overwrite pixels still retained by a draw.
        auto image = std::make_unique<RiveImage>(width, height);
        // WritePixels reads the source synchronously. The JS wrapper shares
        // PixelMap ownership, never a pointer into the Rust raster allocation.
        ImageStatus(OH_PixelmapNative_WritePixels(image->pixels, const_cast<uint8_t*>(pixels), length));
        napi_value bitmap, result, id, w, h;
        ImageStatus(OH_PixelmapNative_ConvertPixelmapNativeToNapi(context.env, image->pixels, &bitmap));

        Check(napi_create_object(context.env, &result));
        Check(napi_create_double(context.env, static_cast<double>(node), &id));
        Check(napi_create_uint32(context.env, width, &w));
        Check(napi_create_uint32(context.env, height, &h));
        Check(napi_set_named_property(context.env, result, "id", id));
        Check(napi_set_named_property(context.env, result, "width", w));
        Check(napi_set_named_property(context.env, result, "height", h));
        Check(napi_set_named_property(context.env, result, "pixelMap", bitmap));
        Check(napi_set_element(context.env, context.frames, context.count++, result));

        return 1;
    } catch (...) {
        // Exceptions must never unwind across Rust's extern-C callback.
        context.failure = std::current_exception();

        return -1;
    }
}

void Finalize(napi_env, void *data, void *) { delete static_cast<Runtime *>(data); }

// All C++ errors become ArkTS exceptions; no exception crosses Node-API's C ABI.
template<typename Function>
napi_value Guard(napi_env env, Function body) {
    try {
        return body();
    } catch (const std::exception &error) {
        napi_throw_error(env, nullptr, error.what());

        return nullptr;
    }
}

Runtime *Unwrap(napi_env env, napi_callback_info info, size_t expected, napi_value *args) {
    size_t argc = expected;
    napi_value self;
    Check(napi_get_cb_info(env, info, &argc, args, &self, nullptr));

    if (argc != expected) throw std::runtime_error("Invalid runtime argument count");
    void *data = nullptr;
    Check(napi_unwrap(env, self, &data));

    if (!data) throw std::runtime_error("Invalid runtime receiver");
    auto *runtime = static_cast<Runtime *>(data);

    runtime->Live();

    return runtime;
}

napi_value Create(napi_env env, napi_callback_info info) {
    return Guard(env, [&]() {
        napi_value target;
        Check(napi_get_new_target(env, info, &target));
        if (!target) throw std::runtime_error("Use new Runtime to create a guest");
        napi_value args[3], self;
        size_t argc = 3;
        Check(napi_get_cb_info(env, info, &argc, args, &self, nullptr));
        if (argc != 3) {
            throw std::runtime_error("Runtime needs Wasm, data directory, and cache directory");
        }

        void *wasm = nullptr;
        size_t len = 0;
        Check(napi_get_arraybuffer_info(env, args[0], &wasm, &len));
        if (len == 0 || len > 32 * 1024 * 1024) throw std::runtime_error("Invalid Wasm size");
        const auto data = String(env, args[1]);
        const auto cache = String(env, args[2]);
        const lk_app_context context{
            reinterpret_cast<const uint8_t *>(data.data()), data.size(),
            reinterpret_cast<const uint8_t *>(cache.data()), cache.size()
        };
        auto runtime = std::make_unique<Runtime>();
        lk_buffer error{};
        runtime->app = leptoskit_app_create_with_context(
            static_cast<const uint8_t *>(wasm), len, &context, &error);
        if (!runtime->app) throw std::runtime_error(Take(error));
        Check(napi_wrap(env, self, runtime.get(), Finalize, nullptr, nullptr));
        runtime.release();
        return self;
    });
}

napi_value Start(napi_env env, napi_callback_info info) {
    return Guard(env, [&]() {
        napi_value args[2];
        auto *runtime = Unwrap(env, info, 2, args);
        runtime->Status(lk_app_start(runtime->app, Number(env, args[0]), Number(env, args[1])));
        return runtime->Render(env);
    });
}

napi_value Resize(napi_env env, napi_callback_info info) {
    return Guard(env, [&]() {
        napi_value args[3];
        auto *runtime = Unwrap(env, info, 3, args);
        bool dark = false;
        Check(napi_get_value_bool(env, args[2], &dark));
        runtime->Status(lk_app_resize(
            runtime->app, Number(env, args[0]), Number(env, args[1]), dark));
        return runtime->Render(env);
    });
}

napi_value Event(napi_env env, napi_callback_info info) {
    return Guard(env, [&]() {
        napi_value args[3];
        auto *runtime = Unwrap(env, info, 3, args);
        const double node = Number(env, args[0]);
        const double kind = Number(env, args[1]);
        if (node < 0 || node > 9007199254740991.0 ||
            std::floor(node) != node || kind < 0 || kind > 4 || std::floor(kind) != kind) {
            throw std::runtime_error("Invalid native event");
        }
        const auto text = String(env, args[2]);
        runtime->Status(lk_app_event(
            runtime->app, static_cast<size_t>(node), static_cast<uint32_t>(kind),
            reinterpret_cast<const uint8_t *>(text.data()), text.size()));
        return runtime->Render(env);
    });
}

napi_value Register(napi_env env, napi_callback_info info) {
    return Guard(env, [&]() {
        napi_value args[2];
        auto *runtime = Unwrap(env, info, 2, args);
        const auto name = String(env, args[0]);
        bool array = false;
        Check(napi_is_array(env, args[1], &array));
        if (!array) throw std::runtime_error("Plugin methods must be an array");
        uint32_t length = 0;
        Check(napi_get_array_length(env, args[1], &length));
        if (length == 0 || length > 256) throw std::runtime_error("Invalid plugin method count");

        std::vector<uint32_t> methods(length);
        for (uint32_t index = 0; index < length; ++index) {
            napi_value method;
            Check(napi_get_element(env, args[1], index, &method));
            methods[index] = static_cast<uint32_t>(Integer(env, method, 0, 4294967295.0));
        }

        lk_bridge *bridge = lk_app_plugins(runtime->app);
        const int64_t id = lk_bridge_register(
            bridge, reinterpret_cast<const uint8_t *>(name.data()), name.size(),
            methods.data(), methods.size());
        if (id < 0 || id > 4294967295LL) {
            throw std::runtime_error("Cannot register native plugin " + name);
        }

        for (const uint32_t method : methods) {
            if (lk_bridge_grant(bridge, static_cast<uint32_t>(id), method, true) != 0) {
                throw std::runtime_error("Cannot grant native plugin " + name);
            }
        }

        runtime->plugins.insert(static_cast<uint32_t>(id));
        return Unsigned(env, static_cast<double>(id));
    });
}

napi_value Complete(napi_env env, napi_callback_info info) {
    return Guard(env, [&]() {
        napi_value args[3];
        auto *runtime = Unwrap(env, info, 3, args);
        const auto id = static_cast<uint64_t>(Integer(env, args[0], 1, kMaxSafeInteger));
        const auto code = static_cast<int32_t>(Integer(env, args[1], 0, 9));
        void *data = nullptr;
        size_t len = 0;
        Check(napi_get_arraybuffer_info(env, args[2], &data, &len));

        const int32_t status = lk_bridge_complete(
            lk_app_plugins(runtime->app), id, code,
            code == 0 ? static_cast<const uint8_t *>(data) : nullptr, code == 0 ? len : 0);
        if (status < 0 && status != kInvalidCall) {
            throw std::runtime_error(
                "Native plugin completion failed with status " + std::to_string(status));
        }

        runtime->Status(lk_app_wake(runtime->app));
        return runtime->Render(env);
    });
}

napi_value Pump(napi_env env, napi_callback_info info) {
    return Guard(env, [&]() { return Unwrap(env, info, 0, nullptr)->Render(env); });
}

napi_value Rive(napi_env env, napi_callback_info info) {
    return Guard(env, [&]() {
        napi_value args[5];
        auto* runtime = Unwrap(env, info, 5, args);
        const double elapsed = Number(env, args[0]);
        const double scale = Number(env, args[1]);
        bool active = false, reduced = false;
        Check(napi_get_value_bool(env, args[2], &active));
        Check(napi_get_value_bool(env, args[3], &reduced));
        uint32_t count = 0;
        Check(napi_get_array_length(env, args[4], &count));
        if (count > 16) throw std::runtime_error("Too many visible Rive canvases");
        napi_value frames;
        Check(napi_create_array(env, &frames));
        RiveContext context{env, {}, frames, 0, nullptr};
        for (uint32_t index = 0; index < count; ++index) {
            napi_value value;
            Check(napi_get_element(env, args[4], index, &value));
            const double node = Number(env, value);
            if (node < 0 || node > 9007199254740991.0 ||
                node > static_cast<double>(std::numeric_limits<size_t>::max()) || std::floor(node) != node) {
                throw std::runtime_error("Invalid Rive node id");
            }
            context.visible.push_back(static_cast<size_t>(node));
        }

        const auto status = lk_app_rive_tick(runtime->app, elapsed, static_cast<float>(scale),
            (active ? 1u : 0u) | (reduced ? 2u : 0u), &context, RiveVisible, RivePresent);
        if (context.failure) std::rethrow_exception(context.failure);
        runtime->Status(status);
        napi_value result, pending;
        Check(napi_create_object(env, &result));
        Check(napi_get_boolean(env, status == 1, &pending));
        Check(napi_set_named_property(env, result, "pending", pending));
        Check(napi_set_named_property(env, result, "frames", frames));
        return result;
    });
}

napi_value Close(napi_env env, napi_callback_info info) {
    return Guard(env, [&]() {
        napi_value self;
        size_t argc = 0;
        Check(napi_get_cb_info(env, info, &argc, nullptr, &self, nullptr));
        void *data = nullptr;
        Check(napi_unwrap(env, self, &data));
        if (!data) throw std::runtime_error("Invalid runtime receiver");
        auto *runtime = static_cast<Runtime *>(data);
        if (runtime->thread != std::this_thread::get_id()) {
            throw std::runtime_error("Runtime must stay on its UI thread");
        }

        lk_app_destroy(runtime->app);
        runtime->app = nullptr;
        napi_value result;
        Check(napi_get_undefined(env, &result));
        return result;
    });
}

napi_value Init(napi_env env, napi_value exports) {
    return Guard(env, [&]() {
        const napi_property_descriptor methods[] = {
            {"start", nullptr, Start, nullptr, nullptr, nullptr, napi_default, nullptr},
            {"resize", nullptr, Resize, nullptr, nullptr, nullptr, napi_default, nullptr},
            {"event", nullptr, Event, nullptr, nullptr, nullptr, napi_default, nullptr},
            {"register", nullptr, Register, nullptr, nullptr, nullptr, napi_default, nullptr},
            {"complete", nullptr, Complete, nullptr, nullptr, nullptr, napi_default, nullptr},
            {"pump", nullptr, Pump, nullptr, nullptr, nullptr, napi_default, nullptr},
            {"rive", nullptr, Rive, nullptr, nullptr, nullptr, napi_default, nullptr},
            {"close", nullptr, Close, nullptr, nullptr, nullptr, napi_default, nullptr}
        };
        napi_value constructor;
        Check(napi_define_class(env, "Runtime", NAPI_AUTO_LENGTH, Create, nullptr,
            sizeof(methods) / sizeof(methods[0]), methods, &constructor));
        Check(napi_set_named_property(env, exports, "Runtime", constructor));
        return exports;
    });
}

napi_module module = {1, 0, nullptr, Init, "leptoskit", nullptr, {0}};
} // namespace

extern "C" __attribute__((constructor)) void RegisterLeptosKit() { napi_module_register(&module); }
