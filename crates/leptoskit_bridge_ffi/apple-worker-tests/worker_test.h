#include "../include/leptoskit_bridge.h"

#ifdef __cplusplus
extern "C" {
#endif
lk_rust_worker *test_worker_create(void);
int test_worker_started(void);
int test_worker_released(void);
void test_worker_unblock(void);
#ifdef __cplusplus
}
#endif
