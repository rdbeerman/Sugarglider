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

  init(_ config: PreferencesConfig = PreferencesConfig()) {
    self.config = config
  }

  func loadConfig() throws -> PreferencesConfig {
    config
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

  /// Rust refuses to save over a config file with an error. The window's
  /// banner shows `lastError`: Rust's message once, without a second
  /// "Failed to save config", until a save succeeds.
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

    backend.saveError = nil
    model.saveToConfig()

    XCTAssertNil(model.lastError)
    XCTAssertEqual(backend.saved.count, 1)
  }
}
