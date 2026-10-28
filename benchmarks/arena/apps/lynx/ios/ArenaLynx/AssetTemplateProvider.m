#import "AssetTemplateProvider.h"

@implementation AssetTemplateProvider

- (void)loadTemplateWithUrl:(NSString *)url onComplete:(LynxTemplateLoadBlock)callback {
  NSString *path = [NSBundle.mainBundle pathForResource:url ofType:nil];
  if (path == nil) {
    callback(nil, [NSError errorWithDomain:@"dev.leptoskit.arena.lynx"
                                      code:404
                                  userInfo:@{NSLocalizedDescriptionKey : [@"no bundled resource " stringByAppendingString:url]}]);
    return;
  }
  NSError *error = nil;
  NSData *data = [NSData dataWithContentsOfFile:path options:0 error:&error];
  callback(data, error);
}

@end
