#import <Foundation/Foundation.h>
#import <Lynx/LynxTemplateProvider.h>

NS_ASSUME_NONNULL_BEGIN

/// Loads bundles from the app's resources: the arena ships its bundle in the
/// app, with no dev server.
@interface AssetTemplateProvider : NSObject <LynxTemplateProvider>
@end

NS_ASSUME_NONNULL_END
