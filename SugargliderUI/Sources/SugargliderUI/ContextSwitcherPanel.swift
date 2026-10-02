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

  /// Gives a key-down event to `keyHandler`, and sends the standard edit
  /// shortcuts that it doesn't use to the text field. While the text field
  /// composes text, every key belongs to the input method, so ↩ commits the
  /// composition instead of acting on the list.
  func handleKeyDown(_ event: NSEvent) -> Bool {
    guard !isComposingText else { return false }
    if let keyHandler, let key = SwitcherKey(event: event), keyHandler(key) {
      return true
    }
    if let action = Self.editAction(
      keyCode: event.keyCode,
      modifiers: event.modifierFlags,
      characters: event.charactersIgnoringModifiers
    ) {
      return firstResponder?.tryToPerform(action, with: nil) ?? false
    }
    return false
  }

  /// The action of ⌘X, ⌘C, ⌘V, ⌘A, ⌘Z, or ⇧⌘Z. Sugarglider has no Edit menu
  /// whose items would send them.
  static func editAction(
    keyCode: UInt16,
    modifiers: NSEvent.ModifierFlags,
    characters: String?
  ) -> Selector? {
    let modifiers = modifiers.intersection([.command, .shift, .option, .control])
    switch (SwitcherKey.letter(keyCode: keyCode, characters: characters), modifiers) {
    case ("x", [.command]):
      return #selector(NSText.cut(_:))
    case ("c", [.command]):
      return #selector(NSText.copy(_:))
    case ("v", [.command]):
      return #selector(NSText.paste(_:))
    case ("a", [.command]):
      return #selector(NSText.selectAll(_:))
    case ("z", [.command]):
      return Selector(("undo:"))
    case ("z", [.command, .shift]):
      return Selector(("redo:"))
    default:
      return nil
    }
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

// MARK: - Presenter

/// Puts the switcher on screen and takes it off.
@MainActor
protocol ContextSwitcherPresenter: AnyObject {
  /// Called when the switcher loses key status.
  var onResignKey: () -> Void { get set }
  func present(_ model: ContextSwitcherModel, displayId: UInt32?)
  func dismiss()
}

/// Shows the switcher in its one panel, centered on a screen.
@MainActor
final class ContextSwitcherPanelPresenter: NSObject, ContextSwitcherPresenter, NSWindowDelegate {
  var onResignKey: () -> Void = {}
  private var panel: ContextSwitcherPanel?

  func present(_ model: ContextSwitcherModel, displayId: UInt32?) {
    let panel = self.panel ?? makePanel()
    panel.keyHandler = { [weak model] key in model?.handle(key) ?? false }
    panel.contentView = NSHostingView(rootView: ContextSwitcherView(model: model))
    let key = NSDeviceDescriptionKey("NSScreenNumber")
    let screens = NSScreen.screens.map { screen in
      (
        displayId: (screen.deviceDescription[key] as? NSNumber)?.uint32Value,
        visibleFrame: screen.visibleFrame
      )
    }
    if let visibleFrame = Self.visibleFrame(
      displayId: displayId, screens: screens, main: NSScreen.main?.visibleFrame)
    {
      panel.setFrame(
        ContextSwitcherPanel.frame(size: ContextSwitcherPanel.size, centeredIn: visibleFrame),
        display: false
      )
    }
    panel.makeKeyAndOrderFront(nil)
    panel.orderFrontRegardless()
  }

  func dismiss() {
    panel?.keyHandler = nil
    panel?.orderOut(nil)
  }

  func windowDidResignKey(_ notification: Notification) {
    onResignKey()
  }

  private func makePanel() -> ContextSwitcherPanel {
    let panel = ContextSwitcherPanel()
    panel.delegate = self
    self.panel = panel
    return panel
  }

  /// The visible frame of the screen with this display id, or of the main
  /// screen when no screen has it.
  static func visibleFrame(
    displayId: UInt32?,
    screens: [(displayId: UInt32?, visibleFrame: NSRect)],
    main: NSRect?
  ) -> NSRect? {
    if let displayId, let screen = screens.first(where: { $0.displayId == displayId }) {
      return screen.visibleFrame
    }
    return main ?? screens.first?.visibleFrame
  }
}

// MARK: - Controller

/// Opens and closes the switcher.
@MainActor
final class ContextSwitcherController {
  static let shared = ContextSwitcherController(
    presenter: ContextSwitcherPanelPresenter(),
    makeBackend: { RustContextSwitcherBackend() }
  )

  private let presenter: ContextSwitcherPresenter
  private let makeBackend: () -> ContextSwitcherBackend
  /// The open switcher's model, or nil when it is closed.
  private(set) var model: ContextSwitcherModel?

  init(presenter: ContextSwitcherPresenter, makeBackend: @escaping () -> ContextSwitcherBackend) {
    self.presenter = presenter
    self.makeBackend = makeBackend
    presenter.onResignKey = { [weak self] in self?.hide() }
  }

  /// Opens a fresh switcher for a show payload, or closes the switcher
  /// when it is open, so its hotkey toggles it. Only this side knows when
  /// the panel closed itself. A payload that doesn't decode opens nothing.
  func show(json: String) {
    if model != nil {
      hide()
      return
    }
    let payload: SwitcherPayload
    do {
      payload = try ContextSwitcherJSON.decode(SwitcherPayload.self, from: json)
    } catch {
      NSLog("Sugarglider: can't read the context switcher payload: %@", String(describing: error))
      return
    }
    let model = ContextSwitcherModel(payload: payload, backend: makeBackend())
    model.onClose = { [weak self, weak model] in
      guard let self, let model, self.model === model else { return }
      self.hide()
    }
    self.model = model
    presenter.present(model, displayId: payload.displayId)
  }

  func hide() {
    guard model != nil else { return }
    model = nil
    presenter.dismiss()
  }
}
