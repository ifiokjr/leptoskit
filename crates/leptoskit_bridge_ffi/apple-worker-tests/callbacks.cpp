#include "worker_test.h"
#include <thread>

extern "C" void test_service_wakes(lk_service_callbacks callbacks, int count) {
    std::thread worker([callbacks, count] {
        for (int index = 0; index < count; ++index) {
            callbacks.wake(callbacks.context);
        }
    });
    worker.join();
}

extern "C" void test_service_release(lk_service_callbacks callbacks) {
    std::thread worker([callbacks] { callbacks.release(callbacks.context); });
    worker.join();
}
