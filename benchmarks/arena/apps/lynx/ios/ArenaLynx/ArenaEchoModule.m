#import "ArenaEchoModule.h"

typedef void (^ArenaCallback)(id _Nullable result);

@implementation ArenaEchoModule

+ (NSString *)name {
  return @"ArenaEcho";
}

+ (NSDictionary<NSString *, NSString *> *)methodLookup {
  return @{
    @"echo" : NSStringFromSelector(@selector(echo:)),
    @"addOne" : NSStringFromSelector(@selector(addOne:callback:)),
    @"echoString" : NSStringFromSelector(@selector(echoString:callback:)),
    @"echoBytes" : NSStringFromSelector(@selector(echoBytes:callback:)),
    @"echoObject" : NSStringFromSelector(@selector(echoObject:callback:)),
    @"echoSync" : NSStringFromSelector(@selector(echoSync)),
    @"addOneSync" : NSStringFromSelector(@selector(addOneSync:)),
    @"echoStringSync" : NSStringFromSelector(@selector(echoStringSync:)),
    @"echoBytesSync" : NSStringFromSelector(@selector(echoBytesSync:)),
    @"echoObjectSync" : NSStringFromSelector(@selector(echoObjectSync:)),
  };
}

- (void)echo:(ArenaCallback)callback {
  callback(nil);
}

- (void)addOne:(double)value callback:(ArenaCallback)callback {
  callback(@(value + 1));
}

- (void)echoString:(NSString *)value callback:(ArenaCallback)callback {
  callback(value);
}

- (void)echoBytes:(NSData *)value callback:(ArenaCallback)callback {
  callback(value);
}

- (void)echoObject:(NSDictionary *)value callback:(ArenaCallback)callback {
  callback(value);
}

- (void)echoSync {
}

- (double)addOneSync:(double)value {
  return value + 1;
}

- (NSString *)echoStringSync:(NSString *)value {
  return value;
}

- (NSData *)echoBytesSync:(NSData *)value {
  return value;
}

- (NSDictionary *)echoObjectSync:(NSDictionary *)value {
  return value;
}

@end
