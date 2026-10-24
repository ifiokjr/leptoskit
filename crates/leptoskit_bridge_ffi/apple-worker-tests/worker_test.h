#include "../include/leptoskit_bridge.h"

#ifdef __cplusplus
extern "C" {
#endif
lk_rust_worker *test_worker_create(void);
int test_worker_started(void);
int test_worker_released(void);
void test_worker_unblock(void);

void test_service_wakes(lk_service_callbacks callbacks, int count);
void test_service_release(lk_service_callbacks callbacks);
#ifdef __cplusplus
}

#endif
