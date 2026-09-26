// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import XCTest

@testable import SugargliderUI

/// Builds show payloads and rank results for the tests that check the
/// switcher against `docs/specs/contexts.md` ("Switcher", "Swift bridge").
enum SpecPayload {
  static func context(
    _ id: UInt32,
    _ name: String,
    number: Int? = nil,
    hotkey: String? = nil,
    active: Bool = false,
    apps: [String] = [],
    members: [SwitcherMember] = []
  ) -> SwitcherContext {
    SwitcherContext(
      id: id,
      name: name,
      number: number,
      hotkey: hotkey,
      active: active,
      apps: apps,
      windows: members.filter { $0.window != nil }.count,
      members: members
    )
  }

  static func window(
    _ id: SwitcherWindowId,
    _ title: String,
    _ app: String,
    pinned: Bool = false
  ) -> SwitcherWindow {
    SwitcherWindow(id: id, title: title, app: app, pinned: pinned)
  }

  /// A payload for the first time: no named context, Everything active.
  static func payload(
    target: SwitcherWindowId? = nil,
    contexts: [SwitcherContext] = [],
    unsorted: SwitcherUnsorted = SwitcherUnsorted(windows: 0, active: false),
    everything: SwitcherEverything = SwitcherEverything(active: true, hotkey: "⌃⌥0"),
    windows: [SwitcherWindow] = []
  ) -> SwitcherPayload {
    SwitcherPayload(
      displayId: nil,
      targetWindow: target,
      contexts: contexts,
      unsorted: unsorted,
      everything: everything,
      windows: windows
    )
  }

  static func ranked(_ entries: [(SwitcherContextKey, SwitcherNameMatch)]) -> [SwitcherRankedEntry]
  {
    entries.map { SwitcherRankedEntry(key: $0.0, match: $0.1) }
  }

  static func named(_ id: UInt32) -> SwitcherContextKey {
    .named(SwitcherContextId(id: id))
  }
}

/// A view model over a `FakeBackend`, counting how often it asks to close.
@MainActor
class SwitcherSpecTestCase: XCTestCase {
  var backend = FakeBackend()
  var closed = 0

  override func setUp() async throws {
    try await super.setUp()
    backend = FakeBackend()
    closed = 0
  }

  func makeModel(_ payload: SwitcherPayload? = nil) throws -> ContextSwitcherModel {
    let model = ContextSwitcherModel(payload: try payload ?? Fixtures.payload(), backend: backend)
    model.onClose = { [weak self] in self?.closed += 1 }
    return model
  }

  /// The rows as the user reads them.
  func names(_ model: ContextSwitcherModel) -> [String] {
    model.rows.map { row in
      switch row {
      case .entry(let entry): return entry.name
      case .newContext(let name): return "New context “\(name)”"
      }
    }
  }

  func entries(_ model: ContextSwitcherModel) -> [SwitcherEntry] {
    model.rows.compactMap { row in
      if case .entry(let entry) = row { return entry }
      return nil
    }
  }

  /// Moves the highlight to a row with the arrow keys.
  func highlight(
    _ model: ContextSwitcherModel,
    row: Int,
    file: StaticString = #filePath,
    line: UInt = #line
  ) {
    for _ in 0..<model.rows.count where model.highlight != row {
      model.handle(model.highlight > row ? .up : .down)
    }
    XCTAssertEqual(model.highlight, row, file: file, line: line)
  }
}
