// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import Darwin
import Foundation

// MARK: - Swift to Rust

/// A Rust function that takes a string and may return one, which the caller
/// frees with `sugarglider_free_string`.
typealias RustStringFunction =
  @convention(c) (UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?

/// Calls the switcher's Rust functions. It looks them up when it calls
/// them, so a binary that lacks them still loads, and the call fails with a
/// message instead.
@MainActor
final class RustContextSwitcherBackend: ContextSwitcherBackend {
  static let rankSymbol = "sugarglider_rank_contexts"
  static let runSymbol = "sugarglider_run_context_command"

  func rank(_ query: String) throws -> [SwitcherRankedEntry] {
    guard let json = try call(Self.rankSymbol, with: query) else {
      throw SwitcherBridgeError(message: "Contexts are unavailable.")
    }
    do {
      return try ContextSwitcherJSON.decode([SwitcherRankedEntry].self, from: json)
    } catch {
      throw SwitcherBridgeError(message: "The ranked contexts can't be read: \(error)")
    }
  }

  func run(_ command: SwitcherCommand) throws {
    let json: String
    do {
      json = try ContextSwitcherJSON.encode(command)
    } catch {
      throw SwitcherBridgeError(message: "The command can't be encoded: \(error)")
    }
    if let message = try call(Self.runSymbol, with: json) {
      throw SwitcherBridgeError(message: message)
    }
  }

  /// Calls the function named `symbol` and returns the string it returned,
  /// after freeing it.
  private func call(_ symbol: String, with argument: String) throws -> String? {
    guard let function = Self.lookUp(symbol) else {
      throw SwitcherBridgeError(message: "\(symbol) is missing from this Sugarglider build.")
    }
    guard let result = argument.withCString({ function($0) }) else { return nil }
    defer { sugarglider_free_string(result) }
    return String(cString: result)
  }

  /// `RTLD_DEFAULT`, which Swift doesn't import: search every loaded image.
  private static let everyImage = UnsafeMutableRawPointer(bitPattern: -2)

  static func lookUp(_ symbol: String) -> RustStringFunction? {
    guard let address = dlsym(everyImage, symbol) else { return nil }
    return unsafeBitCast(address, to: RustStringFunction.self)
  }
}

// MARK: - Rust to Swift

/// Shows the context switcher (callable from Rust via FFI). `json` is the
/// show payload described on `ContextSwitcherJSON`.
@_cdecl("sugarglider_show_context_switcher")
public func showContextSwitcher(json: UnsafePointer<CChar>) {
  let json = String(cString: json)
  DispatchQueue.main.async {
    ContextSwitcherController.shared.show(json: json)
  }
}

/// Hides the context switcher (callable from Rust via FFI).
@_cdecl("sugarglider_hide_context_switcher")
public func hideContextSwitcher() {
  DispatchQueue.main.async {
    ContextSwitcherController.shared.hide()
  }
}
