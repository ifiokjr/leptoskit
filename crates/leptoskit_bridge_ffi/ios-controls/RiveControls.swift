import UIKit

extension UIKitControls {
    func nativeRiveImage() async throws {
        let host = UIView(frame: CGRect(x: 0, y: 0, width: 128, height: 128))
        window?.rootViewController?.view.addSubview(host)
        defer { host.removeFromSuperview() }
        let renderer = MobileWidgetRenderer(view: host)
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let create = """
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"rive","props":{"accessibility_label":"Triangle"}}},
          {"UpdateLayout":{"id":1,"x":0,"y":0,"width":128,"height":128}}
        ]}
        """
        try renderer.render(decoder.decode(Transaction.self, from: Data(create.utf8)))
        guard let widget = host.subviews.first as? UIImageView else {
            record(false, "Rive creates a native UIImageView")
            return
        }
        var pixels: [UInt8] = [128, 0, 0, 128, 0, 0, 255, 255]
        try pixels.withUnsafeBufferPointer { try renderer.presentRive(1, width: 2, height: 1, pixels: $0) }
        let image = widget.image
        pixels = Array(repeating: 0, count: pixels.count)
        if let data = image?.cgImage?.dataProvider?.data {
            let actual = Array(UnsafeBufferPointer(start: CFDataGetBytePtr(data), count: CFDataGetLength(data)))
            record(actual == [128, 0, 0, 128, 0, 0, 255, 255], "Rive preserves RGBA alpha and owns uploaded pixels")
        } else { record(false, "Rive uploads a real native image") }
        record(widget.isAccessibilityElement, "named Rive image is accessible")
        let update = """
        {"version":1,"mutations":[{"UpdateProps":{"id":1,"props":{"aria":{"aria_hidden":true}}}}]}
        """
        try renderer.render(decoder.decode(Transaction.self, from: Data(update.utf8)))
        record(widget.image === image, "paused Rive pose survives property updates")
        record(!widget.isAccessibilityElement, "decorative Rive image leaves accessibility traversal")
        host.isHidden = true
        record(!renderer.riveVisible(1), "hidden Rive ancestor stops frame work")
        host.isHidden = false
        try await nativeRiveDisplay(host, renderer: renderer)
        renderer.close()
        record(widget.image == nil, "closing the renderer releases the Rive image")
    }

    private func nativeRiveDisplay(_ host: UIView, renderer: MobileWidgetRenderer) async throws {
        let animator = NativeRiveAnimator(view: host)
        defer { animator.close() }
        animator.sink = renderer
        var frames = 0, queries = 0
        var reentered = false
        var elapsedTimes: [Double] = []
        animator.hasScenes = { queries += 1; return true }
        animator.tick = { [weak animator] elapsed, _, _, callbacks in
            frames += 1
            elapsedTimes.append(elapsed)
            let previousQueries = queries
            animator?.requestFrame()
            reentered = reentered || queries != previousQueries
            let pixels: [UInt8] = [0, 128, 0, 128]
            let uploaded = pixels.withUnsafeBufferPointer {
                callbacks.present(callbacks.context, 1, 1, 1, $0.baseAddress, $0.count)
            }
            return uploaded == 1 && frames < 2
        }
        let active = await until { UIApplication.shared.applicationState == .active }
        record(active, "display test reaches an active application")
        animator.setPresented(true)
        let rendered = await until { frames == 2 }
        record(rendered && widgetHasGreenPixel(host), "display link uploads real native image frames")
        record(!reentered && elapsedTimes.first == 0, "display callback avoids reentry and starts without elapsed time")
        try await Task.sleep(for: .milliseconds(80))
        record(frames == 2, "idle Rive display link stops scheduling")
        animator.setPresented(false)
        let suspendedQueries = queries
        animator.requestFrame()
        try await Task.sleep(for: .milliseconds(80))
        record(frames == 2 && queries == suspendedQueries,
            "disappeared controllers cannot restart from environment notifications")
        animator.setPresented(true)
        let resumed = await until { frames == 3 }
        record(resumed && elapsedTimes.last == 0, "explicit wake resumes without accumulating idle time")
        animator.close()
        let previousQueries = queries
        animator.requestFrame()
        try await Task.sleep(for: .milliseconds(80))
        record(frames == 3 && queries == previousQueries, "closed animator releases its display callback")
    }

    private func widgetHasGreenPixel(_ host: UIView) -> Bool {
        guard let image = (host.subviews.first as? UIImageView)?.image?.cgImage,
            let data = image.dataProvider?.data, CFDataGetLength(data) == 4 else { return false }
        return Array(UnsafeBufferPointer(start: CFDataGetBytePtr(data), count: 4)) == [0, 128, 0, 128]
    }
}
