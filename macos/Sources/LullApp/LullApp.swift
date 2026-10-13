// The menu-bar app (contract §10): an accessory app with a status item and its panel.
// It is a pure client of the daemon socket and owns no policy.
import AppKit
import LullKit

@main
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var store: Store!
    private var statusItem: StatusItem!
    private var settings: SettingsWindow!

    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.setActivationPolicy(.accessory)
        app.run()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        #if DEBUG
        let preview = PreviewData.launchStore()
        #else
        let preview: Store? = nil
        #endif
        store = preview ?? Store()
        settings = SettingsWindow(store: store)
        statusItem = StatusItem(store: store, openSettings: { [settings] in settings?.show() })
        if preview == nil {
            Task { await store.run() }
        }
    }
}
