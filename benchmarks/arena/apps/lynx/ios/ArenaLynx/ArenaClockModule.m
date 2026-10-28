#import "ArenaClockModule.h"

#import <mach/mach_time.h>

@implementation ArenaClockModule

+ (NSString *)name {
  return @"ArenaClock";
}

+ (NSDictionary<NSString *, NSString *> *)methodLookup {
  return @{@"nowUs" : NSStringFromSelector(@selector(nowUs))};
}

- (double)nowUs {
  static mach_timebase_info_data_t timebase;
  static dispatch_once_t once;
  dispatch_once(&once, ^{
    mach_timebase_info(&timebase);
  });
  return (double)mach_absolute_time() * timebase.numer / timebase.denom / 1000.0;
}

@end
