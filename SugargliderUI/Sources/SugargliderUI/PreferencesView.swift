// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import SwiftUI

/// Sidebar sections for preferences navigation
enum PreferencesSection: String, CaseIterable, Identifiable {
    case general = "General"
    case layouts = "Layouts"
    case hotkeys = "Hotkeys"
    case appRules = "App Rules"
    case about = "About"

    var id: String { rawValue }

    var icon: String {
        switch self {
        case .general: return "gear"
        case .layouts: return "rectangle.3.group"
        case .hotkeys: return "keyboard"
        case .appRules: return "app.badge.checkmark"
        case .about: return "info.circle"
        }
    }
}

/// Main preferences window view with sidebar navigation
public struct PreferencesView: View {
    @StateObject private var viewModel = PreferencesViewModel()
    @State private var selectedSection: PreferencesSection = .general

    public init() {}

    public var body: some View {
        HStack(spacing: 0) {
            // Sidebar
            List(PreferencesSection.allCases, selection: $selectedSection) { section in
                Label(section.rawValue, systemImage: section.icon)
                    .tag(section)
            }
            .listStyle(.sidebar)
            .frame(width: 200)

            Divider()

            VStack(spacing: 0) {
                if let error = viewModel.lastError {
                    PreferencesErrorBanner(message: error) {
                        viewModel.lastError = nil
                    }
                    Divider()
                }

                // Detail pane
                Group {
                    switch selectedSection {
                    case .general:
                        GeneralPane(viewModel: viewModel)
                    case .layouts:
                        LayoutPane(viewModel: viewModel)
                    case .hotkeys:
                        HotkeysPane(viewModel: viewModel)
                    case .appRules:
                        AppRulesPane(viewModel: viewModel)
                    case .about:
                        AboutPane()
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(width: 650, height: 620)
    }
}

/// Why the last change was not applied or saved. The message is monospaced
/// so that the markers under a config file error line up with the text.
struct PreferencesErrorBanner: View {
    let message: String
    let onDismiss: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundColor(.orange)
            Text(message)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button(action: onDismiss) {
                Image(systemName: "xmark")
                    .foregroundColor(.secondary)
            }
            .buttonStyle(.plain)
            .help("Dismiss")
        }
        .padding(12)
        .background(Color.orange.opacity(0.12))
    }
}

// MARK: - General Pane

struct GeneralPane: View {
    @ObservedObject var viewModel: PreferencesViewModel

    var body: some View {
        Form {
            Section {
                Toggle("Launch at login", isOn: $viewModel.launchAtLogin)
                    .onChange(of: viewModel.launchAtLogin) { newValue in
                        viewModel.setLaunchAtLogin(newValue)
                    }
                Toggle("Show menu bar icon", isOn: $viewModel.showMenuBarIcon)
                Toggle("Enable animations", isOn: $viewModel.enableAnimations)
            }

            Section("Focus Behavior") {
                Toggle("Focus follows mouse", isOn: $viewModel.focusFollowsMouse)
                Toggle("Mouse follows focus", isOn: $viewModel.mouseFollowsFocus)
            }

            Section("Dragging Behavior") {
                Toggle("Enable drag to rearrange", isOn: $viewModel.dragDropEnable)
                Toggle("Live preview while dragging", isOn: $viewModel.dragDropLivePreview)
                    .disabled(!viewModel.dragDropEnable)
            }

            Section("Window Gaps") {
                HStack {
                    Text("Outer gap:")
                    Slider(value: $viewModel.outerGap, in: 0...32)
                        .onChange(of: viewModel.outerGap) { newValue in
                            viewModel.outerGap = newValue.rounded()
                        }
                    Text("\(Int(viewModel.outerGap))px")
                        .monospacedDigit()
                        .frame(width: 40)
                }

                HStack {
                    Text("Inner gap:")
                    Slider(value: $viewModel.innerGap, in: 0...32)
                        .onChange(of: viewModel.innerGap) { newValue in
                            viewModel.innerGap = newValue.rounded()
                        }
                    Text("\(Int(viewModel.innerGap))px")
                        .monospacedDigit()
                        .frame(width: 40)
                }
            }

            Section {
                Toggle("Contexts (experimental)", isOn: $viewModel.contextsEnable)
            } footer: {
                Text("Named sets of windows that you switch between, each with its own layout.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .formStyle(.grouped)
        .padding()
    }
}

// MARK: - Layout Pane

struct LayoutPane: View {
    @ObservedObject var viewModel: PreferencesViewModel

    var body: some View {
        Form {
            Section("Default Layout") {
                Picker("Layout mode:", selection: $viewModel.defaultLayout) {
                    Text("Tree (Binary Split)").tag(LayoutMode.tree)
                    Text("Column (Horizontal Scroll)").tag(LayoutMode.column)
                }
                .pickerStyle(.radioGroup)
            }

            Section("Tree Layout Options") {
                Picker("Default split direction:", selection: $viewModel.defaultSplitDirection) {
                    Text("Horizontal").tag(SplitDirection.horizontal)
                    Text("Vertical").tag(SplitDirection.vertical)
                    Text("Auto (based on window size)").tag(SplitDirection.auto)
                }
            }

            Section("Column Layout Options") {
                HStack {
                    Text("Default column width:")
                    Slider(value: $viewModel.defaultColumnWidth, in: 200...800)
                        .onChange(of: viewModel.defaultColumnWidth) { newValue in
                            viewModel.defaultColumnWidth = (newValue / 10).rounded() * 10
                        }
                    Text("\(Int(viewModel.defaultColumnWidth))px")
                        .monospacedDigit()
                        .frame(width: 50)
                }
            }
        }
        .formStyle(.grouped)
        .padding()
    }
}

// MARK: - Hotkeys Pane

struct HotkeysPane: View {
    @ObservedObject var viewModel: PreferencesViewModel
    @State private var recordingCommandId: String? = nil

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if viewModel.hotkeys.isEmpty {
                    Text("No hotkeys configured")
                        .foregroundColor(.secondary)
                        .frame(maxWidth: .infinity, alignment: .center)
                        .padding(.top, 40)
                } else {
                    ForEach(viewModel.hotkeysByCategory, id: \.category) { group in
                        HotkeySection(
                            category: group.category,
                            bindings: group.bindings,
                            recordingCommandId: $recordingCommandId,
                            onHotkeyChange: { commandId, newKey in
                                viewModel.updateHotkey(commandId: commandId, newKey: newKey)
                            },
                            onResetHotkey: { commandId in
                                viewModel.resetHotkeyToDefault(commandId: commandId)
                            }
                        )
                    }
                }
            }
            .padding()
        }
    }
}

struct HotkeySection: View {
    let category: String
    let bindings: [HotkeyBinding]
    @Binding var recordingCommandId: String?
    let onHotkeyChange: (String, String) -> Void
    let onResetHotkey: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(category)
                .font(.headline)
                .foregroundColor(.secondary)

            VStack(spacing: 0) {
                ForEach(Array(bindings.enumerated()), id: \.element.id) { index, binding in
                    HotkeyRow(
                        binding: binding,
                        isRecording: recordingCommandId == binding.commandId,
                        onStartRecording: {
                            recordingCommandId = binding.commandId
                        },
                        onStopRecording: {
                            recordingCommandId = nil
                        },
                        onHotkeyChange: { newKey in
                            onHotkeyChange(binding.commandId, newKey)
                            recordingCommandId = nil
                        },
                        onReset: {
                            onResetHotkey(binding.commandId)
                        }
                    )
                    if index < bindings.count - 1 {
                        Divider()
                    }
                }
            }
            .background(Color(NSColor.controlBackgroundColor))
            .cornerRadius(8)
        }
    }
}

