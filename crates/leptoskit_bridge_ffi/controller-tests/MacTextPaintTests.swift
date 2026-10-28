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
    nonisolated private static let wrapWidth: CGFloat = 64
    nonisolated private static let wideWidth: CGFloat = 256
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

    /// Font, alignment, color and text updates retain the label and caller frame.
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
        #expect(field.text == "Updated\njourney")

        #expect(field.alignment == .center)
        #expect(field.font.pointSize == Self.scaledFont)
        #expect(field.accessibilityIdentifier() == "Updated journey")
    }

    /// Parent-canvas capture inks every pixel of the natural AppKit glyph raster.
    @Test(arguments: ["gypq", "ÅÉç", "façade naïve"], [defaultFont, scaledFont])
    func naturalGlyphsStayWhole(text: String, fontSize: CGFloat) throws {
        let field = try label(text, width: Self.wideWidth, fontSize: fontSize, color: nil)
        let compact = try TextPaint.capture(field)
        let natural = NSTextField(wrappingLabelWithString: text)
        natural.font = field.font
        natural.wantsLayer = true
        natural.frame = CGRect(x: 0, y: 0, width: Self.wideWidth, height: Self.controlHeight)
        let control = try TextPaint.capture(natural)

        let equal = compact.contentInkMask == control.contentInkMask
        #expect(equal)
        #expect(compact.ink > 0)
    }

    /// In the measured box, the drawn label inks exactly the pixels an `NSTextField`
    /// label does. Intensities differ only at glyph edges, which AppKit's own label
    /// darkens with private font dilation.
    @Test(arguments: [
        ("ready", "start", wideWidth), ("alpha beta gamma delta", "start", wrapWidth),
        ("Solana client journey\nready\n\ndenied", "end", wideWidth),
        ("gypq ÅÉç\nfaçade naïve", "center", wideWidth), ("1234", "center", wrapWidth), ("", "start", wrapWidth)
    ])
    func paintsLikeANativeLabel(text: String, align: String, width: CGFloat) throws {
        let field = try label(text, width: width, color: nil, align: align)
        let natural = NSTextField(wrappingLabelWithString: text)
        natural.font = field.font
        natural.alignment = field.alignment
        natural.wantsLayer = true
        natural.frame = field.frame
        let drawn = try TextPaint.capture(field)
        let native = try TextPaint.capture(natural)

        let equal = drawn.inkMask == native.inkMask
        #expect(equal)
        #expect(text.isEmpty || drawn.ink > 0)
    }

    /// The label reports itself to assistive technology as static text.
    @Test func accessibleAsStaticText() throws {
        let field = try label("ready", width: Self.wideWidth)
        #expect(field.isAccessibilityElement())
        #expect(field.accessibilityRole() == .staticText)
        #expect(field.accessibilityValue() as? String == "ready")
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
        _ text: String, width: CGFloat, fontSize: CGFloat = Self.defaultFont,
        color: UInt32? = 255, align: String = "start"
    ) throws -> TextLabel {
        var props: [String: Any] = ["text": text, "font_size": fontSize, "text_align": align]
        props["color"] = color
        let value: [String: Any] = [
            "version": 1, "root": 1, "mutations": [
                ["Create": ["id": 1, "kind": "Text", "props": props]],
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
        let field = try #require(view.subviews.first as? TextLabel)
        field.frame.size.height = measuredHeight(field, width: width)

        return field
    }

    /// The height layout measures: line-fragment bounds inside the label's insets,
    /// one line for an empty string, rounded up to whole points.
    private func measuredHeight(_ field: TextLabel, width: CGFloat) -> CGFloat {
        let inset: CGFloat = field.alignment == .center ? 8 : 4
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = field.alignment
        paragraph.lineBreakMode = .byWordWrapping
        let string = NSAttributedString(string: field.text.isEmpty ? " " : field.text, attributes: [
            .font: field.font, .paragraphStyle: paragraph
        ])

        return ceil(string.boundingRect(
            with: CGSize(width: width - inset, height: .greatestFiniteMagnitude),
            options: .usesLineFragmentOrigin
        ).height)
    }
}
