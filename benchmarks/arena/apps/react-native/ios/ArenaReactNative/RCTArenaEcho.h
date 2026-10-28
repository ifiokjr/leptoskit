#import <ArenaEchoSpec/ArenaEchoSpec.h>
#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

/// The arena's echo module. Promise methods are the async tier; the `Sync`
/// methods are synchronous TurboModule methods, the sync tier.
@interface RCTArenaEcho : NSObject <NativeArenaEchoSpec>
@end

NS_ASSUME_NONNULL_END
