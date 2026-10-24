import UIKit

/// File-backed images remain synchronous and reject remote image sources.
enum WidgetImageStyle {
    static func apply(_ props: WidgetProps, to widget: UIView) throws {
        guard let image = widget as? UIImageView else { return }
        image.contentMode = .scaleAspectFit

        guard let source = props.source else {
            image.image = nil

            return
        }

        guard let url = URL(string: source), url.isFileURL,
            let loaded = UIImage(contentsOfFile: url.path)
        else {
            throw RuntimeFailure(message: "Image requires a readable file URL: \(source)")
        }

        image.image = loaded
    }
}
