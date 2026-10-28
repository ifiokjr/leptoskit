#import "RCTArenaEcho.h"

@implementation RCTArenaEcho

RCT_EXPORT_MODULE(ArenaEcho)

- (std::shared_ptr<facebook::react::TurboModule>)getTurboModule:
    (const facebook::react::ObjCTurboModule::InitParams &)params
{
  return std::make_shared<facebook::react::NativeArenaEchoSpecJSI>(params);
}

- (void)echo:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  resolve(nil);
}

- (void)addOne:(double)value resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  resolve(@(value + 1));
}

- (void)echoString:(NSString *)value resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  resolve(value);
}

/// Decodes to bytes and encodes again, so the bytes really cross into native code.
- (void)echoBase64:(NSString *)value resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  NSData *bytes = [[NSData alloc] initWithBase64EncodedString:value options:0];
  if (bytes == nil) {
    reject(@"E_BASE64", @"echoBase64 received invalid base64", nil);
    return;
  }
  resolve([bytes base64EncodedStringWithOptions:0]);
}

- (void)echoObject:(NSDictionary *)value resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  resolve(value);
}

- (NSNumber *_Nullable)echoSync
{
  return nil;
}

- (NSNumber *)addOneSync:(double)value
{
  return @(value + 1);
}

- (NSString *)echoStringSync:(NSString *)value
{
  return value;
}

/// The bridge already copied the JS bytes into `value`; returning it copies them back into a new ArrayBuffer.
- (NSMutableData *)echoBytesSync:(NSMutableData *)value
{
  return value;
}

- (NSDictionary *)echoObjectSync:(NSDictionary *)value
{
  return value;
}

- (void)log:(NSString *)line
{
  NSLog(@"%@", line);
}

@end
