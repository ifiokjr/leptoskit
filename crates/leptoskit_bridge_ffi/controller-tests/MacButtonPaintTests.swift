@testable import LeptosKitMacControllers
import AppKit
import Foundation
import Testing

@Suite(.serialized)
@MainActor
struct MacButtonPaintTests {
    private static let journey = "Subscribe, transfer and unsubscribe"
    nonisolated private static let defaultFont: CGFloat = 14
    private static let nativeHeight: CGFloat = 24
    private static let naturalWidth: CGFloat = 258.5
    private static let referenceHeight: CGFloat = 160
    private static let referencePadding: CGFloat = 100
    private static let oldWidth: CGFloat = 239
    private static let oldHeight: CGFloat = 17
    private static let constrainedWidth: CGFloat = 64
    private static let customWidth: CGFloat = 431
    private static let customHeight: CGFloat = 33
    private static let customFont: CGFloat = 28
    nonisolated private static let oversizedFonts: [CGFloat] = [28, 42]

    /// A label-sized frame loses glyphs; the actual native frame preserves the full title.
    @Test func nativeButtonPaintsTheCompleteUnchangedJourney() throws {
        let button = try makeButton()
        button.frame = CGRect(x: 0, y: 0, width: Self.naturalWidth, height: Self.nativeHeight)
        let compact = try ButtonPaint.capture(button)
        button.frame.size.width += Self.referencePadding
        button.frame.size.height = Self.referenceHeight
        let reference = try ButtonPaint.capture(button)
        #expect(compact == reference)
        #expect(compact.coverage > 0)

        button.frame.size = CGSize(width: Self.oldWidth, height: Self.oldHeight)
        let old = try ButtonPaint.capture(button)
        #expect(old.coverage < reference.coverage)
    }

    /// Oversized rounded fonts retain native height and overflow, without a parent clipping promise.
    @Test(arguments: oversizedFonts)
    func largeFontsMatchTheNativeReference(fontSize: CGFloat) throws {
        let button = try makeButton(fontSize: fontSize)
        let native = try #require(button.cell).cellSize
        #expect(native.height == Self.nativeHeight)
        button.frame.size = native
        let compact = try ButtonPaint.capture(button)
        button.frame.size.width += Self.referencePadding
        button.frame.size.height = Self.referenceHeight
        let reference = try ButtonPaint.capture(button)

        #expect(compact == reference)
    }

    /// Custom background, border and corners replace chrome, and removing each restores it.
    @Test(arguments: ["background", "borderWidth", "cornerRadius"])
    func chromeAndTitleUpdatesRetainTheButton(property: String) throws {
        let button = try makeButton()
        let identity = ObjectIdentifier(button)
        let frame = CGRect(x: 0, y: 0, width: Self.naturalWidth, height: Self.nativeHeight)
        button.frame = frame
        try apply(["text": "Updated gypq ÅÉç", property: 0], to: button)
        #expect(!button.isBordered)
        #expect(button.title == "Updated gypq ÅÉç")
        try apply(["text": Self.journey], to: button)

        #expect(button.isBordered)
        #expect(button.title == Self.journey)
        #expect(ObjectIdentifier(button) == identity)
        #expect(button.frame == frame)
    }

    /// A borderless control paints the complete title within its content-sized allocation.
    @Test func customButtonUsesContentSizedPaint() throws {
        let button = try makeButton(fontSize: Self.customFont)
        try apply(["text": Self.journey, "fontSize": Self.customFont, "background": 0], to: button)
        colorTitle(button)
        #expect(!button.isBordered)
        button.frame.size = CGSize(width: Self.customWidth, height: Self.customHeight)
        let compact = try ButtonPaint.capture(button)
        button.frame.size.width += Self.referencePadding
        button.frame.size.height = Self.referenceHeight
        let reference = try ButtonPaint.capture(button)
        #expect(compact == reference)
        #expect(compact.coverage > 0)
    }

    /// The production renderer keeps an explicit narrow frame and the full accessible title.
    @Test func intentionallyNarrowFrameRemainsConstrained() throws {
        let button = try makeButton()
        button.frame = CGRect(
            x: 0, y: 0, width: Self.constrainedWidth, height: Self.nativeHeight
        )
        let narrow = try ButtonPaint.capture(button)
        button.frame.size.width = Self.naturalWidth
        let natural = try ButtonPaint.capture(button)
        #expect(narrow.coverage < natural.coverage)
        #expect(button.title == Self.journey)
    }

    private func makeButton(fontSize: CGFloat = Self.defaultFont) throws -> NSButton {
        let value: [String: Any] = [
            "version": 1, "root": 1, "mutations": [
                ["Create": ["id": 1, "kind": "Button", "props": [
                    "text": Self.journey, "font_size": fontSize
                ]]]
            ]
        ]
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let transaction = try decoder.decode(
            Transaction.self, from: JSONSerialization.data(withJSONObject: value)
        )
        let view = NSView()
        try MacWidgetRenderer(view: view).render(transaction)
        let button = try #require(view.subviews.first as? NSButton)
        colorTitle(button)

        return button
    }

    private func colorTitle(_ button: NSButton) {
        let title = NSMutableAttributedString(attributedString: button.attributedTitle)

        title.addAttribute(
            .foregroundColor, value: NSColor.red, range: NSRange(location: 0, length: title.length)
        )
        button.attributedTitle = title
    }

    private func apply(_ value: [String: Any], to button: NSButton) throws {
        let props = try JSONDecoder().decode(
            WidgetProps.self, from: JSONSerialization.data(withJSONObject: value)
        )

        try MacWidgetStyle.apply(props, to: button)
    }
}
