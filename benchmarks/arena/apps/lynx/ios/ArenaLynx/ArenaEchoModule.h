#import <Foundation/Foundation.h>
#import <Lynx/LynxModule.h>

NS_ASSUME_NONNULL_BEGIN

/// The arena's echo module. Lynx calls its methods on the background (JS)
/// thread. The callback methods are the async tier: Lynx queues the callback
/// back to JavaScript as a separate task. The returning methods are the sync
/// tier: their result is the JavaScript call's return value.
@interface ArenaEchoModule : NSObject <LynxModule>
@end

NS_ASSUME_NONNULL_END
