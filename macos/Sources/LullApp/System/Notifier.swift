// The stopping-soon notification (contract §7.1): posted when a server enters
// `stopping-soon`, withdrawn when it leaves that state or goes away. *Keep alive* sends
// `servers.touch`.
import Foundation
@preconcurrency import UserNotifications
import LullKit

@MainActor
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    private nonisolated static let category = "stopping-soon"
    private nonisolated static let keepAlive = "keep-alive"
    private let store: Store
    private let center = UNUserNotificationCenter.current()

    /// `nil` outside an app bundle (`swift run`), where the notification center is unavailable.
    static func make(store: Store) -> Notifier? {
        Bundle.main.bundleIdentifier == nil ? nil : Notifier(store: store)
    }

    private init(store: Store) {
        self.store = store
        super.init()
        center.delegate = self
        let action = UNNotificationAction(identifier: Self.keepAlive, title: "Keep alive")
        center.setNotificationCategories([
            UNNotificationCategory(identifier: Self.category, actions: [action], intentIdentifiers: [])
        ])
        center.requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    func serverChanged(from old: Server?, to new: Server?) {
        let wasSoon = old?.state == .stoppingSoon
        if let new, new.state == .stoppingSoon, !wasSoon {
            post(new)
        } else if wasSoon, new?.state != .stoppingSoon, let id = old?.id {
            let ids = [Self.identifier(id)]
            center.removeDeliveredNotifications(withIdentifiers: ids)
            center.removePendingNotificationRequests(withIdentifiers: ids)
        }
    }

    private static func identifier(_ id: String) -> String { "stopping-soon.\(id)" }

    private func post(_ server: Server) {
        let content = UNMutableNotificationContent()
        let port = server.ports.first.map { " :\($0)" } ?? ""
        content.title = "\(server.project)\(port) stops soon"
        let when = server.stopsAt.map { " at \($0.formatted(date: .omitted, time: .shortened))" } ?? ""
        content.body = "Idle, so lull stops \(server.command)\(when) unless you keep it alive."
        content.categoryIdentifier = Self.category
        content.userInfo = ["server": server.id]
        center.add(UNNotificationRequest(identifier: Self.identifier(server.id), content: content, trigger: nil))
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse
    ) async {
        guard response.actionIdentifier == Self.keepAlive,
              let id = response.notification.request.content.userInfo["server"] as? String
        else { return }
        await store.touch(id)
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .sound]
    }
}
