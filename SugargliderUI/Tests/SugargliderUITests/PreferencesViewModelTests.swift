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
}
