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
    @State private var recordingBindingId: HotkeyBinding.ID? = nil

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
                            recordingBindingId: $recordingBindingId,
                            onHotkeyChange: { id, newKey in
                                viewModel.updateHotkey(id: id, newKey: newKey)
                            },
                            onResetHotkey: { id in
                                viewModel.resetHotkeyToDefault(id: id)
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
    @Binding var recordingBindingId: HotkeyBinding.ID?
    let onHotkeyChange: (HotkeyBinding.ID, String) -> Void
    let onResetHotkey: (HotkeyBinding.ID) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(category)
                .font(.headline)
                .foregroundColor(.secondary)

            VStack(spacing: 0) {
                ForEach(Array(bindings.enumerated()), id: \.element.id) { index, binding in
                    HotkeyRow(
                        binding: binding,
                        isRecording: recordingBindingId == binding.id,
                        onStartRecording: {
                            recordingBindingId = binding.id
                        },
                        onStopRecording: {
                            recordingBindingId = nil
                        },
                        onHotkeyChange: { newKey in
                            onHotkeyChange(binding.id, newKey)
                            recordingBindingId = nil
                        },
                        onReset: {
                            onResetHotkey(binding.id)
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

        let hotkey = Self.hotkeyText(modifierFlags: event.modifierFlags, keyCode: event.keyCode)
        if !hotkey.isEmpty {
            onHotkeyRecorded?(hotkey)
        }
    }

    /// The text of a recorded key press, in the form that
    /// `parse_hotkey_string` in `src/ui/preferences_json.rs` reads back.
    /// Empty when the key can't be recorded: no modifier, a modifier alone,
    /// or a key that the config has no name for.
    static func hotkeyText(modifierFlags: NSEvent.ModifierFlags, keyCode: UInt16) -> String {
        var result = ""

        // Add modifiers in the order Rust shows them
        if modifierFlags.contains(.control) {
            result += "⌃"
        }
        if modifierFlags.contains(.option) {
            result += "⌥"
        }
        if modifierFlags.contains(.shift) {
            result += "⇧"
        }
        if modifierFlags.contains(.command) {
            result += "⌘"
        }

        // Need at least one modifier for a valid hotkey
        if result.isEmpty {
            return ""
        }

        guard let keyName = keyNames[keyCode] else {
            return ""
        }
        return result + keyName
    }

    /// The config's text for the key of a macOS key code: the names that
    /// `livesplit-hotkey` maps the key code to (`src/macos/mod.rs`), which is
    /// how `KeyCode` names it. The modifier keys and Escape are not
    /// recordable.
    private static let keyNames: [UInt16: String] = [
        // Writing system keys
        0x00: "A", 0x01: "S", 0x02: "D", 0x03: "F", 0x04: "H", 0x05: "G",
        0x06: "Z", 0x07: "X", 0x08: "C", 0x09: "V", 0x0A: "IntlBackslash",
        0x0B: "B", 0x0C: "Q", 0x0D: "W", 0x0E: "E", 0x0F: "R",
        0x10: "Y", 0x11: "T", 0x12: "1", 0x13: "2", 0x14: "3", 0x15: "4",
        0x16: "6", 0x17: "5", 0x18: "=", 0x19: "9", 0x1A: "7", 0x1B: "-",
        0x1C: "8", 0x1D: "0", 0x1E: "]", 0x1F: "O",
        0x20: "U", 0x21: "[", 0x22: "I", 0x23: "P",
        0x25: "L", 0x26: "J", 0x27: "'", 0x28: "K", 0x29: ";", 0x2A: "\\",
        0x2B: ",", 0x2C: "/", 0x2D: "N", 0x2E: "M", 0x2F: ".", 0x32: "`",
        // Functional keys
        0x24: "↩", 0x30: "⇥", 0x31: "Space", 0x33: "⌫", 0x39: "CapsLock",
        0x6E: "ContextMenu",
        // Control pad and arrow keys
        0x73: "Home", 0x74: "PageUp", 0x75: "Delete", 0x77: "End",
        0x79: "PageDown", 0x7B: "←", 0x7C: "→", 0x7D: "↓", 0x7E: "↑",
        // Numpad
        0x34: "NumpadEnter", 0x41: "NumpadDecimal", 0x43: "NumpadMultiply",
        0x45: "NumpadAdd", 0x47: "NumLock", 0x4B: "NumpadDivide",
        0x4C: "NumpadEnter", 0x4E: "NumpadSubtract", 0x51: "NumpadEqual",
        0x52: "Numpad0", 0x53: "Numpad1", 0x54: "Numpad2", 0x55: "Numpad3",
        0x56: "Numpad4", 0x57: "Numpad5", 0x58: "Numpad6", 0x59: "Numpad7",
        0x5B: "Numpad8", 0x5C: "Numpad9", 0x5F: "NumpadComma",
        // Function keys
        0x40: "F17", 0x4F: "F18", 0x50: "F19", 0x5A: "F20",
        0x60: "F5", 0x61: "F6", 0x62: "F7", 0x63: "F3", 0x64: "F8",
        0x65: "F9", 0x67: "F11", 0x69: "F13", 0x6A: "F16", 0x6B: "F14",
        0x6D: "F10", 0x6F: "F12", 0x71: "F15", 0x76: "F4", 0x78: "F2",
        0x7A: "F1",
        // Media and international keys
        0x48: "AudioVolumeUp", 0x49: "AudioVolumeDown", 0x4A: "AudioVolumeMute",
        0x5D: "IntlYen", 0x5E: "IntlRo", 0x66: "Lang2", 0x68: "Lang1",
        0x72: "Insert",
    ]
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
            Text(rule.appName ?? "")
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
