// Settings: a sidebar (General, Projects, About) over grouped cards. Every change is sent
// at once as a `config.set {patch}`; a rejected one shows the daemon's message and the
// last good values come back (contract §6, §10).
import AppKit
import ServiceManagement
import SwiftUI
import UniformTypeIdentifiers
import LullKit

enum SettingsSection: String, CaseIterable, Identifiable {
    case general, projects, about
    var id: Self { self }
    var title: String { rawValue.capitalized }
    var symbol: String {
        switch self {
        case .general: "gearshape"
        case .projects: "folder"
        case .about: "info.circle"
        }
    }
}

struct SettingsView: View {
    @Environment(Store.self) private var store
    @State private var section: SettingsSection? = .general
    /// The last config the daemon accepted, and what the controls show.
    @State private var saved: Config?
    @State private var draft: Config?
    @State private var error: String?

    var body: some View {
        NavigationSplitView {
            List(SettingsSection.allCases, selection: $section) { s in
                Label(s.title, systemImage: s.symbol).tag(s)
            }
            .navigationSplitViewColumnWidth(170)
        } detail: {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text((section ?? .general).title).font(.system(size: 20, weight: .bold))
                    if let error {
                        Label(error, systemImage: "exclamationmark.triangle.fill")
                            .font(.system(size: 12))
                            .foregroundStyle(.red)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    switch section ?? .general {
                    case .general: general
                    case .projects: projects
                    case .about: AboutSettings()
                    }
                }
                .padding(24)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(minWidth: 620, minHeight: 420)
        .task(id: store.phase) { await load() }
    }

    // MARK: Loading and saving

    private func load() async {
        guard store.phase == .connected else {
            saved = nil
            draft = nil
            return
        }
        do {
            saved = try await store.config()
            draft = saved
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func save(_ patch: ConfigPatch) {
        Task {
            do {
                saved = try await store.setConfig(patch)
                error = nil
            } catch {
                self.error = error.localizedDescription
            }
            draft = saved
        }
    }

    /// A control bound to one config field; setting it saves that field.
    private func binding<T>(_ path: WritableKeyPath<Config, T>, _ patch: @escaping (T) -> ConfigPatch) -> Binding<T>? {
        guard let draft else { return nil }
        return Binding(
            get: { self.draft?[keyPath: path] ?? draft[keyPath: path] },
            set: { value in
                self.draft?[keyPath: path] = value
                save(patch(value))
            })
    }

    // MARK: General

    @ViewBuilder private var general: some View {
        if let idle = binding(\.idleMinutes, { ConfigPatch(idleMinutes: $0) }),
           let scope = binding(\.autoStop, { ConfigPatch(autoStop: $0) }),
           let warn = binding(\.warnBeforeMinutes, { ConfigPatch(warnBeforeMinutes: $0) }),
           let tabs = binding(\.tabsOnStop, { ConfigPatch(tabsOnStop: $0) }),
           let openWith = binding(\.openWith, { ConfigPatch(openWith: $0) })
        {
            SettingsGroup(title: "Stopping") {
                SettingRow(title: "Stop idle servers after", detail: "Without a visible tab or traffic for this long.") {
                    Stepper("\(idle.wrappedValue) min", value: idle, in: 1...1440, step: 5)
                }
                Divider()
                SettingRow(title: "Stop automatically", detail: "Servers outside this scope are listed but never stopped.") {
                    Picker("", selection: scope) {
                        Text("Agent-started").tag(AutoStop.agent)
                        Text("All servers").tag(AutoStop.all)
                        Text("None").tag(AutoStop.none)
                    }
                    .labelsHidden()
                    .fixedSize()
                }
                Divider()
                SettingRow(title: "Warn before stopping", detail: "Notification with Keep alive, this long before.") {
                    Stepper("\(warn.wrappedValue) min", value: warn, in: 0...60)
                }
                Divider()
                SettingRow(title: "Tabs of a stopped server", detail: "In browsers with the lull extension.") {
                    Picker("", selection: tabs) {
                        Text("Discard").tag(TabsOnStop.discard)
                        Text("Close").tag(TabsOnStop.close)
                        Text("Keep").tag(TabsOnStop.keep)
                    }
                    .labelsHidden()
                    .fixedSize()
                }
            }
            SettingsGroup(title: "Opening") {
                SettingRow(title: "Open servers with", detail: "Used when no browser with the extension is connected.") {
                    HStack {
                        Text(openWith.wrappedValue.isEmpty ? "System default" : appName(openWith.wrappedValue))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                        Button("Choose…") { if let app = chooseApp() { openWith.wrappedValue = app } }
                        if !openWith.wrappedValue.isEmpty {
                            Button("Default") { openWith.wrappedValue = "" }
                        }
                    }
                }
            }
        } else {
            NotConnected()
        }
        SettingsGroup(title: "Menu bar") {
            MenuBarToggles()
        }
        SettingsGroup(title: "App") {
            LaunchAtLogin()
        }
    }

    private func appName(_ path: String) -> String {
        FileManager.default.displayName(atPath: path).replacingOccurrences(of: ".app", with: "")
    }

    private func chooseApp() -> String? {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.application]
        panel.directoryURL = URL(fileURLWithPath: "/Applications")
        panel.prompt = "Choose"
        return panel.runModal() == .OK ? panel.url?.path : nil
    }

    // MARK: Projects

    @ViewBuilder private var projects: some View {
        if let draft {
            let rules = draft.projects ?? []
            Text("Rules for servers whose directory is inside a path. The longest matching path wins; empty fields follow General.")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            SettingsGroup(title: "Project rules") {
                if rules.isEmpty {
                    Text("No rules yet.").foregroundStyle(.secondary).padding(.vertical, 4)
                }
                ForEach(Array(rules.enumerated()), id: \.offset) { i, rule in
                    if i > 0 { Divider() }
                    ProjectRuleRow(rule: rule, onChange: { rule in
                        var rules = rules
                        rules[i] = rule
                        saveRules(rules)
                    }, onRemove: {
                        var rules = rules
                        rules.remove(at: i)
                        saveRules(rules)
                    })
                }
            }
            Button("Add Project…") {
                if let path = chooseFolder() { saveRules(rules + [ProjectRule(path: path)]) }
            }
        } else {
            NotConnected()
        }
    }

    private func saveRules(_ rules: [ProjectRule]) {
        draft?.projects = rules
        save(ConfigPatch(projects: rules))
    }

    private func chooseFolder() -> String? {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.prompt = "Add"
        guard panel.runModal() == .OK, let path = panel.url?.path else { return nil }
        return Format.tilde(path)
    }
}

private struct ProjectRuleRow: View {
    var rule: ProjectRule
    var onChange: (ProjectRule) -> Void
    var onRemove: () -> Void
    @State private var idle: Int?

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "folder").foregroundStyle(.secondary)
            Text(rule.path)
                .font(.system(size: 12, design: .monospaced))
                .lineLimit(1)
                .truncationMode(.middle)
                .help(rule.path)
            Spacer()
            TextField("default", value: $idle, format: .number)
                .frame(width: 60)
                .multilineTextAlignment(.trailing)
                .onSubmit { onChange(ProjectRule(path: rule.path, idleMinutes: idle, autoStop: rule.autoStop)) }
            Text("min").foregroundStyle(.secondary)
            Picker("", selection: Binding(
                get: { rule.autoStop },
                set: { onChange(ProjectRule(path: rule.path, idleMinutes: rule.idleMinutes, autoStop: $0)) }
            )) {
                Text("Default").tag(AutoStop?.none)
                Text("Agent-started").tag(AutoStop?.some(.agent))
                Text("All servers").tag(AutoStop?.some(.all))
                Text("None").tag(AutoStop?.some(AutoStop.none))
            }
            .labelsHidden()
            .fixedSize()
            Button(action: onRemove) { Image(systemName: "minus.circle") }
                .buttonStyle(.borderless)
                .help("Remove rule")
        }
        .onAppear { idle = rule.idleMinutes }
        .onChange(of: rule) { idle = rule.idleMinutes }
    }
}

