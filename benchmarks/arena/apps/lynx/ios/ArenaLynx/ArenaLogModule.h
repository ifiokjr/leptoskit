#import <Foundation/Foundation.h>
#import <Lynx/LynxModule.h>

NS_ASSUME_NONNULL_BEGIN

/// Writes report lines with `NSLog`, which `simctl launch --console-pty` and
/// `log stream` both read.
@interface ArenaLogModule : NSObject <LynxModule>
@end

NS_ASSUME_NONNULL_END
