#import "ArenaLogModule.h"

@implementation ArenaLogModule

+ (NSString *)name {
  return @"ArenaLog";
}

+ (NSDictionary<NSString *, NSString *> *)methodLookup {
  return @{@"log" : NSStringFromSelector(@selector(log:))};
}

- (void)log:(NSString *)line {
  NSLog(@"%@", line);
}

@end
