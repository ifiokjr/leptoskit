// Every runtime belongs to one ArkTS object on the UI thread. Native pointers
// never cross the JS boundary; napi_wrap supplies ownership and GC cleanup.
#include "napi/native_api.h"
#include "leptoskit_bridge.h"

#include <cmath>
#include <memory>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

namespace {
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

struct Runtime {
    lk_app *app = nullptr;
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

    napi_value Render(napi_env env) {
        lk_request request{};
        const int32_t status = lk_app_next_request(app, &request);
        Status(status);

        if (status == 1) {
            // Selection rejects worker/platform plugins. Fail closed if a
            // mismatched installed guest nevertheless requests native work.
            lk_buffer_release(request.payload);
            lk_bridge_cancel(lk_app_plugins(app), request.id);
            throw std::runtime_error("HarmonyOS host cannot dispatch this native plugin");
        }

        const std::string json = std::string("{\"pending\":") + (status == 2 ? "true" : "false") +
            ",\"render\":" + Take(lk_app_render(app)) + "}";

        napi_value text, global, jsonObject, parse, result;
        Check(napi_create_string_utf8(env, json.data(), json.size(), &text));
        Check(napi_get_global(env, &global));
        Check(napi_get_named_property(env, global, "JSON", &jsonObject));
        Check(napi_get_named_property(env, jsonObject, "parse", &parse));
        Check(napi_call_function(env, jsonObject, parse, 1, &text, &result));

        return result;
    }
};

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

napi_value Pump(napi_env env, napi_callback_info info) {
    return Guard(env, [&]() { return Unwrap(env, info, 0, nullptr)->Render(env); });
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
            {"pump", nullptr, Pump, nullptr, nullptr, nullptr, napi_default, nullptr},
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
