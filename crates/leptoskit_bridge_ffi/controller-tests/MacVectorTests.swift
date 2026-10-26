@testable import LeptosKitMacControllers
import AppKit
import Foundation
import Testing

@Suite(.serialized)
@MainActor
struct MacVectorTests {
    private static let ink: UInt32 = 0x1122_33FF
    private static let red: UInt32 = 0xFF00_00FF

    /// Each path paints its fill, then its stroke, in order; fixed strokes keep their layout-unit width.
    @Test func pathsPaintFillThenStrokeInOrderWithUnscaledStrokes() throws {
        let host = NSView()
        let renderer = MacWidgetRenderer(view: host)
        try renderer.render(decode(vector(
            layout: CGRect(x: 0, y: 0, width: 48, height: 24),
            drawing: """
            {"view_box":{"min_x":0,"min_y":0,"width":24,"height":12},"scaling":"stretch","paths":[
              {"data":"M 0 0 L 24 0 L 24 12 Z","fill":\(Self.red),
               "stroke":{"color":\(Self.ink),"width":1.5,"cap":"round","join":"bevel"}},
              {"data":"M 0 0 L 4 0 L 0 4 Z","fill":\(Self.ink)},
              {"data":"M 1 1 L 2 2","stroke":{"color":\(Self.ink),"width":2}}
            ]}
            """
        )))
        let shapes = try shapeLayers(host)
        #expect(shapes.count == 4)

        let fill = shapes[0]
        #expect(fill.fillColor == color(Self.red))
        #expect(fill.strokeColor == nil)
        #expect(fill.fillRule == .nonZero)
        #expect(fill.path?.boundingBoxOfPath == CGRect(x: 0, y: 0, width: 48, height: 24))

        let outline = shapes[1]
        #expect(outline.fillColor == nil)
        #expect(outline.strokeColor == color(Self.ink))
        #expect(outline.lineWidth == 1.5)
        #expect(outline.lineCap == .round)
        #expect(outline.lineJoin == .bevel)
        #expect(outline.miterLimit == 4)
        #expect(outline.path == fill.path)

        #expect(shapes[2].fillColor == color(Self.ink))
        #expect(shapes[2].path?.boundingBoxOfPath == CGRect(x: 0, y: 0, width: 8, height: 8))
        #expect(shapes[3].lineCap == .butt)
        #expect(shapes[3].lineJoin == .miter)
    }

    /// Fit scales uniformly and centres the view box, offsetting by its origin.
    @Test func fitCentresTheViewBoxAndLayoutChangesRepaint() throws {
        let host = NSView()
        let renderer = MacWidgetRenderer(view: host)
        try renderer.render(decode(vector(
            layout: CGRect(x: 0, y: 0, width: 40, height: 20),
            drawing: """
            {"view_box":{"min_x":-5,"min_y":-5,"width":10,"height":10},
             "paths":[{"data":"M -5 -5 L 5 -5 L 5 5 L -5 5 Z","fill":\(Self.ink)}]}
            """
        )))
        #expect(try shapeLayers(host).first?.path?.boundingBoxOfPath == CGRect(x: 10, y: 0, width: 20, height: 20))

        try renderer.render(decode("""
        {"version":1,"mutations":[{"UpdateLayout":{"id":1,"x":0,"y":0,"width":30,"height":60}}]}
        """))
        let shapes = try shapeLayers(host)
        #expect(shapes.count == 1)
        #expect(shapes.first?.path?.boundingBoxOfPath == CGRect(x: 0, y: 15, width: 30, height: 30))
    }

    /// A label makes the drawing an image element; without one it is decorative and removing the drawing clears it.
    @Test func accessibilityFollowsTheLabelAndPropsUpdatesRepaint() throws {
        let host = NSView()
        let renderer = MacWidgetRenderer(view: host)
        try renderer.render(decode(vector(
            layout: CGRect(x: 0, y: 0, width: 10, height: 10),
            drawing: #"{"view_box":{"min_x":0,"min_y":0,"width":1,"height":1},"paths":[]}"#,
            label: "Logo"
        )))
        let view = try #require(host.subviews.first as? MacVectorView)
        #expect(view.isAccessibilityElement())
        #expect(view.accessibilityRole() == .image)
        #expect(view.accessibilityLabel() == "Logo")

        try renderer.render(decode("""
        {"version":1,"mutations":[{"UpdateProps":{"id":1,"props":{}}}]}
        """))
        #expect(!view.isAccessibilityElement())
        #expect(view.drawing == nil)
        #expect(try shapeLayers(host).isEmpty)
    }

