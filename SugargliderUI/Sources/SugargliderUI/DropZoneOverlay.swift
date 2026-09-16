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

/// Action type for drop zones
public enum DropActionType: Int {
    case swap = 0     // Swap window assignments (safest)
    case insert = 1   // Insert as sibling
    case split = 2    // Split into new container
}

/// Represents a visible drop zone on screen
public struct DropZone: Identifiable {
    public let id = UUID()
    public let frame: CGRect
    public let position: DropPosition
    public let isHighlighted: Bool
    public let actionType: DropActionType
    public let dwellProgress: Float  // 0.0-1.0 for split zones
    public let screenIndex: Int      // Which screen this zone belongs to

    public init(
        frame: CGRect,
        position: DropPosition,
        isHighlighted: Bool = false,
        actionType: DropActionType = .swap,
        dwellProgress: Float = 0.0,
        screenIndex: Int = 0
    ) {
        self.frame = frame
        self.position = position
        self.isHighlighted = isHighlighted
        self.actionType = actionType
        self.dwellProgress = dwellProgress
        self.screenIndex = screenIndex
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

    /// Color based on action type: blue for swap/insert, amber for split
    var zoneColor: Color {
        switch zone.actionType {
        case .swap, .insert:
            return .accentColor
        case .split:
            return .orange
        }
    }

    /// Text explaining what this zone does
    var actionLabel: String {
        switch zone.actionType {
        case .swap:
            return "Swap"
        case .insert:
            switch zone.position {
            case .left: return "Insert Left"
            case .right: return "Insert Right"
            case .top: return "Insert Above"
            case .bottom: return "Insert Below"
            case .tab: return "Add Tab"
            case .newColumn: return "New Column"
            }
        case .split:
            switch zone.position {
            case .left, .right: return "Split V"
            case .top, .bottom: return "Split H"
            case .tab, .newColumn: return "Split"
            }
        }
    }

    var body: some View {
        ZStack {
            // Background fill
            RoundedRectangle(cornerRadius: 8)
                .fill(isActive ? zoneColor.opacity(0.4) : zoneColor.opacity(0.2))

            // Dwell progress overlay for split zones
            if zone.actionType == .split && zone.dwellProgress > 0 {
                GeometryReader { geo in
                    RoundedRectangle(cornerRadius: 8)
                        .fill(zoneColor.opacity(0.3))
                        .frame(width: geo.size.width * CGFloat(zone.dwellProgress))
                        .animation(.linear(duration: 0.1), value: zone.dwellProgress)
                }
            }

            // Border
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(
                    isActive ? zoneColor : zoneColor.opacity(0.5),
                    lineWidth: isActive ? 3 : 2
                )

            // Content: icon and label
            VStack(spacing: 4) {
                dropIcon
                    .font(.system(size: isActive ? 28 : 20))
                    .foregroundColor(isActive ? zoneColor : .secondary)

                Text(actionLabel)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(isActive ? zoneColor : .secondary)
            }
        }
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
        self.ignoresMouseEvents = true  // Let mouse events pass through to windows below
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
    private var overlayWindows: [DropZoneOverlayWindow] = []
    private var dropZones: [DropZone] = []
    private var activeZoneId: UUID?

    public static let shared = DropZoneManager()

    private init() {}

    /// Filter zones that belong to this screen by screen index.
    /// Rust sends zones with a screen_index field indicating which screen they belong to.
    /// Zones are in screen-local coordinates (0,0 to width,height).
    private func filterZonesForScreen(_ zones: [DropZone], screenIndex: Int) -> [DropZone] {
        return zones.filter { zone in
            zone.screenIndex == screenIndex
        }
    }

    /// Shows drop zones on all screens
    public func showDropZones(_ zones: [DropZone]) {
        dropZones = zones
        let screens = NSScreen.screens

        // Ensure we have the right number of windows
        while overlayWindows.count < screens.count {
            overlayWindows.append(DropZoneOverlayWindow(screen: screens[overlayWindows.count]))
        }

        // Update each screen's overlay
        for (index, screen) in screens.enumerated() {
            let window = overlayWindows[index]

            // Reposition window in case screen arrangement changed (use visibleFrame)
            window.setFrame(screen.visibleFrame, display: false)

            let screenZones = filterZonesForScreen(zones, screenIndex: index)
            window.updateDropZones(screenZones, activeZone: activeZoneId)
            window.orderFront(nil)
        }

        // Hide any extra windows
        for index in screens.count..<overlayWindows.count {
            overlayWindows[index].orderOut(nil)
        }
    }

    /// Updates which zone is currently hovered
    public func setActiveZone(_ zoneId: UUID?) {
        activeZoneId = zoneId
        let screens = NSScreen.screens

        for (index, _) in screens.enumerated() where index < overlayWindows.count {
            let screenZones = filterZonesForScreen(dropZones, screenIndex: index)
            overlayWindows[index].updateDropZones(screenZones, activeZone: activeZoneId)
        }
    }

    /// Hides all drop zone overlays
    public func hideDropZones() {
        for window in overlayWindows {
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
/// zones_ptr: pointer to array of 10 floats per zone:
///   (x, y, width, height, position_type, target_window_id, action_type, is_active, dwell_progress, screen_index)
@_cdecl("sugarglider_show_drop_zones")
public func showDropZones(zonesPtr: UnsafePointer<Float>, count: Int) {
    // Parse 10 floats per zone
    let floatsPerZone = 10
    var zones: [DropZone] = []

    for i in 0..<count {
        let base = i * floatsPerZone
        let x = CGFloat(zonesPtr[base])
        let y = CGFloat(zonesPtr[base + 1])
        let width = CGFloat(zonesPtr[base + 2])
        let height = CGFloat(zonesPtr[base + 3])
        let positionType = Int(zonesPtr[base + 4])
        let targetWindowId = WindowId(zonesPtr[base + 5])
        let actionTypeRaw = Int(zonesPtr[base + 6])
        let isActive = zonesPtr[base + 7] > 0.5
        let dwellProgress = zonesPtr[base + 8]
        let screenIndex = Int(zonesPtr[base + 9])

        let frame = CGRect(x: x, y: y, width: width, height: height)
        let position = positionFromType(positionType, targetId: targetWindowId)
        let actionType = DropActionType(rawValue: actionTypeRaw) ?? .swap

        zones.append(DropZone(
            frame: frame,
            position: position,
            isHighlighted: isActive,
            actionType: actionType,
            dwellProgress: dwellProgress,
            screenIndex: screenIndex
        ))
    }

    DispatchQueue.main.async {
        DropZoneManager.shared.showDropZones(zones)
    }
}

/// Convert position type integer to DropPosition enum
private func positionFromType(_ type: Int, targetId: WindowId) -> DropPosition {
    switch type {
    case 0: return .left(of: targetId)
    case 1: return .right(of: targetId)
    case 2: return .top(of: targetId)
    case 3: return .bottom(of: targetId)
    case 4: return .tab(with: targetId)
    case 5: return .newColumn
    default: return .left(of: targetId)
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
