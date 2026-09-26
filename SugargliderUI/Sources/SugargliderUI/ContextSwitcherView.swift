// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import AppKit
import SwiftUI

/// The switcher's content: a search field, the list of contexts or the
/// window checklist, inline messages, and key hints.
struct ContextSwitcherView: View {
  @ObservedObject var model: ContextSwitcherModel
  @FocusState private var fieldFocused: Bool

  private static let rowHeight: CGFloat = 30
  private static let listPadding: CGFloat = 6
  private static let maxListHeight: CGFloat = 300

  var body: some View {
    VStack(spacing: 0) {
      VStack(spacing: 0) {
        header
        Divider()
        content
        notices
        Divider()
        footer
      }
      .background(SwitcherBackground())
      .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
      .overlay(
        RoundedRectangle(cornerRadius: 12, style: .continuous)
          .strokeBorder(Color.primary.opacity(0.12))
      )
      Spacer(minLength: 0)
    }
    .frame(
      width: ContextSwitcherPanel.size.width,
      height: ContextSwitcherPanel.size.height,
      alignment: .top
    )
    .task(id: model.mode) {
      fieldFocused = true
    }
  }

  // MARK: Header

  @ViewBuilder
  private var header: some View {
    switch model.mode {
    case .list, .confirmDelete:
      field("Switch context…", text: $model.query, icon: "magnifyingglass")
    case .naming:
      field("Name the new context…", text: $model.nameDraft, icon: "plus")
    case .rename(let id):
      field("New name for “\(name(id))”", text: $model.nameDraft, icon: "pencil")
    case .create(let name):
      title("New context “\(name)”")
    case .edit(let id):
      title("Windows of “\(name(id))”")
    }
  }

  private func field(_ placeholder: String, text: Binding<String>, icon: String) -> some View {
    HStack(spacing: 10) {
      Image(systemName: icon)
        .foregroundStyle(.secondary)
      TextField(placeholder, text: text)
        .textFieldStyle(.plain)
        .font(.system(size: 18))
        .focused($fieldFocused)
    }
    .padding(.horizontal, 16)
    .padding(.vertical, 12)
  }

  private func title(_ text: String) -> some View {
    HStack {
      Text(text)
        .font(.system(size: 18, weight: .medium))
        .lineLimit(1)
      Spacer()
    }
    .padding(.horizontal, 16)
    .padding(.vertical, 12)
  }

  // MARK: Content

  @ViewBuilder
  private var content: some View {
    switch model.mode {
    case .list, .confirmDelete, .rename:
      contextList
    case .naming:
      placeholder("Type a name, then press ↩ to choose its windows.")
    case .create, .edit:
      checklist
    }
  }

  @ViewBuilder
  private var contextList: some View {
    if model.rows.isEmpty {
      placeholder(
        model.isFirstTime ? "Type a name to create your first context" : "No matching contexts"
      )
    } else {
      ScrollViewReader { proxy in
        ScrollView {
          VStack(spacing: 0) {
            ForEach(Array(model.rows.enumerated()), id: \.offset) { index, row in
              rowView(row, highlighted: index == model.highlight)
                .id(index)
                .contentShape(Rectangle())
                .onTapGesture { model.click(row: index) }
            }
          }
          .padding(Self.listPadding)
        }
        .frame(height: listHeight(model.rows.count))
        .onChange(of: model.highlight) { index in
          proxy.scrollTo(index)
        }
      }
    }
  }

  private func rowView(_ row: SwitcherRow, highlighted: Bool) -> some View {
    HStack(spacing: 8) {
      Text(highlighted ? "▸" : " ")
        .frame(width: 10)
      switch row {
      case .entry(let entry):
        Text(entry.active ? "●" : " ")
          .foregroundStyle(Color.accentColor)
          .frame(width: 10)
        Text(entry.name)
          .fontWeight(.medium)
          .lineLimit(1)
        Text(entry.detail)
          .foregroundStyle(.secondary)
          .lineLimit(1)
        Spacer(minLength: 8)
        if let shortcut = entry.shortcut {
          Text(shortcut)
            .foregroundStyle(.secondary)
            .monospacedDigit()
        }
      case .newContext(let name):
        Image(systemName: "plus")
          .foregroundStyle(.secondary)
          .frame(width: 10)
        Text("New context “\(name)”")
          .lineLimit(1)
        Spacer(minLength: 8)
      }
    }
    .font(.system(size: 14))
    .padding(.horizontal, 10)
    .frame(height: Self.rowHeight)
    .background(
      RoundedRectangle(cornerRadius: 6, style: .continuous)
        .fill(highlighted ? Color.accentColor.opacity(0.2) : Color.clear)
    )
  }

  @ViewBuilder
  private var checklist: some View {
    if model.checklist.isEmpty {
      placeholder("No windows on screen")
    } else {
      ScrollViewReader { proxy in
        ScrollView {
          VStack(spacing: 0) {
            ForEach(Array(model.checklist.enumerated()), id: \.offset) { index, item in
              checklistRow(item, highlighted: index == model.checklistHighlight)
                .id(index)
                .contentShape(Rectangle())
                .onTapGesture { model.toggleItem(at: index) }
            }
          }
          .padding(Self.listPadding)
        }
        .frame(height: listHeight(model.checklist.count))
        .onChange(of: model.checklistHighlight) { index in
          proxy.scrollTo(index)
        }
      }
    }
  }

