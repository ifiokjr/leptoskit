#import "ViewController.h"

#import <Lynx/LynxConfig.h>
#import <Lynx/LynxView.h>
#import <sys/sysctl.h>

#import "ArenaClockModule.h"
#import "ArenaEchoModule.h"
#import "ArenaLogModule.h"
#import "AssetTemplateProvider.h"

static NSString *const kBundleName = @"main.lynx.bundle";

/// The value after `--<name>` in the launch arguments, if any.
static NSString *LaunchArgument(NSString *name) {
  NSArray<NSString *> *arguments = NSProcessInfo.processInfo.arguments;
  NSString *flag = [@"--" stringByAppendingString:name];
  NSUInteger index = [arguments indexOfObject:flag];
  if (index == NSNotFound || index + 1 >= arguments.count) return nil;
  return arguments[index + 1];
}

/// The process start as a Unix timestamp in milliseconds, comparable with
/// Lynx's timing entries.
static NSNumber *ProcessStartUnixMs(void) {
  struct kinfo_proc info;
  size_t size = sizeof(info);
  int mib[4] = {CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()};
  if (sysctl(mib, 4, &info, &size, NULL, 0) != 0) return nil;
  struct timeval start = info.kp_proc.p_starttime;
  return @((double)start.tv_sec * 1000.0 + (double)start.tv_usec / 1000.0);
}

@implementation ViewController {
  LynxView *_lynxView;
}

- (void)viewDidLoad {
  [super viewDidLoad];
  self.view.backgroundColor = UIColor.whiteColor;

  LynxConfig *config = [[LynxConfig alloc] initWithProvider:[[AssetTemplateProvider alloc] init]];
  [config registerModule:ArenaClockModule.class];
  [config registerModule:ArenaEchoModule.class];
  [config registerModule:ArenaLogModule.class];

  CGSize size = self.view.bounds.size;
  _lynxView = [[LynxView alloc] initWithBuilderBlock:^(LynxViewBuilder *builder) {
    builder.config = config;
    builder.screenSize = size;
    builder.fontScale = 1.0;
  }];
  _lynxView.layoutWidthMode = LynxViewSizeModeExact;
  _lynxView.layoutHeightMode = LynxViewSizeModeExact;
  [_lynxView setGlobalPropsWithDictionary:[self globalProps]];
  [self.view addSubview:_lynxView];
  [self layoutLynxView];
  [_lynxView loadTemplateFromURL:kBundleName initData:nil];
}

- (void)viewDidLayoutSubviews {
  [super viewDidLayoutSubviews];
  [self layoutLynxView];
}

- (void)layoutLynxView {
  CGRect safe = self.view.safeAreaLayoutGuide.layoutFrame;
  if (CGRectEqualToRect(_lynxView.frame, safe)) return;
  _lynxView.frame = safe;
  _lynxView.preferredLayoutWidth = safe.size.width;
  _lynxView.preferredLayoutHeight = safe.size.height;
  [_lynxView triggerLayout];
}

- (NSDictionary<NSString *, id> *)globalProps {
  NSMutableDictionary<NSString *, id> *props = [NSMutableDictionary dictionary];
  props[@"platform"] = @"ios";
  props[@"frameworkVersion"] = [NSBundle.mainBundle objectForInfoDictionaryKey:@"ArenaLynxVersion"];
  props[@"refreshRate"] = @(UIScreen.mainScreen.maximumFramesPerSecond);
  NSNumber *processStart = ProcessStartUnixMs();
  if (processStart != nil) props[@"processStartUnixMs"] = processStart;
  NSString *scenario = LaunchArgument(@"scenario");
  if (scenario != nil) props[@"scenario"] = scenario;
  NSString *lane = LaunchArgument(@"lane");
  if (lane != nil) props[@"lane"] = lane;
  return props;
}

@end
