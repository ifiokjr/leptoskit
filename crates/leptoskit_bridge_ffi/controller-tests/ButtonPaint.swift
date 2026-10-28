import AppKit
import Foundation
import Testing

/// Reads actual colored title pixels from a generous parent canvas, including native overflow.
@MainActor
enum ButtonPaint {
    private static let margin: CGFloat = 96
    private static let componentBits = 8

    static func capture(_ button: NSButton) throws -> ButtonInk {
        let width = Int(ceil(button.frame.width + margin * 2))
        let height = Int(ceil(button.frame.height + margin * 2))
        let bitmap = try #require(NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
            bitsPerSample: componentBits, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
        ))
        let context = try #require(NSGraphicsContext(bitmapImageRep: bitmap))
        let cell = try #require(button.cell)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = context

        NSColor.white.setFill()
        NSRect(x: 0, y: 0, width: CGFloat(width), height: CGFloat(height)).fill()
        context.cgContext.translateBy(x: margin, y: margin)
        cell.draw(withFrame: button.bounds, in: button)
        NSGraphicsContext.restoreGraphicsState()

        return try ink(bitmap)
    }

    private static func ink(_ bitmap: NSBitmapImageRep) throws -> ButtonInk {
        let bytes = try #require(bitmap.bitmapData)
        var total = ButtonInk()

        for rowIndex in 0..<bitmap.pixelsHigh {
            total.add(row: bytes + rowIndex * bitmap.bytesPerRow, width: bitmap.pixelsWide)
        }

        return total
    }

}

struct ButtonInk: Equatable {
    private static let colorThreshold = 20
    var pixels = 0
    var coverage = 0

    mutating func add(row: UnsafeMutablePointer<UInt8>, width: Int) {
        for columnIndex in 0..<width {
            let offset = columnIndex * 4
            let difference = Int(row[offset]) - max(Int(row[offset + 1]), Int(row[offset + 2]))

            if difference > Self.colorThreshold {
                pixels += 1
                coverage += difference
            }
        }
    }
}
