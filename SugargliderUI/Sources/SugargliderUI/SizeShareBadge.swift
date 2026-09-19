// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import AppKit
import SwiftUI

// MARK: - Size Share Badge Types

/// Style of a size share badge.
public enum SizeShareBadgeKind: Int, Sendable {
  case applied = 0
  case released = 1
  case rejected = 2
}

// MARK: - Size Share Badge View

/// A transient badge that confirms a size share command.
public struct SizeShareBadgeView: View {
  public let text: String
  public let kind: SizeShareBadgeKind
  /// Where the badge should be centered, in window-local coordinates.
  public let anchor: CGPoint

  public init(text: String, kind: SizeShareBadgeKind, anchor: CGPoint) {
    self.text = text
    self.kind = kind
    self.anchor = anchor
  }

  public var body: some View {
    Text(text)
      .font(.system(size: 15, weight: .semibold))
      .foregroundStyle(.white)
      .padding(.horizontal, 14)
      .padding(.vertical, 8)
      .background(background)
      .clipShape(Capsule())
      .shadow(color: .black.opacity(0.25), radius: 8, y: 2)
      .position(x: anchor.x, y: anchor.y)
  }

  private var background: Color {
    switch kind {
    case .applied:
      return .accentColor.opacity(0.9)
    case .released:
      return Color(nsColor: .darkGray).opacity(0.9)
    case .rejected:
      return .red.opacity(0.9)
    }
  }
}

// MARK: - Size Share Badge Window

/// Borderless overlay window that hosts the badge.
public class SizeShareBadgeOverlayWindow: NSWindow {
  public init(screen: NSScreen) {
    // Use visibleFrame to match Rust's coordinate system (excludes menu bar/dock)
    super.init(
      contentRect: screen.visibleFrame,
      styleMask: .borderless,
      backing: .buffered,
      defer: false
    )

    self.level = .floating
    self.backgroundColor = .clear
    self.isOpaque = false
    self.hasShadow = false
    self.ignoresMouseEvents = true
    self.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
  }

  fileprivate func present(text: String, kind: SizeShareBadgeKind, anchor: CGPoint) {
    self.contentView = NSHostingView(
      rootView: SizeShareBadgeView(text: text, kind: kind, anchor: anchor)
    )
    self.orderFront(nil)
  }
}

// MARK: - Size Share Badge Manager

/// Manages the size share badge overlay across all screens.
@MainActor
public class SizeShareBadgeManager {
  private var overlay: SizeShareBadgeOverlayWindow?
  private var hideWorkItem: DispatchWorkItem?

  public static let shared = SizeShareBadgeManager()

  private init() {}

  /// Shows the badge near the given window frame (screen-local coordinates)
  /// on the given screen.
  public func show(text: String, kind: SizeShareBadgeKind, frame: CGRect, screenIndex: Int) {
    let screens = NSScreen.screens
    guard screenIndex >= 0, screenIndex < screens.count else { return }
    let screen = screens[screenIndex]

    let overlay =
      self.overlay
      ?? {
        let window = SizeShareBadgeOverlayWindow(screen: screen)
        self.overlay = window
        return window
      }()

    overlay.setFrame(screen.visibleFrame, display: false)

    // Center the badge near the top of the window, clamped to the screen.
    let margin: CGFloat = 80
    let x = min(max(frame.midX, margin), screen.visibleFrame.width - margin)
    let y = max(frame.minY + 36, 36)
    overlay.present(text: text, kind: kind, anchor: CGPoint(x: x, y: y))

    hideWorkItem?.cancel()
    let work = DispatchWorkItem { [weak self] in
      self?.overlay?.orderOut(nil)
    }
    hideWorkItem = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 1.2, execute: work)
  }

  /// Hides the badge immediately.
  public func hide() {
    hideWorkItem?.cancel()
    hideWorkItem = nil
    overlay?.orderOut(nil)
  }
}

// MARK: - C API for Rust Integration

/// Shows a transient size share badge (callable from Rust via FFI).
/// The frame is the target window's frame in screen-local coordinates.
@_cdecl("sugarglider_show_size_share_badge")
public func showSizeShareBadge(
  textPtr: UnsafePointer<CChar>,
  kind: Int,
  x: Float,
  y: Float,
  width: Float,
  height: Float,
  screenIndex: Int
) {
  let text = String(cString: textPtr)
  let kind = SizeShareBadgeKind(rawValue: kind) ?? .applied
  let frame = CGRect(x: CGFloat(x), y: CGFloat(y), width: CGFloat(width), height: CGFloat(height))

  DispatchQueue.main.async {
    SizeShareBadgeManager.shared.show(
      text: text,
      kind: kind,
      frame: frame,
      screenIndex: screenIndex
    )
  }
}

// MARK: - Preview

#Preview {
  SizeShareBadgeView(
    text: "1/2",
    kind: .applied,
    anchor: CGPoint(x: 100, y: 40)
  )
  .frame(width: 200, height: 100)
}
