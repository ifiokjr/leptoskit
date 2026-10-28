#import <Foundation/Foundation.h>
#import <Lynx/LynxModule.h>

NS_ASSUME_NONNULL_BEGIN

/// A monotonic microsecond clock for the bundle. Lynx runs the iOS
/// background thread on JavaScriptCore, which has no `performance.now()`
/// and only millisecond `Date.now()`; the echo ladder needs microseconds.
@interface ArenaClockModule : NSObject <LynxModule>
@end

NS_ASSUME_NONNULL_END
