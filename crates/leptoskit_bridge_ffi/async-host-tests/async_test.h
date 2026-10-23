#include "../include/leptoskit_bridge.h"

#ifdef __cplusplus
extern "C" {
#endif
lk_async_job *test_async_create(int mode);
int test_async_started(void);
int test_async_cancelled(void);
int test_async_live_jobs(void);
int test_async_live_replies(void);

int test_async_live_tokens(void);
void test_async_finish(void);
#ifdef __cplusplus
}
#endif