    /// Strokes paint top-down and overhang the frame unless the element clips.
    @Test func strokesPaintFromTheTopAndOverhangTheFrame() throws {
        let root = FlippedView(frame: CGRect(x: 0, y: 0, width: 40, height: 40))
        root.wantsLayer = true
        let renderer = MacWidgetRenderer(view: root)
        try renderer.render(decode(vector(
            layout: CGRect(x: 10, y: 10, width: 20, height: 20),
            drawing: """
            {"view_box":{"min_x":0,"min_y":0,"width":10,"height":10},"scaling":"stretch",
             "paths":[{"data":"M 0 0 L 10 0","stroke":{"color":\(Self.red),"width":4}}]}
            """
        )))
        let bitmap = try #require(root.bitmapImageRepForCachingDisplay(in: root.bounds))
        root.cacheDisplay(in: root.bounds, to: bitmap)

        #expect(isRed(pixel(bitmap, in: root, at: CGPoint(x: 20, y: 9))), "the stroke overhangs the frame's top edge")
        #expect(isRed(pixel(bitmap, in: root, at: CGPoint(x: 20, y: 11))))
        #expect(!isRed(pixel(bitmap, in: root, at: CGPoint(x: 20, y: 28))), "nothing paints at the frame's bottom")
    }

    /// Miter limits, dashes and fill rules reach the layers; a scaling stroke multiplies its width and dashes
    /// by the square root of the area scale, while a fixed one keeps its layout units.
    @Test func strokeStylesAndFillRulesReachTheLayers() throws {
        let host = NSView()
        let renderer = MacWidgetRenderer(view: host)
        try renderer.render(decode(vector(
            layout: CGRect(x: 0, y: 0, width: 40, height: 10),
            drawing: """
            {"view_box":{"min_x":0,"min_y":0,"width":10,"height":10},"scaling":"stretch","paths":[
              {"data":"M 0 0 L 10 0 L 10 10 Z","fill":\(Self.red),"fill_rule":"evenodd",
               "stroke":{"color":\(Self.ink),"width":1.5,"miter_limit":10,"dashes":[2,1],"dash_offset":1,
                         "scaling":"drawing"}},
              {"data":"M 0 0 L 10 10","stroke":{"color":\(Self.ink),"width":2,"dashes":[3],"dash_offset":0.5}},
              {"data":"M 0 0 L 10 10","stroke":{"color":\(Self.ink),"width":2,"dashes":[0,0],"dash_offset":3}}
            ]}
            """
        )))
        let shapes = try shapeLayers(host)
        #expect(shapes.count == 4)
        #expect(shapes[0].fillRule == .evenOdd)

        // Stretching 10 by 10 onto 40 by 10 scales the area by four, so a scaling stroke doubles.
        let scaled = shapes[1]
        #expect(scaled.lineWidth == 3)
        #expect(scaled.miterLimit == 10)
        #expect(scaled.lineDashPattern == [4, 2])
        #expect(scaled.lineDashPhase == 2)

        let fixed = shapes[2]
        #expect(fixed.lineWidth == 2)
        #expect(fixed.miterLimit == 4)
        #expect(fixed.lineDashPattern == [3, 3], "an odd dash list repeats to make it even")
        #expect(fixed.lineDashPhase == 0.5)

        #expect(shapes[3].lineDashPattern == nil, "an all-zero dash list is solid")
        #expect(shapes[3].lineDashPhase == 0)
    }