struct HotkeyRow: View {
    let binding: HotkeyBinding
    let isRecording: Bool
    let onStartRecording: () -> Void
    let onStopRecording: () -> Void
    let onHotkeyChange: (String) -> Void
    let onReset: () -> Void

    var body: some View {
        HStack {
            Text(binding.description)
                .lineLimit(1)
            Spacer()

            // Reset button (shown when hotkey differs from default)
            if binding.isModified {
                Button(action: onReset) {
                    Image(systemName: "arrow.counterclockwise")
                        .foregroundColor(.secondary)
                }
                .buttonStyle(.plain)
                .help("Reset to default: \(binding.defaultKey ?? "")")
            }

            HotkeyButton(
                currentKey: binding.key,
                isRecording: isRecording,
                onStartRecording: onStartRecording,
                onStopRecording: onStopRecording,
                onHotkeyChange: onHotkeyChange
            )
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }
}

/// A button that captures keyboard shortcuts when clicked
struct HotkeyButton: View {
    let currentKey: String
    let isRecording: Bool
    let onStartRecording: () -> Void
    let onStopRecording: () -> Void
    let onHotkeyChange: (String) -> Void

    var body: some View {
        Button(action: {
            if isRecording {
                onStopRecording()
            } else {
                onStartRecording()
            }
        }) {
            Text(isRecording ? "Press keys..." : currentKey)
                .font(.system(.body, design: .monospaced))
                .foregroundColor(isRecording ? .accentColor : .secondary)
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .frame(minWidth: 80)
                .background(
                    RoundedRectangle(cornerRadius: 4)
                        .fill(isRecording
                            ? Color.accentColor.opacity(0.15)
                            : Color(NSColor.quaternaryLabelColor).opacity(0.3))
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 4)
                        .stroke(isRecording ? Color.accentColor : Color.clear, lineWidth: 1)
                )
        }
        .buttonStyle(.plain)
        .background(
            HotkeyRecorderView(
                isRecording: isRecording,
                onHotkeyRecorded: onHotkeyChange,
                onCancel: onStopRecording
            )
        )
    }
}

