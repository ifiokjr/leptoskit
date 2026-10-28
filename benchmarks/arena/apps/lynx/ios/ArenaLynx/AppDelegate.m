#import "AppDelegate.h"

#import <Lynx/LynxEnv.h>

#import "ViewController.h"

@implementation AppDelegate

- (BOOL)application:(UIApplication *)application
    didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
  // Initialise the engine once, before any LynxView exists.
  [LynxEnv sharedInstance];
  self.window = [[UIWindow alloc] initWithFrame:UIScreen.mainScreen.bounds];
  self.window.rootViewController = [[ViewController alloc] init];
  [self.window makeKeyAndVisible];
  return YES;
}

@end