// MARK: About

private struct AboutSettings: View {
    @Environment(Store.self) private var store

    var body: some View {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
        SettingsGroup(title: "lull") {
            SettingRow(title: "App", detail: nil) { Text(version).foregroundStyle(.secondary) }
            Divider()
            SettingRow(title: "Daemon", detail: LullPaths.socket) {
                Text(store.daemonVersion ?? "not connected").foregroundStyle(.secondary)
            }
            if let status = store.status {
                Divider()
                SettingRow(title: "Activity", detail: status.activityError) {
                    Text("\(status.activityBackend.rawValue), \(status.activityHealth.rawValue)")
                        .foregroundStyle(.secondary)
                }
                Divider()
                SettingRow(title: "Browsers with the extension", detail: "Optional: lull browsers install adds it.") {
                    Text(status.browsers.isEmpty ? "none" : status.browsers.map(\.name).joined(separator: ", "))
                        .foregroundStyle(.secondary)
                }
            }
        }
        .task { await store.refresh() }
        Text("Licensed MIT OR Apache-2.0. lull makes no network requests.")
            .font(.system(size: 11))
            .foregroundStyle(.secondary)
    }
}

// MARK: Menu bar

/// App-only display preferences, kept in UserDefaults and never sent to the daemon.
private struct MenuBarToggles: View {
    @AppStorage(MenuBarPrefs.showCount) private var showCount = false
    @AppStorage(MenuBarPrefs.showDot) private var showDot = false

