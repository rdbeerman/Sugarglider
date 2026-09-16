// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import AppKit
import SwiftUI

/// Main entry point for the SugargliderUI library
@MainActor
public struct SugargliderUI {
  /// The shared preferences window controller
  private static var preferencesWindowController: PreferencesWindowController?

  /// Shows the preferences window
  public static func showPreferences() {
    if preferencesWindowController == nil {
      preferencesWindowController = PreferencesWindowController()
    }
    preferencesWindowController?.showWindow(nil)
    NSApp.activate(ignoringOtherApps: true)
  }

  /// Hides the preferences window
  public static func hidePreferences() {
    preferencesWindowController?.close()
  }
}

/// Window controller for the preferences window
class PreferencesWindowController: NSWindowController {
  convenience init() {
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 650, height: 500),
      styleMask: [.titled, .closable],
      backing: .buffered,
      defer: false
    )
    window.title = "Sugarglider Settings"
    window.center()
    window.contentView = NSHostingView(rootView: PreferencesView())
    // Keep the preferences window above tiled windows so the window manager
    // doesn't obscure it when raising managed windows.
    window.level = .floating

    self.init(window: window)
  }

  override func showWindow(_ sender: Any?) {
    super.showWindow(sender)
    // Ensure the window becomes key and is ordered to the front.
    window?.makeKeyAndOrderFront(nil)
  }
}

// MARK: - C API for Rust Integration

/// Opens the preferences window (callable from Rust via FFI)
@_cdecl("sugarglider_show_preferences")
public func showPreferences() {
  DispatchQueue.main.async {
    SugargliderUI.showPreferences()
  }
}

/// Hides the preferences window (callable from Rust via FFI)
@_cdecl("sugarglider_hide_preferences")
public func hidePreferences() {
  DispatchQueue.main.async {
    SugargliderUI.hidePreferences()
  }
}

// MARK: - FFI Declarations for Config Bridge (Swift calls Rust)

/// Get current config as JSON string. Returns null if unavailable.
/// Caller must free the returned string with sugarglider_free_string.
@_silgen_name("sugarglider_get_config")
func sugarglider_get_config() -> UnsafeMutablePointer<CChar>?

/// Update config from JSON string. Returns null on success, error message on failure.
/// Caller must free any returned error string with sugarglider_free_string.
@_silgen_name("sugarglider_update_config")
func sugarglider_update_config(_ json: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?

/// Save config to TOML file. Returns null on success, error message on failure.
/// Caller must free any returned error string with sugarglider_free_string.
@_silgen_name("sugarglider_save_config_to_file")
func sugarglider_save_config_to_file(_ json: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?

/// Free a string returned by other FFI functions.
@_silgen_name("sugarglider_free_string")
func sugarglider_free_string(_ ptr: UnsafeMutablePointer<CChar>?)
