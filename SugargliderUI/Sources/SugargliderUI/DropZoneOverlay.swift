// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import SwiftUI
import AppKit

// MARK: - Drop Zone Types

/// Represents a position where a window can be dropped
public enum DropPosition: Equatable {
    case left(of: WindowId)
    case right(of: WindowId)
    case top(of: WindowId)
    case bottom(of: WindowId)
    case tab(with: WindowId)
    case newColumn
}

/// Identifier for a window in the layout
public typealias WindowId = UInt64

/// Represents a visible drop zone on screen
public struct DropZone: Identifiable {
    public let id = UUID()
    public let frame: CGRect
    public let position: DropPosition
    public let isHighlighted: Bool

    public init(frame: CGRect, position: DropPosition, isHighlighted: Bool = false) {
        self.frame = frame
        self.position = position
        self.isHighlighted = isHighlighted
    }
}

// MARK: - Drop Zone Overlay View

/// Transparent overlay showing drop zones during window drag
public struct DropZoneOverlayView: View {
    public let dropZones: [DropZone]
    public let activeZone: UUID?

    public init(dropZones: [DropZone], activeZone: UUID? = nil) {
        self.dropZones = dropZones
        self.activeZone = activeZone
    }

    public var body: some View {
        ZStack {
            // Semi-transparent background
            Color.black.opacity(0.1)
                .ignoresSafeArea()

            // Drop zone indicators
            ForEach(dropZones) { zone in
                DropZoneIndicator(
                    zone: zone,
                    isActive: zone.id == activeZone
                )
            }
        }
    }
}

/// Individual drop zone indicator
struct DropZoneIndicator: View {
    let zone: DropZone
    let isActive: Bool

    var body: some View {
        RoundedRectangle(cornerRadius: 8)
            .fill(isActive ? Color.accentColor.opacity(0.4) : Color.accentColor.opacity(0.2))
            .overlay(
                RoundedRectangle(cornerRadius: 8)
                    .strokeBorder(
                        isActive ? Color.accentColor : Color.accentColor.opacity(0.5),
                        lineWidth: isActive ? 3 : 2
                    )
            )
            .overlay(
                dropIcon
                    .font(.system(size: isActive ? 32 : 24))
                    .foregroundColor(isActive ? .accentColor : .secondary)
            )
            .frame(width: zone.frame.width, height: zone.frame.height)
            .position(x: zone.frame.midX, y: zone.frame.midY)
            .animation(.easeInOut(duration: 0.15), value: isActive)
    }

    @ViewBuilder
    var dropIcon: some View {
        switch zone.position {
        case .left:
            Image(systemName: "rectangle.leadinghalf.inset.filled.arrow.leading")
        case .right:
            Image(systemName: "rectangle.trailinghalf.inset.filled.arrow.trailing")
        case .top:
            Image(systemName: "rectangle.tophalf.inset.filled.arrow.up")
        case .bottom:
            Image(systemName: "rectangle.bottomhalf.inset.filled.arrow.down")
        case .tab:
            Image(systemName: "rectangle.stack")
        case .newColumn:
            Image(systemName: "plus.rectangle")
        }
    }
}

// MARK: - Drop Zone Window Controller

/// NSWindow subclass for the transparent overlay
public class DropZoneOverlayWindow: NSWindow {
    public init(screen: NSScreen) {
        super.init(
            contentRect: screen.frame,
            styleMask: .borderless,
            backing: .buffered,
            defer: false
        )

        self.level = .floating
        self.backgroundColor = .clear
        self.isOpaque = false
        self.hasShadow = false
        self.ignoresMouseEvents = false
        self.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
    }

    public func updateDropZones(_ zones: [DropZone], activeZone: UUID?) {
        self.contentView = NSHostingView(
            rootView: DropZoneOverlayView(dropZones: zones, activeZone: activeZone)
        )
    }
}

// MARK: - Drop Zone Manager

/// Manages the drop zone overlay across all screens
@MainActor
public class DropZoneManager {
    private var overlayWindows: [NSScreen: DropZoneOverlayWindow] = [:]
    private var dropZones: [DropZone] = []
    private var activeZoneId: UUID?

    public static let shared = DropZoneManager()

    private init() {}

    /// Shows drop zones on all screens
    public func showDropZones(_ zones: [DropZone]) {
        dropZones = zones

        for screen in NSScreen.screens {
            let window = overlayWindows[screen] ?? {
                let w = DropZoneOverlayWindow(screen: screen)
                overlayWindows[screen] = w
                return w
            }()

            let screenZones = zones.filter { screen.frame.contains($0.frame) }
            window.updateDropZones(screenZones, activeZone: activeZoneId)
            window.orderFront(nil)
        }
    }

    /// Updates which zone is currently hovered
    public func setActiveZone(_ zoneId: UUID?) {
        activeZoneId = zoneId
        for (screen, window) in overlayWindows {
            let screenZones = dropZones.filter { screen.frame.contains($0.frame) }
            window.updateDropZones(screenZones, activeZone: activeZoneId)
        }
    }

    /// Hides all drop zone overlays
    public func hideDropZones() {
        for window in overlayWindows.values {
            window.orderOut(nil)
        }
        dropZones = []
        activeZoneId = nil
    }

    /// Returns the drop zone at the given point, if any
    public func zoneAt(point: CGPoint) -> DropZone? {
        dropZones.first { $0.frame.contains(point) }
    }
}

// MARK: - C API for Rust Integration

/// Shows drop zones (callable from Rust via FFI)
/// zones_ptr: pointer to array of (x, y, width, height, position_type, target_window_id)
@_cdecl("sugarglider_show_drop_zones")
public func showDropZones(zonesPtr: UnsafePointer<Float>, count: Int) {
    // TODO: Parse zones from C array and call DropZoneManager
    DispatchQueue.main.async {
        // Placeholder - actual implementation will parse the C data
        DropZoneManager.shared.showDropZones([])
    }
}

/// Hides drop zones (callable from Rust via FFI)
@_cdecl("sugarglider_hide_drop_zones")
public func hideDropZones() {
    DispatchQueue.main.async {
        DropZoneManager.shared.hideDropZones()
    }
}

// MARK: - Preview

#Preview {
    DropZoneOverlayView(
        dropZones: [
            DropZone(frame: CGRect(x: 50, y: 50, width: 200, height: 200), position: .left(of: 1)),
            DropZone(frame: CGRect(x: 300, y: 50, width: 200, height: 200), position: .right(of: 1)),
            DropZone(frame: CGRect(x: 175, y: 300, width: 200, height: 100), position: .bottom(of: 1)),
        ],
        activeZone: nil
    )
    .frame(width: 600, height: 500)
}