  private func checklistRow(_ item: SwitcherChecklistItem, highlighted: Bool) -> some View {
    HStack(spacing: 8) {
      Image(systemName: item.checked ? "checkmark.square.fill" : "square")
        .foregroundStyle(item.checked && !item.pinned ? Color.accentColor : Color.secondary)
      Text(item.title.isEmpty ? "Untitled" : item.title)
        .lineLimit(1)
      Text(item.app)
        .foregroundStyle(.secondary)
        .lineLimit(1)
      Spacer(minLength: 8)
      if let note = note(for: item) {
        Text(note)
          .font(.system(size: 12))
          .foregroundStyle(.secondary)
      }
    }
    .font(.system(size: 14))
    .padding(.horizontal, 10)
    .frame(height: Self.rowHeight)
    .background(
      RoundedRectangle(cornerRadius: 6, style: .continuous)
        .fill(highlighted ? Color.accentColor.opacity(0.2) : Color.clear)
    )
  }

  /// Marks pinned windows, and members that the user can't see on screen.
  private func note(for item: SwitcherChecklistItem) -> String? {
    if item.pinned {
      return "pinned"
    }
    switch item.source {
    case .record:
      return "closed"
    case .window(let id):
      return model.payload.windows.contains { $0.id == id } ? nil : "not on screen"
    }
  }

  private func placeholder(_ text: String) -> some View {
    Text(text)
      .foregroundStyle(.secondary)
      .frame(maxWidth: .infinity)
      .padding(.vertical, 24)
  }

  private func listHeight(_ count: Int) -> CGFloat {
    min(CGFloat(count) * Self.rowHeight + 2 * Self.listPadding, Self.maxListHeight)
  }

  // MARK: Notices

  @ViewBuilder
  private var notices: some View {
    if case .confirmDelete(let id) = model.mode {
      notice(
        "Delete “\(name(id))”? Its windows stay open.",
        icon: "trash",
        color: .red
      )
    }
    if let message = model.message {
      notice(message, icon: "exclamationmark.circle", color: .red)
    }
    if let rankError = model.rankError {
      notice(rankError, icon: "exclamationmark.triangle", color: .orange)
    }
  }

  private func notice(_ text: String, icon: String, color: Color) -> some View {
    HStack(spacing: 8) {
      Image(systemName: icon)
        .foregroundStyle(color)
      Text(text)
        .lineLimit(2)
      Spacer(minLength: 0)
    }
    .font(.system(size: 13))
    .padding(.horizontal, 16)
    .padding(.vertical, 8)
    .background(color.opacity(0.1))
  }

  // MARK: Footer

  private var footer: some View {
    VStack(alignment: .leading, spacing: 4) {
      ForEach(Array(hintLines.enumerated()), id: \.offset) { _, line in
        HStack(spacing: 14) {
          ForEach(Array(line.enumerated()), id: \.offset) { _, hint in
            (Text(hint.key).fontWeight(.semibold) + Text(" " + hint.action))
              .lineLimit(1)
          }
          Spacer(minLength: 0)
        }
      }
      if model.mode == .list, let target = model.targetWindow {
        Text("Window: \(target.title.isEmpty ? "Untitled" : target.title) — \(target.app)")
          .lineLimit(1)
      }
    }
    .font(.system(size: 12))
    .foregroundStyle(.secondary)
    .padding(.horizontal, 16)
    .padding(.vertical, 10)
  }

  private var hintLines: [[(key: String, action: String)]] {
    switch model.mode {
    case .list:
      let pin = model.targetWindow?.pinned == true ? "unpin" : "pin"
      return [
        [("↩", "switch"), ("⌘↩", "add window"), ("⇧⌘↩", "move window"), ("⌘N", "new")],
        [
          ("⌘E", "edit"), ("⌘R", "rename"), ("⌘1–9", "number"), ("⌘P", pin),
          ("⌘⌫", "delete"), ("esc", "close"),
        ],
      ]
    case .naming:
      return [[("↩", "choose windows"), ("esc", "back")]]
    case .rename:
      return [[("↩", "rename"), ("esc", "back")]]
    case .create:
      return [[("↑↓", "select"), ("space", "check"), ("↩", "create"), ("esc", "back")]]
    case .edit:
      return [[("↑↓", "select"), ("space", "check"), ("↩", "save"), ("esc", "back")]]
    case .confirmDelete:
      return [[("↩", "delete"), ("esc", "cancel")]]
    }
  }

  private func name(_ id: SwitcherContextId) -> String {
    model.context(id)?.name ?? ""
  }
}

/// A blurred background that shows through the panel's clear window.
private struct SwitcherBackground: NSViewRepresentable {
  func makeNSView(context: Context) -> NSVisualEffectView {
    let view = NSVisualEffectView()
    view.material = .popover
    view.blendingMode = .behindWindow
    view.state = .active
    return view
  }

  func updateNSView(_ view: NSVisualEffectView, context: Context) {}
}
