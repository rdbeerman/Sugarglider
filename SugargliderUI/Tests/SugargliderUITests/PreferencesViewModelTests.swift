// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import XCTest

@testable import SugargliderUI

/// Stands in for Rust, which the test process doesn't have.
@MainActor
final class FakePreferencesBackend: PreferencesBackend {
  var config: PreferencesConfig
  var updated: [PreferencesConfig] = []
  var saved: [PreferencesConfig] = []
  /// Thrown by the next saves while set.
  var saveError: Error?
  /// Thrown by the next loads while set.
  var loadError: Error?

  init(_ config: PreferencesConfig = PreferencesConfig()) {
    self.config = config
  }

  func loadConfig() throws -> PreferencesConfig {
    if let loadError { throw loadError }
    return config
  }

  func updateConfig(_ config: PreferencesConfig) throws {
    updated.append(config)
  }

  func saveConfigToFile(_ config: PreferencesConfig) throws {
    if let saveError { throw saveError }
    saved.append(config)
  }
}

@MainActor
final class PreferencesViewModelTests: XCTestCase {
  func testLoadsFromTheBackendAndSavesThroughIt() {
    let backend = FakePreferencesBackend(PreferencesConfig(animate: false, outerGap: 8))
    let model = PreferencesViewModel(backend: backend)
    XCTAssertFalse(model.enableAnimations)
    XCTAssertEqual(model.outerGap, 8)

    model.innerGap = 4
    model.saveToConfig()

    XCTAssertEqual(backend.updated.map(\.innerGap), [4])
    XCTAssertEqual(backend.saved.map(\.innerGap), [4])
    XCTAssertEqual(backend.saved.map(\.outerGap), [8])
    XCTAssertNil(model.lastError)
  }

  /// Two bindings of the same kind of command, with the same description,
  /// stay apart: a change to one leaves the other alone, and each keeps its
  /// command.
  func testChangesOnlyTheBindingItIsGiven() throws {
    let terminal = HotkeyBinding(
      key: "⌥Q", command: #"{"exec":"open -a Terminal"}"#, description: "Execute command",
      category: "Utilities")
    let safari = HotkeyBinding(
      key: "⌥W", command: #"{"exec":"open -a Safari"}"#, description: "Execute command",
      category: "Utilities")
    let backend = FakePreferencesBackend(PreferencesConfig(hotkeys: [terminal, safari]))
    let model = PreferencesViewModel(backend: backend)
    let second = try XCTUnwrap(model.hotkeys.last)

    model.updateHotkey(id: second.id, newKey: "⌥E")

    let saved = try XCTUnwrap(backend.saved.last)
    XCTAssertEqual(saved.hotkeys.map(\.key), ["⌥Q", "⌥E"])
    XCTAssertEqual(saved.hotkeys.map(\.command), [terminal.command, safari.command])
    XCTAssertEqual(backend.updated.last?.hotkeys.map(\.key), ["⌥Q", "⌥E"])
  }

  /// Resetting a binding resets only that binding.
  func testResetsOnlyTheBindingItIsGiven() throws {
    let bindings = [5, 10].map { percent in
      HotkeyBinding(
        key: "⌃⌥⇧\(percent == 5 ? "H" : "J")",
        command: #"{"resize":{"direction":"left","percent":\#(percent).0}}"#,
        description: "Resize left by \(percent)%", category: "Resize",
        defaultKey: percent == 5 ? "⌃⌥H" : nil)
    }
    let backend = FakePreferencesBackend(PreferencesConfig(hotkeys: bindings))
    let model = PreferencesViewModel(backend: backend)
    let first = try XCTUnwrap(model.hotkeys.first)

    model.resetHotkeyToDefault(id: first.id)

    let saved = try XCTUnwrap(backend.saved.last)
    XCTAssertEqual(saved.hotkeys.map(\.key), ["⌃⌥H", "⌃⌥⇧J"])
    XCTAssertEqual(saved.hotkeys.map(\.command), bindings.map(\.command))
  }

