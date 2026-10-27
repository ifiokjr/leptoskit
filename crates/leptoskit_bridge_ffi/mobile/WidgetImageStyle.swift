import UIKit

/// File-backed images remain synchronous and reject remote image sources.
@MainActor
enum WidgetImageStyle {
    static func apply(_ props: WidgetProps, to widget: UIView) throws {
        guard let image = widget as? UIImageView else { return }
        image.contentMode = .scaleAspectFit
        if image is NativeRiveImageView {
            image.isAccessibilityElement = props.aria?.ariaHidden != true
            image.accessibilityTraits = .image
            return
        }
        image.image = try props.source.map(load)
    }

    private static func load(_ source: String) throws -> UIImage {
        guard let url = URL(string: source), url.isFileURL,
            let loaded = UIImage(contentsOfFile: url.path)
        else {
            throw RuntimeFailure(message: "Image requires a readable file URL: \(source)")
        }

        return loaded
    }
}
