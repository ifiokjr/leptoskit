import CLeptosKitBridge
import CoreGraphics
import Foundation
import LeptosKitBridge
import QuartzCore
#if os(macOS)
import AppKit
typealias NativeRiveView = NSView
#else
import UIKit
typealias NativeRiveView = UIView
#endif

/// A retained animation image is independent of file-backed Image properties.
@MainActor
final class NativeRiveImageView: NativeRiveImageBase {}

#if os(macOS)
typealias NativeRiveImageBase = NSImageView
#else
typealias NativeRiveImageBase = UIImageView
#endif

/// Native images only: callbacks cannot retain borrowed pixels or enter the guest.
@MainActor
protocol NativeRiveSink: AnyObject {
    func riveVisible(_ node: Int) -> Bool
    func presentRive(_ node: Int, width: Int, height: Int, pixels: UnsafeBufferPointer<UInt8>) throws
}

typealias NativeRiveVisible = @convention(c) (UnsafeMutableRawPointer?, Int) -> Int32
typealias NativeRivePresent = @convention(c) (
    UnsafeMutableRawPointer?, Int, UInt32, UInt32, UnsafePointer<UInt8>?, Int
) -> Int32

struct NativeRiveCallbacks {
    let context: UnsafeMutableRawPointer
    let visible: NativeRiveVisible
    let present: NativeRivePresent
}

/// One display link per host, invalidated whenever its scenes become idle.
@MainActor
final class NativeRiveAnimator: NSObject {
    weak var sink: (any NativeRiveSink)?
    private weak var view: NativeRiveView?
    var tick: ((Double, Float, UInt32, NativeRiveCallbacks) throws -> Bool)?
    var hasScenes: (() -> Bool)?
    var onFailure: ((Error) -> Void)?
    private var link: CADisplayLink?
    private var previous: CFTimeInterval = 0
    private var open = true
    private var presented = false
    private var ticking = false
    private var callbackError: Error?
    private var observers: [(NotificationCenter, NSObjectProtocol)] = []

    init(view: NativeRiveView) {
        self.view = view
        super.init()

        #if os(macOS)
        observe(.default, NSApplication.didBecomeActiveNotification)
        observe(.default, NSApplication.didResignActiveNotification)
        observe(.default, NSWindow.didMiniaturizeNotification)
        observe(.default, NSWindow.didDeminiaturizeNotification)
        observe(.default, NSWindow.didChangeBackingPropertiesNotification)

        observe(.default, NSView.boundsDidChangeNotification)
        observe(NSWorkspace.shared.notificationCenter, NSWorkspace.accessibilityDisplayOptionsDidChangeNotification)
        #else
        observe(.default, UIApplication.didBecomeActiveNotification)
        observe(.default, UIApplication.didEnterBackgroundNotification)
        observe(.default, UIApplication.willResignActiveNotification)
        observe(.default, UIAccessibility.reduceMotionStatusDidChangeNotification)
        observe(.default, UIScreen.modeDidChangeNotification)
        #endif
    }

    isolated deinit { close() }

    private func observe(_ center: NotificationCenter, _ name: Notification.Name) {
        let token = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.requestFrame() }
        }
        observers.append((center, token))
    }

    private var active: Bool {
        guard presented, let view, let window = view.window else { return false }

        #if os(macOS)

        return NSApplication.shared.isActive && window.isVisible
            && !window.isMiniaturized && !view.isHiddenOrHasHiddenAncestor
        #else

        return UIApplication.shared.applicationState == .active && !window.isHidden && !view.isHidden
        #endif
    }

    func setPresented(_ shown: Bool) {
        guard open else { return }
        presented = shown

        if shown { requestFrame() } else { stop() }
    }

    func requestFrame() {
        guard open else { return }

        guard active, let view else { stop(); return }
        // A synchronous native image upload can post layout notifications.
        // Never query the Rust app again while that display callback is live.
        guard link == nil && !ticking else { return }

        guard hasScenes?() == true else { return }

        #if os(macOS)
        let display = view.displayLink(target: self, selector: #selector(displayFrame(_:)))
        #else
        let display = CADisplayLink(target: self, selector: #selector(displayFrame(_:)))

        #endif
        display.preferredFrameRateRange = CAFrameRateRange(minimum: 30, maximum: 60, preferred: 60)
        previous = 0
        link = display
        display.add(to: .main, forMode: .common)
    }

    func stop() {
        link?.invalidate()
        link = nil
        previous = 0
    }

    func close() {
        guard open else { return }
        open = false
        stop()

        for (center, token) in observers { center.removeObserver(token) }
        observers.removeAll()
        sink = nil
        tick = nil
        hasScenes = nil

        onFailure = nil
    }

    @objc private func displayFrame(_ display: CADisplayLink) {
        guard open, !ticking, active, let view else { stop(); return }
        ticking = true
        defer { ticking = false }
        let elapsed = previous == 0 ? 0 : max(0, display.timestamp - previous)
        previous = display.timestamp

        #if os(macOS)
        let scale = Float(view.window?.backingScaleFactor ?? 1)
        let reduced = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion

        #else
        let scale = Float(view.window?.screen.scale ?? 1)
        let reduced = UIAccessibility.isReduceMotionEnabled
        #endif
        do {
            callbackError = nil
            let pending = try autoreleasepool {
                try tick?(elapsed, scale, reduced ? 3 : 1, NativeRiveCallbacks(
                    context: Unmanaged.passUnretained(self).toOpaque(), visible: Self.visible, present: Self.present
                )) ?? false
            }

            if !pending { stop() }
        } catch {
            stop()
            onFailure?(callbackError ?? error)
            callbackError = nil
        }
    }

    private nonisolated static let visible: NativeRiveVisible = { context, node in

        guard let context else { return -1 }
        let owner = Unmanaged<NativeRiveAnimator>.fromOpaque(context).takeUnretainedValue()

        return MainActor.assumeIsolated {
            return owner.sink?.riveVisible(node) == true ? 1 : 0
        }
    }

    private nonisolated static let present: NativeRivePresent = { context, node, width, height, bytes, length in

        guard let context, let bytes, width > 0, height > 0, width <= 4096, height <= 4096,
            length >= 0, UInt64(width) * UInt64(height) <= 4_194_304,

            UInt64(length) == UInt64(width) * UInt64(height) * 4 else { return -1 }
        let owner = Unmanaged<NativeRiveAnimator>.fromOpaque(context).takeUnretainedValue()
        // This C callback is synchronous on the display link's main actor.
        // assumeIsolated asserts that executor; the borrowed buffer never escapes.
        nonisolated(unsafe) let pixels = UnsafeBufferPointer(start: bytes, count: length)

        return MainActor.assumeIsolated {
            guard let sink = owner.sink else { return -1 }
            do {
                try sink.presentRive(node, width: Int(width), height: Int(height), pixels: pixels)

                return 1
            } catch { owner.callbackError = error; return -1 }
        }
    }
}

/// One native upload copy. Core Graphics retains its own immutable CFData.
func nativeRiveImage(width: Int, height: Int, pixels: UnsafeBufferPointer<UInt8>) throws -> CGImage {
    let data = Data(buffer: pixels)

    guard let provider = CGDataProvider(data: data as CFData),
        let space = CGColorSpace(name: CGColorSpace.sRGB),
        let image = CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32,
            bytesPerRow: width * 4, space: space,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue).union(.byteOrder32Big),
            provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent)
    else { throw PluginError.native }

    return image
}