  /// Two rows that name one hotkey: the window refuses the key for the
  /// second row, shows why in the banner, keeps the old key, and saves
  /// nothing.
  func testRefusesAKeyAnotherRowAlreadyUses() throws {
    let toggle = HotkeyBinding(
      key: "⌥Z", command: #""toggle_global_enabled""#, description: "Toggle tiling globally",
      category: "System")
    let focus = HotkeyBinding(
      key: "⌥H", command: #"{"move_focus":"left"}"#, description: "Focus left",
      category: "Focus", defaultKey: "⌥H")
    let backend = FakePreferencesBackend(PreferencesConfig(hotkeys: [toggle, focus]))
    let model = PreferencesViewModel(backend: backend)
    let second = try XCTUnwrap(model.hotkeys.last)

    model.updateHotkey(id: second.id, newKey: "⌥Z")

    XCTAssertEqual(model.hotkeys.map(\.key), ["⌥Z", "⌥H"])
    XCTAssertEqual(
      model.lastError,
      #"⌥Z is already assigned to "Toggle tiling globally". The key was not changed."#)
    XCTAssertEqual(backend.updated.count, 0)
    XCTAssertEqual(backend.saved.count, 0)
  }

  /// The reset button refuses a default key that another row already uses.
  func testRefusesAResetToAKeyAnotherRowAlreadyUses() throws {
    let toggle = HotkeyBinding(
      key: "⌥Z", command: #""toggle_global_enabled""#, description: "Toggle tiling globally",
      category: "System")
    let focus = HotkeyBinding(
      key: "⌥H", command: #"{"move_focus":"left"}"#, description: "Focus left",
      category: "Focus", defaultKey: "⌥Z")
    let backend = FakePreferencesBackend(PreferencesConfig(hotkeys: [toggle, focus]))
    let model = PreferencesViewModel(backend: backend)
    let second = try XCTUnwrap(model.hotkeys.last)

    model.resetHotkeyToDefault(id: second.id)

    XCTAssertEqual(model.hotkeys.map(\.key), ["⌥Z", "⌥H"])
    XCTAssertEqual(backend.saved.count, 0)
    XCTAssertNotNil(model.lastError)
  }

  /// A failed initial load disables saving: the window shows the defaults
  /// and an empty key list, and neither may reach the running app or the
  /// file.
  func testDoesNotSaveWhenTheInitialLoadFailed() {
    let backend = FakePreferencesBackend(
      PreferencesConfig(animate: false, outerGap: 8, hotkeys: [
        HotkeyBinding(
          key: "⌥Z", command: #""toggle_global_enabled""#, description: "Toggle tiling globally",
          category: "System")
      ]))
    backend.loadError = ConfigBridgeError.loadFailed("No config available from backend")
    let model = PreferencesViewModel(backend: backend)
    XCTAssertEqual(
      model.lastError, "Failed to load config: No config available from backend")

    model.showMenuBarIcon = false
    model.saveToConfig()

    XCTAssertTrue(backend.updated.isEmpty)
    XCTAssertTrue(backend.saved.isEmpty)
  }

  /// A window rule condition that the App Rules pane doesn't show survives
  /// a save, in the running config and in the file.
  func testKeepsWindowRuleConditionsThePaneDoesNotShow() throws {
    let rule = WindowRuleJson(
      behavior: "float", titleRegex: "Picture-in-Picture", axSubrole: "AXDialog")
    let backend = FakePreferencesBackend(PreferencesConfig(windowRules: [rule]))
    let model = PreferencesViewModel(backend: backend)
    XCTAssertEqual(model.appRules.first?.titleRegex, "Picture-in-Picture")
    XCTAssertEqual(model.appRules.first?.axSubrole, "AXDialog")

    model.enableAnimations = false
    model.saveToConfig()

    let updated = try XCTUnwrap(backend.updated.last)
    XCTAssertEqual(updated.windowRules.first?.titleRegex, "Picture-in-Picture")
    XCTAssertEqual(updated.windowRules.first?.axSubrole, "AXDialog")
    let saved = try XCTUnwrap(backend.saved.last)
    XCTAssertEqual(saved.windowRules.first?.titleRegex, "Picture-in-Picture")
    XCTAssertEqual(saved.windowRules.first?.behavior, "float")
  }

  /// A window rule whose condition is an empty string keeps it: an empty
  /// string and no condition are different rules.
  func testKeepsEmptyWindowRuleConditions() throws {
    let rule = WindowRuleJson(appName: "", bundleId: "", behavior: "float")
    let backend = FakePreferencesBackend(PreferencesConfig(windowRules: [rule]))
    let model = PreferencesViewModel(backend: backend)
    let row = try XCTUnwrap(model.appRules.first)
    XCTAssertEqual(row.appName, "")
    XCTAssertEqual(row.bundleId, "")

    model.enableAnimations = false
    model.saveToConfig()

    let updated = try XCTUnwrap(backend.updated.last?.windowRules.first)
    XCTAssertEqual(updated.appName, "")
    XCTAssertEqual(updated.bundleId, "")
    let saved = try XCTUnwrap(backend.saved.last?.windowRules.first)
    XCTAssertEqual(saved.appName, "")
    XCTAssertEqual(saved.bundleId, "")
  }

  /// The Hotkeys pane shows the Contexts category, and any category it
  /// doesn't know after the others.
  func testGroupsEveryCategoryOfBindings() {
    let binding = { (key: String, category: String) in
      HotkeyBinding(key: key, command: #""debug""#, description: key, category: category)
    }
    let backend = FakePreferencesBackend(
      PreferencesConfig(hotkeys: [
        binding("⌃⌥0", "Contexts"), binding("⌥Z", "System"), binding("⌃⌥1", "Contexts"),
        binding("⌥X", "Zebra"), binding("⌥H", "Focus"), binding("⌥Y", "Apes"),
      ]))
    let model = PreferencesViewModel(backend: backend)

    let groups = model.hotkeysByCategory

    XCTAssertEqual(groups.map(\.category), ["System", "Focus", "Contexts", "Apes", "Zebra"])
    XCTAssertEqual(groups[2].bindings.map(\.key), ["⌃⌥0", "⌃⌥1"])
    XCTAssertEqual(groups.map(\.bindings.count).reduce(0, +), model.hotkeys.count)
  }

  /// Rust refuses to save over a config file with an error. The window's
  /// banner shows `lastError`: Rust's message once, without a second
  /// "Failed to save config", until a save succeeds. A failed save leaves
  /// the running app alone, so the two still match.
  func testShowsWhySavingFailedUntilASaveSucceeds() {
    let message = """
      Failed to save config: /tmp/glide.toml has an error, so it was not changed.

      error: could not parse config
       --> /tmp/glide.toml:2:11
        |
      2 | animate = tru
        |           ^^^ invalid boolean, expected `true`
      """
    let backend = FakePreferencesBackend()
    backend.saveError = ConfigBridgeError.saveFailed(message)
    let model = PreferencesViewModel(backend: backend)
    XCTAssertNil(model.lastError)

    model.saveToConfig()

    XCTAssertEqual(model.lastError, message)
    XCTAssertEqual(backend.saved.count, 0)
    // The running app was not changed either, so it still matches the file.
    XCTAssertEqual(backend.updated.count, 0)

    backend.saveError = nil
    model.saveToConfig()

    XCTAssertNil(model.lastError)
    XCTAssertEqual(backend.saved.count, 1)
    XCTAssertEqual(backend.updated.count, 1)
  }
}
