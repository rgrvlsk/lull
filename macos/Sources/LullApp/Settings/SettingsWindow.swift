// The Settings window, created on first use and brought to the front after.
import AppKit
import SwiftUI
import LullKit

@MainActor
final class SettingsWindow {
    private var window: NSWindow?
    private let store: Store

    init(store: Store) {
        self.store = store
    }

    func show() {
        if window == nil {
            let host = NSHostingController(rootView: SettingsView().environment(store))
            let w = NSWindow(contentViewController: host)
            w.title = "lull Settings"
            w.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
            w.setContentSize(NSSize(width: 720, height: 520))
            w.isReleasedWhenClosed = false
            w.center()
            window = w
        }
        NSApp.activate()
        window?.makeKeyAndOrderFront(nil)
    }
}
