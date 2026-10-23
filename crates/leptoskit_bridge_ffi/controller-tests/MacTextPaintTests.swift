@testable import LeptosKitMacControllers
import AppKit
import Foundation
import Testing

@Suite(.serialized)
@MainActor
struct MacTextPaintTests {
    nonisolated private static let defaultFont: CGFloat = 14
    nonisolated private static let scaledFont: CGFloat = 28
    private static let controlHeight: CGFloat = 320
    private static let journeyWidth: CGFloat = 712
    private static let wrapWidth: CGFloat = 64
    private static let wideWidth: CGFloat = 256
    private static let sceneInset: CGFloat = 8
    private static let sceneGap: CGFloat = 4
    private static let markerHeight: CGFloat = 8
    private static let journey = """
    Solana client journey
    ready
    denied
    balance: 500000000000000000 lamports, slot 277
    """

    /// The native measured four-line frame paints every pixel that the tall control paints.
    @Test func compactJourneyPaintsEveryLine() throws {
        let field = try label(Self.journey, width: Self.journeyWidth)
        let compact = try TextPaint.capture(field, name: "journey-compact-native")
        let nativeHeight = field.frame.height
        field.frame.size.height = Self.controlHeight
        let control = try TextPaint.capture(field, name: "journey-control-native")
        print("TEXT journey height=\(nativeHeight) compact=\(compact.ink) control=\(control.ink)")

        let equal = compact.rows == control.rows
        #expect(equal)
        #expect(compact.ink > 0)
    }

    /// Empty logical lines, word wrapping, descenders and accents preserve all pixels.
    @Test(arguments: ["", "ready\n", "ready\n\ndenied\n", "alpha beta gamma delta", "gypq ÅÉç\nfaçade naïve"])
    func logicalLinesAndWrapping(text: String) throws {
        let width = text.contains("Å") ? Self.wideWidth : Self.wrapWidth
        let field = try label(text, width: width)
        let compact = try TextPaint.capture(field)
        field.frame.size.height = Self.controlHeight
        let control = try TextPaint.capture(field)
        let equal = compact.rows == control.rows
        #expect(equal)
    }

    /// Font, alignment, color and text updates retain the native field and caller frame.
    @Test func fontAndPropertyUpdates() throws {
        let field = try label("ready\ndenied", width: Self.wideWidth, fontSize: Self.scaledFont)
        let compact = try TextPaint.capture(field)
        let originalFrame = field.frame
        field.frame.size.height = Self.controlHeight
        let originalEqual = compact.rows == (try TextPaint.capture(field)).rows
        #expect(originalEqual)
        field.frame = originalFrame
        let props = try JSONDecoder().decode(WidgetProps.self, from: Data(#"""
        {"text":"Updated\njourney", "fontSize":28, "fontWeight":"bold",
         "textAlign":"center", "color":255, "accessibilityLabel":"Updated journey"}
        """#.utf8))

        try MacWidgetStyle.apply(props, to: field)
        let updated = try TextPaint.capture(field)
        #expect(field.frame == originalFrame)
        field.frame.size.height = Self.controlHeight
        let updatedEqual = updated.rows == (try TextPaint.capture(field)).rows
        #expect(updatedEqual)
        #expect(updated.ink > 0)
        #expect(field.stringValue == "Updated\njourney")

        #expect(field.alignment == .center)
        #expect(field.font?.pointSize == Self.scaledFont)
        #expect(field.accessibilityIdentifier() == "Updated journey")
    }

    /// Parent-canvas capture preserves the complete natural AppKit glyph raster.
    @Test(arguments: ["gypq", "ÅÉç", "façade naïve"], [defaultFont, scaledFont])
    func naturalGlyphsStayWhole(text: String, fontSize: CGFloat) throws {
        let field = try label(text, width: Self.wideWidth, fontSize: fontSize)
        let compact = try TextPaint.capture(field)
        let natural = NSTextField(wrappingLabelWithString: text)
        natural.font = field.font
        natural.textColor = .black
        natural.wantsLayer = true
        natural.frame = CGRect(x: 0, y: 0, width: Self.wideWidth, height: Self.controlHeight)
        let control = try TextPaint.capture(natural)

        let equal = compact.contentRows == control.contentRows
        #expect(equal)
        #expect(compact.ink > 0)
    }

    /// Explicit parent clipping contains ink while an adjacent view retains every pixel.
    @Test(arguments: [false, true])
    func parentClippingAndAdjacentContent(clip: Bool) throws {
        let field = try label("ÅÉç", width: Self.wideWidth)
        let parent = FlippedView(frame: CGRect(
            x: 0, y: Self.sceneInset, width: field.frame.width, height: field.frame.height
        ))
        let props = try JSONDecoder().decode(
            WidgetProps.self, from: Data("{\"clip\":\(clip)}".utf8)
        )
        try MacWidgetStyle.apply(props, to: parent)
        parent.addSubview(field)

        let markerTop = parent.frame.maxY + Self.sceneGap
        let canvas = FlippedView(frame: CGRect(
            x: 0, y: 0, width: Self.wideWidth, height: markerTop + Self.markerHeight + Self.sceneInset
        ))
        canvas.addSubview(parent)

        let marker = NSView(frame: CGRect(
            x: 0, y: markerTop, width: Self.wideWidth, height: Self.markerHeight
        ))
        marker.wantsLayer = true
        marker.layer?.backgroundColor = NSColor.blue.cgColor
        canvas.addSubview(marker)

        let image = try TextPaint.captureView(canvas, name: "parent-clip-\(clip)")
        let spillRow = image.rows[Int(parent.frame.maxY)]
        let hasSpill = spillRow.contains { $0 < UInt8.max }
        #expect(hasSpill == !clip)
        let markerRows = image.rows[Int(markerTop)..<Int(markerTop + Self.markerHeight)]
        #expect(markerRows.allSatisfy { row in
            stride(from: 0, to: row.count, by: 4).allSatisfy { index in
                row[index] == 0 && row[index + 1] == 0 && row[index + 2] == UInt8.max
            }
        })
    }

    private func label(
        _ text: String, width: CGFloat, fontSize: CGFloat = Self.defaultFont
    ) throws -> NSTextField {
        let value: [String: Any] = [
            "version": 1, "root": 1, "mutations": [
                ["Create": ["id": 1, "kind": "Text", "props": [
                    "text": text, "font_size": fontSize, "color": 255
                ]]],
                ["UpdateLayout": ["id": 1, "x": 0, "y": 0, "width": width, "height": 0]]
            ]
        ]
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let transaction = try decoder.decode(
            Transaction.self, from: JSONSerialization.data(withJSONObject: value)
        )
        let view = NSView()
        try MacWidgetRenderer(view: view).render(transaction)
        let field = try #require(view.subviews.first as? NSTextField)
        let cell = try #require(field.cell)
        field.frame.size.height = cell.cellSize(forBounds: CGRect(
            x: 0, y: 0, width: width, height: Self.controlHeight
        )).height

        return field
    }
}
