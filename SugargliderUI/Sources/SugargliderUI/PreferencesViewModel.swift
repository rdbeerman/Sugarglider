// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import SwiftUI
import Combine
import ServiceManagement

// MARK: - Types

public enum LayoutMode: String, CaseIterable {
    case tree = "tree"
    case column = "column"
}

public enum SplitDirection: String, CaseIterable {
    case horizontal = "horizontal"
    case vertical = "vertical"
    case auto = "auto"
}

public enum AppBehavior: String, CaseIterable {
    case tile = "Tile"
    case float = "Float"
    case ignore = "Ignore"
}

public struct AppRule: Identifiable {
    public let id = UUID()
    public var appName: String
    public var bundleId: String
    public var behavior: AppBehavior

    public init(appName: String, bundleId: String, behavior: AppBehavior) {
        self.appName = appName
        self.bundleId = bundleId
        self.behavior = behavior
    }
}

// MARK: - View Model

@MainActor
public class PreferencesViewModel: ObservableObject {
    // General
    @Published public var launchAtLogin: Bool = false
    @Published public var showMenuBarIcon: Bool = true
    @Published public var enableAnimations: Bool = true
    @Published public var focusFollowsMouse: Bool = false
    @Published public var mouseFollowsFocus: Bool = false
    @Published public var outerGap: Double = 0
    @Published public var innerGap: Double = 0

    // Layout
    @Published public var defaultLayout: LayoutMode = .tree
    @Published public var defaultSplitDirection: SplitDirection = .auto
    @Published public var defaultColumnWidth: Double = 400

    // Hotkeys (read-only, loaded from config)
    @Published public var hotkeys: [HotkeyBinding] = []

    /// Hotkeys grouped by category for display
    public var hotkeysByCategory: [(category: String, bindings: [HotkeyBinding])] {
        let grouped = Dictionary(grouping: hotkeys) { $0.category }
        // Define category order
        let categoryOrder = ["System", "Focus", "Move", "Resize", "Layout", "Floating", "Scroll Layout", "Developer", "Utilities"]
        return categoryOrder.compactMap { category in
            guard let bindings = grouped[category], !bindings.isEmpty else { return nil }
            return (category: category, bindings: bindings)
        }
    }

    // App Rules
    @Published public var appRules: [AppRule] = []

    // Error state for UI feedback
    @Published public var lastError: String? = nil

    private var cancellables = Set<AnyCancellable>()
    private var isLoading = false

    public init() {
        loadFromConfig()
        setupAutoSave()
        loadLaunchAtLogin()
    }

    public func addAppRule() {
        appRules.append(AppRule(appName: "New App", bundleId: "", behavior: .tile))
    }

    /// Update a hotkey binding and save to config
    public func updateHotkey(commandId: String, newKey: String) {
        guard let index = hotkeys.firstIndex(where: { $0.commandId == commandId }) else {
            return
        }

        // Update the local state
        hotkeys[index].key = newKey

        // Save to config
        saveToConfig()
    }

    /// Reset a hotkey to its default value
    public func resetHotkeyToDefault(commandId: String) {
        guard let index = hotkeys.firstIndex(where: { $0.commandId == commandId }),
              let defaultKey = hotkeys[index].defaultKey else {
            return
        }

        // Update the local state to the default
        hotkeys[index].key = defaultKey

        // Save to config
        saveToConfig()
    }

    // MARK: - Config Loading

    private func loadFromConfig() {
        isLoading = true
        defer { isLoading = false }

        do {
            let config = try ConfigBridge.shared.loadConfig()
            applyConfig(config)
        } catch {
            print("Failed to load config: \(error)")
            // Use defaults - they're already set
        }
    }

    private func applyConfig(_ config: PreferencesConfig) {
        showMenuBarIcon = config.statusIconEnable
        enableAnimations = config.animate
        focusFollowsMouse = config.focusFollowsMouse
        mouseFollowsFocus = config.mouseFollowsFocus
        outerGap = config.outerGap
        innerGap = config.innerGap

        // Map layout kind: "scroll" -> .column, "tree" -> .tree
        defaultLayout = config.defaultLayoutKind == "scroll" ? .column : .tree

        // Load hotkeys
        hotkeys = config.hotkeys

        // Map window rules
        appRules = config.windowRules.map { rule in
            let behavior: AppBehavior
            switch rule.behavior.lowercased() {
            case "float": behavior = .float
            case "ignore": behavior = .ignore
            default: behavior = .tile
            }
            return AppRule(
                appName: rule.appName ?? "",
                bundleId: rule.bundleId ?? "",
                behavior: behavior
            )
        }
    }

    // MARK: - Auto-Save Setup

    private func setupAutoSave() {
        // Create publishers for all saveable properties
        let publishers: [AnyPublisher<Void, Never>] = [
            $showMenuBarIcon.map { _ in () }.eraseToAnyPublisher(),
            $enableAnimations.map { _ in () }.eraseToAnyPublisher(),
            $focusFollowsMouse.map { _ in () }.eraseToAnyPublisher(),
            $mouseFollowsFocus.map { _ in () }.eraseToAnyPublisher(),
            $outerGap.map { _ in () }.eraseToAnyPublisher(),
            $innerGap.map { _ in () }.eraseToAnyPublisher(),
            $defaultLayout.map { _ in () }.eraseToAnyPublisher(),
            $appRules.map { _ in () }.eraseToAnyPublisher(),
        ]

        Publishers.MergeMany(publishers)
            .dropFirst(publishers.count) // Skip initial values
            .debounce(for: .milliseconds(300), scheduler: RunLoop.main)
            .sink { [weak self] in
                guard let self = self, !self.isLoading else { return }
                self.saveToConfig()
            }
            .store(in: &cancellables)
    }

    // MARK: - Config Saving

    public func saveToConfig() {
        let config = buildConfig()

        do {
            // Update running window manager immediately
            try ConfigBridge.shared.updateConfig(config)
            // Persist to file
            try ConfigBridge.shared.saveConfigToFile(config)
            lastError = nil
        } catch {
            lastError = error.localizedDescription
            print("Failed to save config: \(error)")
        }
    }

    private func buildConfig() -> PreferencesConfig {
        PreferencesConfig(
            statusIconEnable: showMenuBarIcon,
            animate: enableAnimations,
            focusFollowsMouse: focusFollowsMouse,
            mouseFollowsFocus: mouseFollowsFocus,
            outerGap: outerGap,
            innerGap: innerGap,
            // Map layout mode: .column -> "scroll", .tree -> "tree"
            defaultLayoutKind: defaultLayout == .column ? "scroll" : "tree",
            windowRules: appRules.map { rule in
                WindowRuleJson(
                    appName: rule.appName.isEmpty ? nil : rule.appName,
                    bundleId: rule.bundleId.isEmpty ? nil : rule.bundleId,
                    behavior: rule.behavior.rawValue.lowercased()
                )
            },
            hotkeys: hotkeys
        )
    }

    // MARK: - Launch at Login

    private func loadLaunchAtLogin() {
        if #available(macOS 13.0, *) {
            launchAtLogin = SMAppService.mainApp.status == .enabled
        }
    }

    public func setLaunchAtLogin(_ enabled: Bool) {
        guard #available(macOS 13.0, *) else { return }

        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            launchAtLogin = enabled
        } catch {
            print("Failed to set launch at login: \(error)")
            // Revert UI state
            launchAtLogin = !enabled
        }
    }
}
