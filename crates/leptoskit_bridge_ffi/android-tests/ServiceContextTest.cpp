// Host-JVM control for the production JNI adapter. The C service boundary is a
// controlled callback-table recipient, not a real AppService or Android guest.
#include "../include/leptoskit_bridge.h"
#include <jni.h>
#include <thread>

namespace {
const uintptr_t CLOSING_HANDLE = 17;
lk_service_callbacks retained{};
}

extern "C" int32_t lk_app_activate_service(lk_app*, lk_service_callbacks callbacks) {
    retained = callbacks;

    return 0;
}

extern "C" JNIEXPORT void JNICALL
Java_dev_leptoskit_mobile_ServiceJniControl_closeAndWake(JNIEnv*, jclass) {
    auto callbacks = retained;
    retained = {};
    // Close must enqueue without polling/reentering the currently active ABI.
    callbacks.close(callbacks.context, reinterpret_cast<lk_closing_app*>(CLOSING_HANDLE));
    std::thread wake([callbacks] {
        callbacks.wake(callbacks.context);
        callbacks.release(callbacks.context);
    });
    wake.join();
}
