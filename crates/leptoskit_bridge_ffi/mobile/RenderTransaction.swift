import Foundation

struct WidgetProps: Decodable {
    let text: String?
    let source: String?
    let placeholder: String?
    let value: String?
    let accessibilityLabel: String?
    let color: UInt32?
    let background: UInt32?
    let enabled: Bool?
    let hidden: Bool?
    let clip: Bool?
    let cornerRadius: CGFloat?
    let borderWidth: CGFloat?
    let borderColor: UInt32?
    let opacity: CGFloat?
    let fontSize: CGFloat?
    let fontWeight: String?
    let textAlign: String?
}

struct WidgetLayout: Decodable {
    let id: Int
    let offsetX: CGFloat
    let offsetY: CGFloat
    let width: CGFloat
    let height: CGFloat
    enum CodingKeys: String, CodingKey {
        case id, width, height
        case offsetX = "x"
        case offsetY = "y"
    }
}

struct Change: Decodable {
    struct Widget: Decodable { let id: Int; let kind: String?; let props: WidgetProps }
    struct Edge: Decodable { let parent: Int; let child: Int; let index: Int? }
    struct Removed: Decodable { let id: Int }
    let create: Widget?
    let updateProps: Widget?
    let updateLayout: WidgetLayout?
    let insertChild: Edge?
    let removeChild: Edge?
    let delete: Removed?
    enum CodingKeys: String, CodingKey {
        case create = "Create"
        case updateProps = "UpdateProps"
        case updateLayout = "UpdateLayout"
        case insertChild = "InsertChild"
        case removeChild = "RemoveChild"
        case delete = "Delete"
    }
}

struct Transaction: Decodable {
    let version: Int
    let root: Int?
    let mutations: [Change]
}
