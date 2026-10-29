import Flutter
import UIKit

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    ArenaHost.register(messenger: engineBridge.applicationRegistrar.messenger())
  }
}

/// The Runner's side of the arena app: launch arguments, process uptime, report
/// logging, and the `ArenaEcho` module for the async tier of `echo`.
enum ArenaHost {
  static func register(messenger: FlutterBinaryMessenger) {
    let host = FlutterMethodChannel(name: "arena/host", binaryMessenger: messenger)
    host.setMethodCallHandler { call, result in
      switch call.method {
      case "arguments":
        result(Array(CommandLine.arguments.dropFirst()))
      case "uptimeMs":
        result(processUptimeMs())
      case "log":
        NSLog("%@", call.arguments as? String ?? "")
        result(nil)
      default:
        result(FlutterMethodNotImplemented)
      }
    }

    let echo = FlutterMethodChannel(name: "arena/echo", binaryMessenger: messenger)
    echo.setMethodCallHandler { call, result in
      switch call.method {
      case "echo":
        result(call.arguments)
      case "addOne":
        guard let value = call.arguments as? Int else {
          result(FlutterError(code: "bad_argument", message: "addOne takes an integer", details: nil))
          return
        }
        result(value + 1)
      default:
        result(FlutterMethodNotImplemented)
      }
    }
  }

  /// Milliseconds since this process started, from the kernel's start time.
  private static func processUptimeMs() -> Double? {
    var info = kinfo_proc()
    var size = MemoryLayout<kinfo_proc>.stride
    var name: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
    guard sysctl(&name, u_int(name.count), &info, &size, nil, 0) == 0 else { return nil }
    let start = info.kp_proc.p_starttime
    var now = timeval()
    gettimeofday(&now, nil)
    return Double(now.tv_sec - start.tv_sec) * 1000 + Double(now.tv_usec - start.tv_usec) / 1000
  }
}