/// NSViewRepresentable that captures keyboard events for hotkey recording
struct HotkeyRecorderView: NSViewRepresentable {
    let isRecording: Bool
    let onHotkeyRecorded: (String) -> Void
    let onCancel: () -> Void

    func makeNSView(context: Context) -> HotkeyRecorderNSView {
        let view = HotkeyRecorderNSView()
        view.onHotkeyRecorded = onHotkeyRecorded
        view.onCancel = onCancel
        return view
    }

    func updateNSView(_ nsView: HotkeyRecorderNSView, context: Context) {
        nsView.onHotkeyRecorded = onHotkeyRecorded
        nsView.onCancel = onCancel
        if isRecording {
            // Delay to allow the button click to complete
            DispatchQueue.main.async {
                nsView.window?.makeFirstResponder(nsView)
            }
        }
    }
}

/// NSView that captures keyboard events
class HotkeyRecorderNSView: NSView {
    var onHotkeyRecorded: ((String) -> Void)?
    var onCancel: (() -> Void)?

    override var acceptsFirstResponder: Bool { true }

    override func keyDown(with event: NSEvent) {
        // Escape cancels recording
        if event.keyCode == 53 {
            onCancel?()
            return
        }

        // Build the hotkey string
        let hotkey = formatHotkey(event: event)
        if !hotkey.isEmpty {
            onHotkeyRecorded?(hotkey)
        }
    }

