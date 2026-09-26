// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import AppKit
import SwiftUI

// MARK: - Panel

/// The switcher's window. It takes key focus without making Sugarglider the
/// active app, so the user can type while the focused window keeps its app
/// in front.
final class ContextSwitcherPanel: NSPanel {
  static let size = NSSize(width: 600, height: 460)

  /// Acts on a key before the text field sees it. Returns true when it used
  /// the key.
  var keyHandler: ((SwitcherKey) -> Bool)?

  init() {
    super.init(
      contentRect: NSRect(origin: .zero, size: Self.size),
      styleMask: [.nonactivatingPanel, .borderless],
      backing: .buffered,
      defer: true
    )
    level = .floating
    collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
    isFloatingPanel = true
    hidesOnDeactivate = false
    becomesKeyOnlyIfNeeded = false
    isReleasedWhenClosed = false
    backgroundColor = .clear
    isOpaque = false
    hasShadow = true
  }

  override var canBecomeKey: Bool { true }

  override var canBecomeMain: Bool { false }

  // Command keys can reach `performKeyEquivalent` before `sendEvent`, so
  // both give the switcher the first look at a key.
  override func performKeyEquivalent(with event: NSEvent) -> Bool {
    handleKey(event) || super.performKeyEquivalent(with: event)
  }

  override func sendEvent(_ event: NSEvent) {
    if handleKey(event) { return }
    super.sendEvent(event)
  }

  private func handleKey(_ event: NSEvent) -> Bool {
    guard event.type == .keyDown, isKeyWindow else { return false }
    return handleKeyDown(event)
  }

  /// Gives a key-down event to `keyHandler`. While the text field composes
  /// text, every key belongs to the input method, so ↩ commits the
  /// composition instead of acting on the list.
  func handleKeyDown(_ event: NSEvent) -> Bool {
    guard !isComposingText, let keyHandler, let key = SwitcherKey(event: event) else {
      return false
    }
    return keyHandler(key)
  }

  /// Whether the focused text field holds marked text from an input method.
  var isComposingText: Bool {
    (firstResponder as? NSTextInputClient)?.hasMarkedText() == true
  }

  /// A frame of `size` centered in `visibleFrame`.
  static func frame(size: NSSize, centeredIn visibleFrame: NSRect) -> NSRect {
    NSRect(
      x: (visibleFrame.midX - size.width / 2).rounded(),
      y: (visibleFrame.midY - size.height / 2).rounded(),
      width: size.width,
      height: size.height
    )
  }
}

extension SwitcherKey {
  init?(event: NSEvent) {
    self.init(
      keyCode: event.keyCode,
      modifiers: event.modifierFlags,
      characters: event.charactersIgnoringModifiers
    )
  }
}

// MARK: - Controller

/// Shows and hides the one switcher panel.
@MainActor
final class ContextSwitcherController: NSObject, NSWindowDelegate {
  static let shared = ContextSwitcherController()

  private var panel: ContextSwitcherPanel?
  private var model: ContextSwitcherModel?

  /// Shows a fresh switcher for a show payload, replacing one that is open.
  func show(json: String) {
    let payload: SwitcherPayload
    do {
      payload = try ContextSwitcherJSON.decode(SwitcherPayload.self, from: json)
    } catch {
      NSLog("Sugarglider: can't read the context switcher payload: %@", String(describing: error))
      return
    }
    hide()

    let model = ContextSwitcherModel(payload: payload, backend: RustContextSwitcherBackend())
    model.onClose = { [weak self] in self?.hide() }
    self.model = model

    let panel = self.panel ?? makePanel()
    panel.keyHandler = { [weak model] key in model?.handle(key) ?? false }
    panel.contentView = NSHostingView(rootView: ContextSwitcherView(model: model))
    if let screen = Self.screen(displayId: payload.displayId) {
      panel.setFrame(
        ContextSwitcherPanel.frame(
          size: ContextSwitcherPanel.size, centeredIn: screen.visibleFrame),
        display: false
      )
    }
    panel.makeKeyAndOrderFront(nil)
    panel.orderFrontRegardless()
  }

  func hide() {
    guard model != nil else { return }
    model = nil
    panel?.keyHandler = nil
    panel?.orderOut(nil)
  }

  func windowDidResignKey(_ notification: Notification) {
    hide()
  }

  private func makePanel() -> ContextSwitcherPanel {
    let panel = ContextSwitcherPanel()
    panel.delegate = self
    self.panel = panel
    return panel
  }

  /// The screen with this display id, or the main screen.
  private static func screen(displayId: UInt32?) -> NSScreen? {
    let key = NSDeviceDescriptionKey("NSScreenNumber")
    if let displayId,
      let screen = NSScreen.screens.first(where: {
        ($0.deviceDescription[key] as? NSNumber)?.uint32Value == displayId
      })
    {
      return screen
    }
    return NSScreen.main ?? NSScreen.screens.first
  }
}