    var body: some View {
        SettingRow(title: "Show the number of running servers", detail: "Beside the cat in the menu bar.") {
            Toggle("", isOn: $showCount).toggleStyle(.switch).labelsHidden()
        }
        Divider()
        SettingRow(title: "Show a dot when a server is stopping soon", detail: nil) {
            Toggle("", isOn: $showDot).toggleStyle(.switch).labelsHidden()
        }
    }
}

// MARK: Launch at login

private struct LaunchAtLogin: View {
    @State private var enabled = SMAppService.mainApp.status == .enabled
    @State private var error: String?

    var body: some View {
        SettingRow(title: "Open at login", detail: error ?? approvalHint) {
            Toggle("", isOn: Binding(get: { enabled }, set: set))
                .toggleStyle(.switch)
                .labelsHidden()
        }
    }

    private var approvalHint: String? {
        SMAppService.mainApp.status == .requiresApproval
            ? "Allow lull in System Settings › General › Login Items." : nil
    }

    private func set(_ on: Bool) {
        do {
            if on { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
            error = nil
        } catch {
            self.error = "Could not change it: \(error.localizedDescription)"
        }
        enabled = SMAppService.mainApp.status == .enabled
    }
}

// MARK: Layout

private struct NotConnected: View {
    var body: some View {
        SettingsGroup(title: "Daemon") {
            Text("Settings live in the lull daemon, which isn't connected. Start it with `brew services start lull`.")
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

struct SettingsGroup<Content: View>: View {
    var title: String
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            SectionTitle(text: title)
            Card {
                VStack(alignment: .leading, spacing: 10) { content }
                    .padding(14)
            }
        }
    }
}

struct SettingRow<Control: View>: View {
    var title: String
    var detail: String?
    @ViewBuilder var control: Control

    var body: some View {
        HStack(alignment: .center, spacing: 16) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.system(size: 13))
                if let detail {
                    Text(detail).font(.system(size: 11)).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 12)
            control
        }
    }
}
