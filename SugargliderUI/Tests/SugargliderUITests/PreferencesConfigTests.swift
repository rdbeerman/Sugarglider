// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import Foundation
import XCTest

@testable import SugargliderUI

/// The JSON that `ConfigBridge` sends to Rust. The Rust test
/// `preferences_from_the_swift_ui_save_with_their_key_bindings` in
/// `src/config.rs` saves the same fixture.
final class PreferencesConfigTests: XCTestCase {
  private static let fixtureURL = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()  // SugargliderUITests
    .deletingLastPathComponent()  // Tests
    .deletingLastPathComponent()  // SugargliderUI
    .deletingLastPathComponent()  // repository root
    .appendingPathComponent("tests/fixtures/preferences-from-swift.json")

  private static let fixtureConfig = PreferencesConfig(
    statusIconEnable: true,
    animate: false,
    focusFollowsMouse: true,
    mouseFollowsFocus: false,
    outerGap: 8,
    innerGap: 4,
    dragDropEnable: true,
    dragDropLivePreview: false,
    defaultLayoutKind: "tree",
    windowRules: [
      WindowRuleJson(appName: "Finder", bundleId: "com.apple.finder", behavior: "float"),
      WindowRuleJson(appName: "Calculator", behavior: "float"),
    ],
    hotkeys: [
      HotkeyBinding(
        key: "⌥Z", commandId: "toggle_global_enabled", description: "Toggle tiling globally",
        category: "System", defaultKey: "⌥Z"),
      HotkeyBinding(
        key: "⌃⌥⇧H", commandId: "move_focus_left", description: "Focus left",
        category: "Focus", defaultKey: "⌃⌥⇧←"),
      HotkeyBinding(
        key: "⌥T", commandId: "exec", description: "Execute command", category: "Utilities"),
    ]
  )

  private static func jsonObject(_ data: Data) throws -> NSDictionary {
    let object = try JSONSerialization.jsonObject(with: data)
    return try XCTUnwrap(object as? NSDictionary)
  }

  func testEncodesTheFixtureThatRustSaves() throws {
    let encoded = try Self.jsonObject(JSONEncoder().encode(Self.fixtureConfig))
    let fixture = try Self.jsonObject(Data(contentsOf: Self.fixtureURL))

    XCTAssertEqual(fixture, encoded)
  }
}