    /// Fill covers the frame with a uniform scale, and alignment places the overflow; fit aligns the spare room.
    @Test(arguments: [
        (#""scaling":"fill","align":{"x":"center","y":"start"}"#, CGRect(x: 0, y: 0, width: 40, height: 40)),
        (#""scaling":"fill""#, CGRect(x: 0, y: -10, width: 40, height: 40)),
        (#""scaling":"fill","align":{"x":"center","y":"end"}"#, CGRect(x: 0, y: -20, width: 40, height: 40)),
        (#""scaling":"fit","align":{"x":"end","y":"start"}"#, CGRect(x: 20, y: 0, width: 20, height: 20)),
        (#""align":{"x":"start","y":"end"}"#, CGRect(x: 0, y: 0, width: 20, height: 20))
    ])
    func scalingAndAlignmentPlaceTheViewBox(fields: String, bounds: CGRect) throws {
        let host = NSView()
        let renderer = MacWidgetRenderer(view: host)
        try renderer.render(decode(vector(
            layout: CGRect(x: 0, y: 0, width: 40, height: 20),
            drawing: """
            {"view_box":{"min_x":-5,"min_y":-5,"width":10,"height":10},\(fields),
             "paths":[{"data":"M -5 -5 L 5 -5 L 5 5 L -5 5 Z","fill":\(Self.ink)}]}
            """
        )))
        #expect(try shapeLayers(host).first?.path?.boundingBoxOfPath == bounds)
    }

    /// An inner square wound the same way as its outer one is a hole under evenodd and filled under nonzero.
    @Test(arguments: [("evenodd", false), ("nonzero", true)])
    func fillRulesDecideWhetherNestedOutlinesPunchHoles(rule: String, centreFilled: Bool) throws {
        let root = FlippedView(frame: CGRect(x: 0, y: 0, width: 40, height: 40))
        root.wantsLayer = true
        let renderer = MacWidgetRenderer(view: root)
        try renderer.render(decode(vector(
            layout: CGRect(x: 0, y: 0, width: 40, height: 40),
            drawing: """
            {"view_box":{"min_x":0,"min_y":0,"width":40,"height":40},"paths":[{"fill":\(Self.red),
             "fill_rule":"\(rule)","data":"M 0 0 L 40 0 L 40 40 L 0 40 Z M 10 10 L 30 10 L 30 30 L 10 30 Z"}]}
            """
        )))
        let bitmap = try #require(root.bitmapImageRepForCachingDisplay(in: root.bounds))
        root.cacheDisplay(in: root.bounds, to: bitmap)

        #expect(isRed(pixel(bitmap, in: root, at: CGPoint(x: 5, y: 5))))
        #expect(isRed(pixel(bitmap, in: root, at: CGPoint(x: 20, y: 20))) == centreFilled)
    }

    /// Sending an unchanged drawing keeps its layers; a resize remaps the same layers instead of rebuilding them.
    @Test func unchangedDrawingsKeepTheirLayersAndResizesReuseThem() throws {
        let host = NSView()
        let renderer = MacWidgetRenderer(view: host)
        let drawing = """
        {"view_box":{"min_x":0,"min_y":0,"width":10,"height":10},"scaling":"stretch",
         "paths":[{"data":"M 0 0 L 10 10","stroke":{"color":\(Self.ink),"width":1,"scaling":"drawing"}}]}
        """
        try renderer.render(decode(vector(layout: CGRect(x: 0, y: 0, width: 10, height: 10), drawing: drawing)))
        let layer = try #require(try shapeLayers(host).first)
        let path = layer.path

        try renderer.render(decode("""
        {"version":1,"mutations":[{"UpdateProps":{"id":1,"props":{"drawing":\(drawing)}}}]}
        """))
        #expect(try shapeLayers(host).first === layer)
        #expect(layer.path === path, "an unchanged drawing at an unchanged size is not remapped")

        try renderer.render(decode("""
        {"version":1,"mutations":[{"UpdateLayout":{"id":1,"x":0,"y":0,"width":30,"height":30}}]}
        """))
        #expect(try shapeLayers(host) == [layer])
        #expect(layer.path?.boundingBoxOfPath == CGRect(x: 0, y: 0, width: 30, height: 30))
        #expect(layer.lineWidth == 3)
    }

    /// Unknown keywords and invalid numbers fail the transaction rather than drawing something else.
    @Test(arguments: [
        #""scaling":"slice""#,
        #""align":{"x":"middle","y":"start"}"#,
        #""align":{"x":"start"}"#,
        #""paths":[{"data":"M 0 0","fill_rule":"winding"}]"#,
        #""paths":[{"data":"M 0 0","stroke":{"color":0,"width":1,"scaling":"view"}}]"#,
        #""paths":[{"data":"M 0 0","stroke":{"color":0,"width":1,"miter_limit":0.5}}]"#,
        #""paths":[{"data":"M 0 0","stroke":{"color":0,"width":1,"dashes":[-1,2]}}]"#,
        #""paths":[{"data":"M 0 0","stroke":{"color":0,"width":0}}]"#
    ])
    func invalidDrawingFieldsFailToDecode(fields: String) {
        #expect(throws: DecodingError.self) {
            try decode(vector(
                layout: CGRect(x: 0, y: 0, width: 1, height: 1),
                drawing: #"{"view_box":{"min_x":0,"min_y":0,"width":1,"height":1},\#(fields)}"#
            ))
        }
    }

    /// Path data is canonical; anything else fails the transaction instead of drawing a partial shape.
    @Test(arguments: ["M 0 0 A 1 1 0 0 0 2 2", "L 0 0", "M 0", "M 0 0 L x 1"])
    func malformedPathDataFailsToDecode(data: String) {
        #expect(throws: DecodingError.self) {
            try decode(vector(
                layout: CGRect(x: 0, y: 0, width: 1, height: 1),
                drawing: #"{"view_box":{"min_x":0,"min_y":0,"width":1,"height":1},"paths":[{"data":"\#(data)"}]}"#
            ))
        }
    }

    @Test func emptyViewBoxFailsToDecode() {
        #expect(throws: DecodingError.self) {
            try decode(vector(
                layout: CGRect(x: 0, y: 0, width: 1, height: 1),
                drawing: #"{"view_box":{"min_x":0,"min_y":0,"width":0,"height":1}}"#
            ))
        }
    }

    private func vector(
        layout frame: CGRect, drawing: String, label: String? = nil
    ) -> String {
        let accessibility = label.map { #","accessibility_label":"\#($0)""# } ?? ""

        return """
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"vector","props":{"drawing":\(drawing)\(accessibility)}}},
          {"UpdateLayout":{"id":1,"x":\(frame.minX),"y":\(frame.minY),"width":\(frame.width),"height":\(frame.height)}}
        ]}
        """
    }

    private func shapeLayers(_ host: NSView) throws -> [CAShapeLayer] {
        let view = try #require(host.subviews.first as? MacVectorView)

        return view.layer?.sublayers?.compactMap { $0 as? CAShapeLayer } ?? []
    }

    private func color(_ rgba: UInt32) -> CGColor {
        CGColor(
            srgbRed: CGFloat((rgba >> 24) & 255) / 255,
            green: CGFloat((rgba >> 16) & 255) / 255,
            blue: CGFloat((rgba >> 8) & 255) / 255,
            alpha: CGFloat(rgba & 255) / 255
        )
    }

    /// The bitmap may be at the backing scale; read it at a point in `view`'s coordinates.
    private func pixel(_ bitmap: NSBitmapImageRep, in view: NSView, at point: CGPoint) -> NSColor? {
        let scale = CGFloat(bitmap.pixelsWide) / view.bounds.width

        return bitmap.colorAt(x: Int(point.x * scale), y: Int(point.y * scale))
    }

    /// Compares in the bitmap's own color space, as `ButtonPaint` does, so no conversion shifts the channels.
    private func isRed(_ pixel: NSColor?) -> Bool {
        guard let pixel else { return false }

        return pixel.redComponent > 0.9 && pixel.greenComponent < 0.1 && pixel.alphaComponent > 0.9
    }

    private func decode(_ source: String) throws -> Transaction {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase

        return try decoder.decode(Transaction.self, from: Data(source.utf8))
    }
}