    private func formatHotkey(event: NSEvent) -> String {
        var result = ""

        // Add modifiers in standard macOS order
        let flags = event.modifierFlags

        if flags.contains(.control) {
            result += "⌃"
        }
        if flags.contains(.option) {
            result += "⌥"
        }
        if flags.contains(.shift) {
            result += "⇧"
        }
        if flags.contains(.command) {
            result += "⌘"
        }

        // Need at least one modifier for a valid hotkey
        if result.isEmpty {
            return ""
        }

        // Add the key
        if let keyName = keyCodeToName(event.keyCode) {
            result += keyName
        } else if let chars = event.charactersIgnoringModifiers?.uppercased(), !chars.isEmpty {
            result += chars
        } else {
            return ""
        }

        return result
    }

    private func keyCodeToName(_ keyCode: UInt16) -> String? {
        switch keyCode {
        case 0: return "A"
        case 1: return "S"
        case 2: return "D"
        case 3: return "F"
        case 4: return "H"
        case 5: return "G"
        case 6: return "Z"
        case 7: return "X"
        case 8: return "C"
        case 9: return "V"
        case 11: return "B"
        case 12: return "Q"
        case 13: return "W"
        case 14: return "E"
        case 15: return "R"
        case 16: return "Y"
        case 17: return "T"
        case 18: return "1"
        case 19: return "2"
        case 20: return "3"
        case 21: return "4"
        case 22: return "6"
        case 23: return "5"
        case 24: return "="
        case 25: return "9"
        case 26: return "7"
        case 27: return "-"
        case 28: return "8"
        case 29: return "0"
        case 30: return "]"
        case 31: return "O"
        case 32: return "U"
        case 33: return "["
        case 34: return "I"
        case 35: return "P"
        case 37: return "L"
        case 38: return "J"
        case 39: return "'"
        case 40: return "K"
        case 41: return ";"
        case 42: return "\\"
        case 43: return ","
        case 44: return "/"
        case 45: return "N"
        case 46: return "M"
        case 47: return "."
        case 50: return "`"
        case 36: return "↩"
        case 48: return "⇥"
        case 49: return "Space"
        case 51: return "⌫"
        case 123: return "←"
        case 124: return "→"
        case 125: return "↓"
        case 126: return "↑"
        default: return nil
        }
    }
}

// MARK: - App Rules Pane

struct AppRulesPane: View {
    @ObservedObject var viewModel: PreferencesViewModel

    var body: some View {
        VStack(alignment: .leading) {
            Text("Configure per-app behavior")
                .font(.headline)
                .padding(.bottom, 4)

            List {
                ForEach(viewModel.appRules) { rule in
                    AppRuleRow(rule: rule)
                }
                .onDelete { indexSet in
                    viewModel.appRules.remove(atOffsets: indexSet)
                }
            }
            .listStyle(.bordered)

            HStack {
                Button(action: { viewModel.addAppRule() }) {
                    Label("Add Rule", systemImage: "plus")
                }
                Spacer()
            }
            .padding(.top, 8)
        }
        .padding()
    }
}

struct AppRuleRow: View {
    let rule: AppRule

    var body: some View {
        HStack {
            Image(systemName: "app.fill")
                .foregroundColor(.secondary)
            Text(rule.appName)
            Spacer()
            Text(rule.behavior.rawValue)
                .foregroundColor(.secondary)
        }
    }
}

// MARK: - About Pane

struct AboutPane: View {
    var body: some View {
        VStack(spacing: 16) {
            SugargliderIcon(size: 80)
                .help("Click me!")

            Text("Sugarglider")
                .font(.largeTitle)
                .fontWeight(.bold)

            Text("Version 0.1.0")
                .foregroundColor(.secondary)

            Text("A tiling window manager for macOS")
                .multilineTextAlignment(.center)

            Divider()
                .padding(.vertical)

            VStack(spacing: 8) {
                Text("Based on Glide by Tyler Mandry")
                    .font(.caption)
                    .foregroundColor(.secondary)

                Link("View on GitHub", destination: URL(string: "https://github.com/rdbeerman/sugarglider")!)
                    .font(.caption)
            }

            Spacer()
        }
        .padding()
    }
}

// MARK: - Preview

#Preview {
    PreferencesView()
}
