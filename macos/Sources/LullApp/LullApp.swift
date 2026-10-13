// The menu-bar app (contract §10): an accessory app with a status item and its panel.
// It is a pure client of the daemon socket and owns no policy.
import AppKit
import SwiftUI
import LullKit

@main
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var store: Store!
    private var statusItem: StatusItem!
    private var settings: SettingsWindow!
    private var notifier: Notifier?

    static func main() {
        // build-app.sh: `Lull --write-iconset <dir>` writes the bundle icon from the logo grid.
        let args = CommandLine.arguments
        if args.count == 3, args[1] == "--write-iconset" {
            do {
                try Logo.writeIconset(to: URL(fileURLWithPath: args[2]))
                exit(0)
            } catch {
                FileHandle.standardError.write(Data("\(error)\n".utf8))
                exit(1)
            }
        }
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
        notifier = Notifier.make(store: store)
        store.onServerChange = { [notifier] old, new in notifier?.serverChanged(from: old, to: new) }
        if preview == nil {
            Task { await store.run() }
        }
        #if DEBUG
        if ProcessInfo.processInfo.environment["LULL_PANEL_WINDOW"] != nil { showPanelWindow() }
        #endif
    }

    #if DEBUG
    private var panelWindow: NSWindow?

    /// The panel in an ordinary window, for UI automation that cannot drive a status item.
    private func showPanelWindow() {
        NSApp.setActivationPolicy(.regular)
        let view = MenuView(openSettings: { [settings] in settings?.show() })
            .environment(store)
            .background(.regularMaterial)
        let host = NSHostingController(rootView: view)
        host.sizingOptions = .preferredContentSize
        let window = NSWindow(contentViewController: host)
        window.title = "lull panel (debug)"
        window.styleMask = [.titled, .closable]
        window.setFrameTopLeftPoint(NSPoint(x: 200, y: (NSScreen.main?.visibleFrame.maxY ?? 900) - 40))
        panelWindow = window
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
    }
    #endif
}
