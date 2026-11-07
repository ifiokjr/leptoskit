import Foundation

#if canImport(ActivityKit)
import ActivityKit

/// Portable Live Activity attributes shared by the plugin and Widget Extensions.
///
/// Apps that want Lock Screen or Dynamic Island UI must include a Widget
/// Extension that renders this type. The plugin starts and updates activities;
/// it does not ship widget chrome.
@available(iOS 16.1, *)
public struct LeptosKitLiveActivityAttributes: ActivityAttributes {
    public struct ContentState: Codable, Hashable, Sendable {
        public var fields: [String: String]

        public init(fields: [String: String]) {
            self.fields = fields
        }
    }

    /// Application-chosen activity identifier.
    public var id: String
    /// Grouping key for the Widget Extension.
    public var kind: String
    /// Static title stored for the lifetime of the activity.
    public var title: String

    public init(id: String, kind: String, title: String) {
        self.id = id
        self.kind = kind
        self.title = title
    }
}
#endif
