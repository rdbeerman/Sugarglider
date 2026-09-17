// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import Foundation

// MARK: - Config Data Types

/// Preferences configuration matching the Rust PreferencesJson struct.
/// Field names use camelCase to match Swift conventions (Rust uses snake_case in JSON).
public struct PreferencesConfig: Codable {
    // General settings
    public var statusIconEnable: Bool
    public var animate: Bool
    public var focusFollowsMouse: Bool
    public var mouseFollowsFocus: Bool
    public var outerGap: Double
    public var innerGap: Double

    // Dragging behavior
    public var dragDropEnable: Bool
    public var dragDropWindowDrag: Bool

    // Layout settings
    public var defaultLayoutKind: String

    // Window rules
    public var windowRules: [WindowRuleJson]

    // Hotkey bindings (read-only)
    public var hotkeys: [HotkeyBinding]

    public init(
        statusIconEnable: Bool = true,
        animate: Bool = true,
        focusFollowsMouse: Bool = false,
        mouseFollowsFocus: Bool = false,
        outerGap: Double = 0,
        innerGap: Double = 0,
        dragDropEnable: Bool = true,
        dragDropWindowDrag: Bool = true,
        defaultLayoutKind: String = "tree",
        windowRules: [WindowRuleJson] = [],
        hotkeys: [HotkeyBinding] = []
    ) {
        self.statusIconEnable = statusIconEnable
        self.animate = animate
        self.focusFollowsMouse = focusFollowsMouse
        self.mouseFollowsFocus = mouseFollowsFocus
        self.outerGap = outerGap
        self.innerGap = innerGap
        self.dragDropEnable = dragDropEnable
        self.dragDropWindowDrag = dragDropWindowDrag
        self.defaultLayoutKind = defaultLayoutKind
        self.windowRules = windowRules
        self.hotkeys = hotkeys
    }
}

/// Hotkey binding from the configuration.
public struct HotkeyBinding: Codable, Identifiable, Equatable {
    public var id: String { "\(key)-\(commandId)" }

    /// The formatted hotkey string (e.g., "⌥H")
    public var key: String

    /// The command identifier for internal use
    public var commandId: String

    /// Human-readable description of what the command does
    public var description: String

    /// Category for grouping in the UI
    public var category: String

    /// The default hotkey for this command (if any)
    public var defaultKey: String?

    /// Whether this hotkey differs from its default
    public var isModified: Bool {
        guard let defaultKey = defaultKey else { return false }
        return key != defaultKey
    }

    public init(key: String, commandId: String, description: String, category: String, defaultKey: String? = nil) {
        self.key = key
        self.commandId = commandId
        self.description = description
        self.category = category
        self.defaultKey = defaultKey
    }
}

/// Window rule matching the Rust WindowRuleJson struct.
public struct WindowRuleJson: Codable, Identifiable {
    public var id = UUID()
    public var appName: String?
    public var bundleId: String?
    public var behavior: String

    public init(appName: String? = nil, bundleId: String? = nil, behavior: String = "tile") {
        self.appName = appName
        self.bundleId = bundleId
        self.behavior = behavior
    }

    enum CodingKeys: String, CodingKey {
        case appName
        case bundleId
        case behavior
    }
}

// MARK: - Config Bridge

/// Error types for config operations.
public enum ConfigBridgeError: LocalizedError {
    case loadFailed(String)
    case updateFailed(String)
    case saveFailed(String)
    case encodingFailed(String)
    case decodingFailed(String)

    public var errorDescription: String? {
        switch self {
        case .loadFailed(let msg): return "Failed to load config: \(msg)"
        case .updateFailed(let msg): return "Failed to update config: \(msg)"
        case .saveFailed(let msg): return "Failed to save config: \(msg)"
        case .encodingFailed(let msg): return "Failed to encode config: \(msg)"
        case .decodingFailed(let msg): return "Failed to decode config: \(msg)"
        }
    }
}

/// Bridge for communicating config changes with the Rust backend.
@MainActor
public final class ConfigBridge {
    public static let shared = ConfigBridge()

    private init() {}

    /// Load the current configuration from the Rust backend.
    public func loadConfig() throws -> PreferencesConfig {
        guard let ptr = sugarglider_get_config() else {
            throw ConfigBridgeError.loadFailed("No config available from backend")
        }
        defer { sugarglider_free_string(ptr) }

        let jsonString = String(cString: ptr)

        guard let data = jsonString.data(using: .utf8) else {
            throw ConfigBridgeError.decodingFailed("Invalid UTF-8 in config JSON")
        }

        do {
            let decoder = JSONDecoder()
            return try decoder.decode(PreferencesConfig.self, from: data)
        } catch {
            throw ConfigBridgeError.decodingFailed(error.localizedDescription)
        }
    }

    /// Update the running window manager with new configuration.
    /// This takes effect immediately but does not persist to disk.
    public func updateConfig(_ config: PreferencesConfig) throws {
        let encoder = JSONEncoder()
        let data: Data
        do {
            data = try encoder.encode(config)
        } catch {
            throw ConfigBridgeError.encodingFailed(error.localizedDescription)
        }

        guard let jsonString = String(data: data, encoding: .utf8) else {
            throw ConfigBridgeError.encodingFailed("Failed to create UTF-8 string from JSON")
        }

        let errorPtr = jsonString.withCString { cStr in
            sugarglider_update_config(cStr)
        }

        if let errorPtr = errorPtr {
            defer { sugarglider_free_string(errorPtr) }
            let errorMsg = String(cString: errorPtr)
            throw ConfigBridgeError.updateFailed(errorMsg)
        }
    }

    /// Save configuration to the TOML config file.
    /// This persists the settings to disk.
    public func saveConfigToFile(_ config: PreferencesConfig) throws {
        let encoder = JSONEncoder()
        let data: Data
        do {
            data = try encoder.encode(config)
        } catch {
            throw ConfigBridgeError.encodingFailed(error.localizedDescription)
        }

        guard let jsonString = String(data: data, encoding: .utf8) else {
            throw ConfigBridgeError.encodingFailed("Failed to create UTF-8 string from JSON")
        }

        let errorPtr = jsonString.withCString { cStr in
            sugarglider_save_config_to_file(cStr)
        }

        if let errorPtr = errorPtr {
            defer { sugarglider_free_string(errorPtr) }
            let errorMsg = String(cString: errorPtr)
            throw ConfigBridgeError.saveFailed(errorMsg)
        }
    }

    /// Update config and save to file in one operation.
    public func updateAndSave(_ config: PreferencesConfig) throws {
        try updateConfig(config)
        try saveConfigToFile(config)
    }
}
