@testable import LeptosKitMacControllers
import AppKit
import Foundation
import Testing

/// Captures real AppKit drawing in a parent canvas, retaining glyphs outside the line box.
@MainActor
struct TextPaint {
    private static let componentBits = 8
    private static let margin: CGFloat = 8
    let rows: [[UInt8]]
    var ink: Int { rows.flatMap { $0 }.filter { $0 < UInt8.max }.count }
    var contentRows: [[UInt8]] { Array(rows.drop(while: { $0.allSatisfy { $0 == UInt8.max } })) }
    /// Which pixels carry ink, whatever their intensity.
    var inkMask: [[Bool]] { rows.map { $0.map { $0 < UInt8.max } } }
    var contentInkMask: [[Bool]] { contentRows.map { $0.map { $0 < UInt8.max } } }

    static func capture(_ field: NSView, name: String? = nil) throws -> Self {
        let frame = field.frame
        let parent = field.superview
        let canvas = FlippedView(frame: CGRect(
            x: 0, y: 0, width: frame.width + margin * 2, height: frame.height + margin * 2
        ))
        field.removeFromSuperview()
        field.frame.origin = CGPoint(x: margin, y: margin)
        canvas.addSubview(field)
        defer {
            field.removeFromSuperview()
            field.frame = frame
            parent?.addSubview(field)
        }

        return try captureView(canvas, name: name)
    }

    /// Every channel is retained; only unused white rows at the end are trimmed.
    static func captureView(_ view: NSView, name: String? = nil) throws -> Self {
        let width = Int(view.frame.width)
        let height = Int(view.frame.height)
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.white.cgColor
        let window = NSWindow(
            contentRect: view.bounds, styleMask: .borderless, backing: .buffered, defer: false
        )
        window.isReleasedWhenClosed = false
        // The light appearance, so the dynamic label color is dark on the white canvas.
        window.appearance = NSAppearance(named: .aqua)
        window.contentView = view
        defer { window.contentView = nil; window.close() }
        let bitmap = try #require(NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
            bitsPerSample: componentBits, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
        ))
        view.cacheDisplay(in: view.bounds, to: bitmap)
        try save(bitmap, name: name)
        let pixels = try #require(bitmap.bitmapData)
        var rows = (0..<height).map { row in
            Array(UnsafeBufferPointer(start: pixels + row * bitmap.bytesPerRow, count: width * 4))
        }

        while rows.last?.allSatisfy({ $0 == UInt8.max }) == true { rows.removeLast() }

        return Self(rows: rows)
    }

    private static func save(_ bitmap: NSBitmapImageRep, name: String?) throws {
        let directory = ProcessInfo.processInfo.environment["LEPTOSKIT_PAINT_OUTPUT"]

        guard let name, let directory else { return }

        let data = try #require(bitmap.representation(using: .png, properties: [:]))
        try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
    }
}
