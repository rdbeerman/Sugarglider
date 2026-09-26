// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JSON types for preferences UI communication.
//!
//! This module defines types that are serialized to JSON for exchange between
//! the Swift preferences UI and the Rust backend.

use std::str::FromStr;

use livesplit_hotkey::Hotkey;
use serde::{Deserialize, Serialize};

use crate::actor::layout::{LayoutCommand, SizeShare};
use crate::actor::reactor::{
    Command as ReactorCommand, ContextCommand, ContextRef, ReactorCommand as ReactorCmd,
};
use crate::actor::wm_controller::{WmCmd, WmCommand};
use crate::config::{Config, WindowRule, WindowRuleConditions};
use crate::log::MetricsCommand;
use crate::model::{Direction, LayoutKind, Orientation};

/// Subset of Config fields editable via the preferences UI.
///
/// Uses JSON-friendly field names (snake_case maps to Swift camelCase).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesJson {
    // General settings
    pub status_icon_enable: bool,
    pub animate: bool,
    pub focus_follows_mouse: bool,
    pub mouse_follows_focus: bool,
    pub outer_gap: f64,
    pub inner_gap: f64,

    // Dragging behavior
    pub drag_drop_enable: bool,
    pub drag_drop_live_preview: bool,

    // Layout settings
    pub default_layout_kind: String,

    // Experimental features
    pub contexts_enable: bool,

    // Window rules
    pub window_rules: Vec<WindowRuleJson>,

    // Hotkey bindings (read-only for now)
    pub hotkeys: Vec<HotkeyBindingJson>,
}

/// JSON representation of a hotkey binding.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyBindingJson {
    /// The formatted hotkey string (e.g., "⌥H")
    pub key: String,
    /// The command identifier for internal use
    pub command_id: String,
    /// Human-readable description of what the command does
    pub description: String,
    /// Category for grouping in the UI
    pub category: String,
    /// The default hotkey for this command (if any)
    pub default_key: Option<String>,
    /// Sort order within the category (lower = earlier). The Swift UI doesn't
    /// send it back.
    #[serde(default)]
    pub sort_order: u32,
}

/// JSON representation of a window rule.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowRuleJson {
    pub app_name: Option<String>,
    pub bundle_id: Option<String>,
    pub behavior: String, // "tile" or "float"
}

impl PreferencesJson {
    /// Create PreferencesJson from a Config.
    pub fn from_config(config: &Config) -> Self {
        let settings = &config.settings;

        Self {
            status_icon_enable: settings.status_icon.enable,
            animate: settings.animate,
            focus_follows_mouse: settings.focus_follows_mouse,
            mouse_follows_focus: settings.mouse_follows_focus,
            outer_gap: settings.outer_gap,
            inner_gap: settings.inner_gap,
            drag_drop_enable: settings.drag_drop.enable,
            drag_drop_live_preview: settings.drag_drop.live_preview,
            default_layout_kind: match settings.default_layout_kind {
                LayoutKind::Tree => "tree".to_string(),
                LayoutKind::Scroll => "scroll".to_string(),
            },
            contexts_enable: settings.experimental.contexts.enable,
            window_rules: config.window_rules.iter().map(WindowRuleJson::from_rule).collect(),
            hotkeys: {
                // Build a map of command_id -> default hotkey from the default config
                let default_config = Config::default();
                let default_keys: std::collections::HashMap<String, String> =
                    command_ids(&default_config.keys)
                        .into_iter()
                        .zip(&default_config.keys)
                        .map(|((command_id, _), (hotkey, _))| {
                            (command_id, format_hotkey(hotkey))
                        })
                        .collect();

                let mut hotkeys: Vec<_> = command_ids(&config.keys)
                    .into_iter()
                    .zip(&config.keys)
                    .map(|((command_id, cmd), (hotkey, _))| {
                        let default_key = default_keys.get(&command_id).cloned();
                        HotkeyBindingJson::from_binding(hotkey, &cmd, command_id, default_key)
                    })
                    .collect();

                // Sort by category, then by sort_order within each category
                hotkeys.sort_by(|a, b| {
                    a.category.cmp(&b.category).then_with(|| a.sort_order.cmp(&b.sort_order))
                });

                hotkeys
            },
        }
    }

    /// Apply this preferences JSON to a Config, returning a new Config.
    ///
    /// Fields not present in PreferencesJson are preserved from the original.
    pub fn apply_to_config(&self, config: &Config) -> Config {
        let mut settings = config.settings.clone();

        settings.status_icon.enable = self.status_icon_enable;
        settings.animate = self.animate;
        settings.focus_follows_mouse = self.focus_follows_mouse;
        settings.mouse_follows_focus = self.mouse_follows_focus;
        settings.outer_gap = self.outer_gap;
        settings.inner_gap = self.inner_gap;
        settings.drag_drop.enable = self.drag_drop_enable;
        settings.drag_drop.live_preview = self.drag_drop_live_preview;
        settings.default_layout_kind = match self.default_layout_kind.as_str() {
            "scroll" => LayoutKind::Scroll,
            _ => LayoutKind::Tree,
        };
        settings.experimental.contexts.enable = self.contexts_enable;

        let window_rules: Vec<WindowRule> =
            self.window_rules.iter().map(WindowRuleJson::to_rule).collect();

        // Build new keys from hotkeys, matching by command_id
        let keys = self.build_keys_from_hotkeys(config);

        Config { settings, window_rules, keys }
    }

    /// Build the keys vector from hotkeys JSON, using original commands from config.
    fn build_keys_from_hotkeys(&self, config: &Config) -> Vec<(Hotkey, WmCommand)> {
        // Create a map from command_id to the original command.
        // Include both the current config commands AND default config commands
        // to ensure we have all standard commands available.
        let mut command_map: std::collections::HashMap<String, WmCommand> =
            std::collections::HashMap::new();

        // Add default commands first
        for (command_id, cmd) in command_ids(&Config::default().keys) {
            command_map.insert(command_id, cmd);
        }

        // Override with current config commands (for custom exec commands, etc.)
        for (command_id, cmd) in command_ids(&config.keys) {
            command_map.insert(command_id, cmd);
        }

        self.hotkeys
            .iter()
            .filter_map(|hk| {
                // Get the original command for this command_id
                let Some(cmd) = command_map.get(&hk.command_id) else {
                    tracing::warn!(
                        "Unknown command_id '{}' for hotkey '{}', dropping binding",
                        hk.command_id,
                        hk.key
                    );
                    return None;
                };

                // Parse the hotkey string back to a Hotkey
                let Some(hotkey) = parse_hotkey_string(&hk.key) else {
                    tracing::warn!(
                        "Invalid hotkey format '{}' for command '{}', dropping binding",
                        hk.key,
                        hk.command_id
                    );
                    return None;
                };

                Some((hotkey, cmd.clone()))
            })
            .collect()
    }
}

/// Parse a macOS-style hotkey string (e.g., "⌥⇧H") back to a livesplit Hotkey.
fn parse_hotkey_string(s: &str) -> Option<Hotkey> {
    let mut modifiers = Vec::new();
    let mut key_part = String::new();

    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '⌃' => modifiers.push("Ctrl"),
            '⌥' => modifiers.push("Alt"),
            '⇧' => modifiers.push("Shift"),
            '⌘' => modifiers.push("Cmd"),
            _ => {
                // Everything else is the key
                key_part.push(c);
                key_part.extend(chars);
                break;
            }
        }
    }

    if key_part.is_empty() || modifiers.is_empty() {
        // Need both a key and at least one modifier for a valid hotkey
        return None;
    }

    // Convert key symbols back to livesplit format
    let key_name = match key_part.as_str() {
        "←" => "ArrowLeft",
        "→" => "ArrowRight",
        "↑" => "ArrowUp",
        "↓" => "ArrowDown",
        "\\" => "Backslash",
        "/" => "Slash",
        "=" => "Equal",
        "Space" => "Space",
        "Esc" => "Escape",
        "⌫" => "Backspace",
        "↩" => "Return",
        "⇥" => "Tab",
        "-" => "Minus",
        "[" => "BracketLeft",
        "]" => "BracketRight",
        "'" => "Quote",
        ";" => "Semicolon",
        "," => "Comma",
        "." => "Period",
        "`" => "Backquote",
        other => {
            // Single letter or number - prepend "Key" for letters
            if other.len() == 1 {
                let c = other.chars().next().unwrap();
                if c.is_ascii_alphabetic() {
                    // Will be handled below with format
                    other
                } else {
                    other
                }
            } else {
                other
            }
        }
    };

    // Build the hotkey string in livesplit format
    let mut parts = modifiers.iter().map(|s| s.to_string()).collect::<Vec<_>>();

    // Add key name with proper prefix
    let final_key = if key_name.len() == 1 && key_name.chars().next().unwrap().is_ascii_alphabetic()
    {
        format!("Key{}", key_name.to_uppercase())
    } else if key_name.len() == 1 && key_name.chars().next().unwrap().is_ascii_digit() {
        format!("Digit{}", key_name)
    } else {
        key_name.to_string()
    };
    parts.push(final_key);

    let hotkey_str = parts.join(" + ");
    Hotkey::from_str(&hotkey_str).ok()
}

impl WindowRuleJson {
    /// Create from a WindowRule.
    pub fn from_rule(rule: &WindowRule) -> Self {
        Self {
            app_name: rule.conditions.app_name.clone(),
            bundle_id: rule.conditions.app_id.clone(),
            behavior: if rule.float { "float" } else { "tile" }.to_string(),
        }
    }

    /// Convert to a WindowRule.
    pub fn to_rule(&self) -> WindowRule {
        WindowRule {
            conditions: WindowRuleConditions {
                app_id: self.bundle_id.clone(),
                app_name: self.app_name.clone(),
                title_regex: None,
                title_substring: None,
                ax_role: None,
                ax_subrole: None,
            },
            float: self.behavior == "float",
        }
    }
}

impl HotkeyBindingJson {
    /// Create from a hotkey binding, with the command id that
    /// [`command_ids`] gives the binding and an optional default key.
    fn from_binding(
        hotkey: &Hotkey,
        cmd: &WmCommand,
        command_id: String,
        default_key: Option<String>,
    ) -> Self {
        let (description, category, _, sort_order) = describe_command(cmd);
        Self {
            key: format_hotkey(hotkey),
            command_id,
            description,
            category,
            default_key,
            sort_order,
        }
    }
}

/// The command id of each binding, in the order of `keys`, each with its
/// command. Two bindings can run the same command, and one command can name
/// different records or numbers in each binding, so the first binding keeps
/// the name that `describe_command` gives its command and each later one
/// takes an occurrence number. The Preferences UI keys its rows by command
/// id, so the ids must be unique for it to tell two bindings apart.
fn command_ids(keys: &[(Hotkey, WmCommand)]) -> Vec<(String, WmCommand)> {
    let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    keys.iter()
        .map(|(_, cmd)| {
            let base = describe_command(cmd).2;
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            let id = match count {
                1 => base,
                n => format!("{base}#{n}"),
            };
            (id, cmd.clone())
        })
        .collect()
}

/// Format a hotkey using macOS-style symbols.
fn format_hotkey(hotkey: &Hotkey) -> String {
    let s = hotkey.to_string();
    // Convert "Alt + Ctrl + Shift + KeyH" to "⌥⌃⇧H"
    let mut result = String::new();

    // Check for modifiers in order: Ctrl, Alt/Option, Shift, Cmd
    if s.contains("Ctrl") {
        result.push('⌃');
    }
    if s.contains("Alt") {
        result.push('⌥');
    }
    if s.contains("Shift") {
        result.push('⇧');
    }
    if s.contains("Cmd") || s.contains("Super") {
        result.push('⌘');
    }

    // Extract the key name (last part after " + ")
    if let Some(key_part) = s.rsplit(" + ").next() {
        let key_name = key_part
            .strip_prefix("Key")
            .or_else(|| key_part.strip_prefix("Numpad"))
            .or_else(|| key_part.strip_prefix("Digit"))
            .unwrap_or(key_part);

        // Map special keys
        let formatted = match key_name {
            "ArrowLeft" => "←",
            "ArrowRight" => "→",
            "ArrowUp" => "↑",
            "ArrowDown" => "↓",
            "Backslash" => "\\",
            "Equal" => "=",
            "Slash" => "/",
            "Space" => "Space",
            "Escape" => "Esc",
            "Backspace" => "⌫",
            "Enter" | "Return" => "↩",
            "Tab" => "⇥",
            _ => key_name,
        };
        result.push_str(formatted);
    }

    result
}

/// Get the description, category, and command ID for a WmCommand.
/// Public wrapper for use from config.rs.
pub fn describe_command_for_toml(cmd: &WmCommand) -> (String, String, String) {
    let (desc, cat, id, _) = describe_command(cmd);
    (desc, cat, id)
}

/// Get the description, category, command ID, and sort order for a WmCommand.
fn describe_command(cmd: &WmCommand) -> (String, String, String, u32) {
    match cmd {
        WmCommand::Wm(wm_cmd) => match wm_cmd {
            WmCmd::ToggleGlobalEnabled => (
                "Toggle tiling globally".to_string(),
                "System".to_string(),
                "toggle_global_enabled".to_string(),
                0,
            ),
            WmCmd::SetGlobalEnabled(enabled) => (
                format!("Set tiling {}", if *enabled { "on" } else { "off" }),
                "System".to_string(),
                "set_global_enabled".to_string(),
                1,
            ),
            WmCmd::ToggleSpaceActivated => (
                "Toggle tiling on current space".to_string(),
                "System".to_string(),
                "toggle_space_activated".to_string(),
                2,
            ),
            WmCmd::Exec(_) => (
                "Execute command".to_string(),
                "Utilities".to_string(),
                "exec".to_string(),
                0,
            ),
        },
        WmCommand::ReactorCommand(reactor_cmd) => match reactor_cmd {
            ReactorCommand::Layout(layout_cmd) => describe_layout_command(layout_cmd),
            ReactorCommand::Metrics(metrics_cmd) => match metrics_cmd {
                MetricsCommand::ShowTiming => (
                    "Show performance timing".to_string(),
                    "Developer".to_string(),
                    "show_timing".to_string(),
                    0,
                ),
            },
            ReactorCommand::Reactor(reactor_cmd) => match reactor_cmd {
                ReactorCmd::Debug => (
                    "Print layout debug info".to_string(),
                    "Developer".to_string(),
                    "debug".to_string(),
                    0,
                ),
                ReactorCmd::Serialize => (
                    "Serialize layout state".to_string(),
                    "Developer".to_string(),
                    "serialize".to_string(),
                    1,
                ),
                ReactorCmd::SaveAndExit => (
                    "Save state and exit".to_string(),
                    "System".to_string(),
                    "save_and_exit".to_string(),
                    10,
                ),
            },
            ReactorCommand::Context(context_cmd) => describe_context_command(context_cmd),
        },
    }
}

/// Get the description, category, command ID, and sort order for a ContextCommand.
fn describe_context_command(cmd: &ContextCommand) -> (String, String, String, u32) {
    let category = "Contexts".to_string();
    match cmd {
        ContextCommand::ShowEverything => (
            "Show every window".to_string(),
            category,
            "show_everything".to_string(),
            0,
        ),
        ContextCommand::SwitchContext(ContextRef::Number(number)) => (
            format!("Switch to context {number}"),
            category,
            format!("switch_context_{number}"),
            u32::from(*number),
        ),
        ContextCommand::PreviousContext => (
            "Switch to the previous context".to_string(),
            category,
            "previous_context".to_string(),
            10,
        ),
        ContextCommand::SwitchContext(ContextRef::Name(name)) => (
            format!("Switch to context \"{name}\""),
            category,
            format!("switch_context_name_{name}"),
            20,
        ),
        ContextCommand::SwitchContext(ContextRef::Id(id)) => (
            format!("Switch to context with id {}", id.get()),
            category,
            format!("switch_context_id_{}", id.get()),
            30,
        ),
        ContextCommand::AddWindowToContext(reference) => {
            let (name, id, order) = describe_context_ref(reference);
            (
                format!("Add the window to {name}"),
                category,
                format!("add_window_to_context_{id}"),
                40 + order,
            )
        }
        ContextCommand::MoveWindowToContext(reference) => {
            let (name, id, order) = describe_context_ref(reference);
            (
                format!("Move the window to {name}"),
                category,
                format!("move_window_to_context_{id}"),
                60 + order,
            )
        }
        ContextCommand::RemoveWindowFromContext => (
            "Remove the window from the active context".to_string(),
            category,
            "remove_window_from_context".to_string(),
            80,
        ),
        ContextCommand::ToggleWindowPinned => (
            "Pin or unpin the window".to_string(),
            category,
            "toggle_window_pinned".to_string(),
            81,
        ),
        ContextCommand::CreateContext(name) => (
            format!("Create context \"{name}\" from the windows on screen"),
            category,
            format!("create_context_{name}"),
            40,
        ),
        ContextCommand::RenameContext { context, name } => {
            let (context, id, order) = describe_context_ref(context);
            (
                format!("Rename {context} to \"{name}\""),
                category,
                format!("rename_context_{id}"),
                100 + order,
            )
        }
        ContextCommand::SetContextNumber { context, number } => {
            let (context, id, order) = describe_context_ref(context);
            (
                format!("Give {context} the number {number}"),
                category,
                format!("set_context_number_{id}"),
                120 + order,
            )
        }
        ContextCommand::DeleteContext(context) => {
            let (context, id, order) = describe_context_ref(context);
            (
                format!("Delete {context}"),
                category,
                format!("delete_context_{id}"),
                140 + order,
            )
        }
        ContextCommand::EditContextMembers { context, .. } => {
            let (context, id, order) = describe_context_ref(context);
            (
                format!("Change the windows of {context}"),
                category,
                format!("edit_context_members_{id}"),
                160 + order,
            )
        }
        ContextCommand::RemoveRecord { context, record } => {
            let (context, id, order) = describe_context_ref(context);
            (
                format!("Forget member record {} of {context}", record.record),
                category,
                format!("remove_record_{id}_{}", record.record),
                180 + order,
            )
        }
    }
}

/// The description, the part of a command id, and the offset in the sort
/// order of a context that a command names.
fn describe_context_ref(reference: &ContextRef) -> (String, String, u32) {
    match reference {
        ContextRef::Number(number) => (
            format!("context {number}"),
            number.to_string(),
            u32::from(*number),
        ),
        ContextRef::Name(name) => (format!("context \"{name}\""), format!("name_{name}"), 10),
        ContextRef::Id(id) => (
            format!("context with id {}", id.get()),
            format!("id_{}", id.get()),
            11,
        ),
    }
}

/// Get the description, category, command ID, and sort order for a LayoutCommand.
fn describe_layout_command(cmd: &LayoutCommand) -> (String, String, String, u32) {
    match cmd {
        LayoutCommand::MoveFocus(dir) => (
            format!("Focus {}", direction_name(dir)),
            "Focus".to_string(),
            format!("move_focus_{}", direction_id(dir)),
            direction_sort_order(dir),
        ),
        LayoutCommand::FocusNext => (
            "Focus next window".to_string(),
            "Focus".to_string(),
            "focus_next".to_string(),
            10,
        ),
        LayoutCommand::FocusPrev => (
            "Focus previous window".to_string(),
            "Focus".to_string(),
            "focus_prev".to_string(),
            11,
        ),
        LayoutCommand::Ascend => (
            "Select parent container".to_string(),
            "Focus".to_string(),
            "ascend".to_string(),
            20,
        ),
        LayoutCommand::Descend => (
            "Select child node".to_string(),
            "Focus".to_string(),
            "descend".to_string(),
            21,
        ),
        LayoutCommand::MoveNode(dir) => (
            format!("Move window {}", direction_name(dir)),
            "Move".to_string(),
            format!("move_node_{}", direction_id(dir)),
            direction_sort_order(dir),
        ),
        LayoutCommand::Resize { direction, percent } => (
            format!("Resize {} by {}%", direction_name(direction), percent),
            "Resize".to_string(),
            format!("resize_{}", direction_id(direction)),
            10 + direction_sort_order(direction), // After SetSizeShare
        ),
        LayoutCommand::Split(orientation) => (
            format!("Split {}", orientation_name(orientation)),
            "Layout".to_string(),
            format!("split_{}", orientation_id(orientation)),
            0,
        ),
        LayoutCommand::ToggleOrientation => (
            "Toggle split orientation".to_string(),
            "Layout".to_string(),
            "toggle_orientation".to_string(),
            1,
        ),
        LayoutCommand::Group(orientation) => (
            format!(
                "Group {} ({})",
                orientation_name(orientation),
                group_mode_name(orientation)
            ),
            "Layout".to_string(),
            format!("group_{}", orientation_id(orientation)),
            10,
        ),
        LayoutCommand::Ungroup => (
            "Ungroup container".to_string(),
            "Layout".to_string(),
            "ungroup".to_string(),
            11,
        ),
        LayoutCommand::ToggleFocusFloating => (
            "Toggle focus between tiled/floating".to_string(),
            "Floating".to_string(),
            "toggle_focus_floating".to_string(),
            0,
        ),
        LayoutCommand::ToggleWindowFloating => (
            "Toggle window floating".to_string(),
            "Floating".to_string(),
            "toggle_window_floating".to_string(),
            1,
        ),
        LayoutCommand::ToggleFullscreen => (
            "Toggle fullscreen".to_string(),
            "Layout".to_string(),
            "toggle_fullscreen".to_string(),
            20,
        ),
        LayoutCommand::NextLayout => (
            "Switch to next saved layout".to_string(),
            "Layout".to_string(),
            "next_layout".to_string(),
            30,
        ),
        LayoutCommand::PrevLayout => (
            "Switch to previous saved layout".to_string(),
            "Layout".to_string(),
            "prev_layout".to_string(),
            31,
        ),
        LayoutCommand::CycleColumnWidth => (
            "Cycle column width preset".to_string(),
            "Scroll Layout".to_string(),
            "cycle_column_width".to_string(),
            0,
        ),
        LayoutCommand::ChangeLayoutKind => (
            "Change layout mode (tree/scroll)".to_string(),
            "Scroll Layout".to_string(),
            "change_layout_kind".to_string(),
            1,
        ),
        LayoutCommand::ToggleColumnTabbed => (
            "Toggle column tabbed mode".to_string(),
            "Scroll Layout".to_string(),
            "toggle_column_tabbed".to_string(),
            2,
        ),
        LayoutCommand::CleanUpSpace => (
            "Clean up space".to_string(),
            "System".to_string(),
            "clean_up_space".to_string(),
            5,
        ),
        LayoutCommand::SetSizeShare(share) => {
            // Use fractions for display: 1/2, 1/3, 1/4
            // Sort order: 1/2=0, 1/3=1, 1/4=2
            let (name, sort_order) = match share {
                SizeShare::Fraction(f) if *f == 0.5 => ("1/2".to_string(), 0),
                SizeShare::Fraction(f) if *f == 0.25 => ("1/4".to_string(), 2),
                SizeShare::Fraction(f) => (format!("{}%", (f * 100.0).round()), 5),
                SizeShare::Denominator { denominator: 3 } => ("1/3".to_string(), 1),
                SizeShare::Denominator { denominator } => {
                    (format!("1/{denominator}"), *denominator)
                }
            };
            let id = match share {
                SizeShare::Fraction(fraction) => format!("set_size_share_{fraction}"),
                SizeShare::Denominator { denominator } => {
                    format!("set_size_share_1_{denominator}")
                }
            };
            (
                format!("Toggle window size {name}"),
                "Resize".to_string(),
                id,
                sort_order,
            )
        }
        LayoutCommand::ToggleSizeLock => (
            "Lock window at current size".to_string(),
            "Resize".to_string(),
            "toggle_size_lock".to_string(),
            3, // After 1/4 (sort_order=2)
        ),
    }
}

fn direction_sort_order(dir: &Direction) -> u32 {
    match dir {
        Direction::Left => 0,
        Direction::Down => 1,
        Direction::Up => 2,
        Direction::Right => 3,
    }
}

fn direction_name(dir: &Direction) -> &'static str {
    match dir {
        Direction::Left => "left",
        Direction::Right => "right",
        Direction::Up => "up",
        Direction::Down => "down",
    }
}

fn direction_id(dir: &Direction) -> &'static str {
    match dir {
        Direction::Left => "left",
        Direction::Right => "right",
        Direction::Up => "up",
        Direction::Down => "down",
    }
}

fn orientation_name(orientation: &Orientation) -> &'static str {
    match orientation {
        Orientation::Horizontal => "horizontally",
        Orientation::Vertical => "vertically",
    }
}

fn orientation_id(orientation: &Orientation) -> &'static str {
    match orientation {
        Orientation::Horizontal => "horizontal",
        Orientation::Vertical => "vertical",
    }
}

fn group_mode_name(orientation: &Orientation) -> &'static str {
    match orientation {
        Orientation::Horizontal => "tabbed",
        Orientation::Vertical => "stacked",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::reactor::RecordRef;

    #[test]
    fn test_preferences_json_roundtrip() {
        let prefs = PreferencesJson {
            status_icon_enable: true,
            animate: false,
            focus_follows_mouse: true,
            mouse_follows_focus: false,
            outer_gap: 10.0,
            inner_gap: 5.0,
            drag_drop_enable: true,
            drag_drop_live_preview: true,
            default_layout_kind: "tree".to_string(),
            contexts_enable: true,
            window_rules: vec![WindowRuleJson {
                app_name: Some("Finder".to_string()),
                bundle_id: Some("com.apple.finder".to_string()),
                behavior: "float".to_string(),
            }],
            hotkeys: vec![HotkeyBindingJson {
                key: "⌥H".to_string(),
                command_id: "move_focus_left".to_string(),
                description: "Focus left".to_string(),
                category: "Focus".to_string(),
                default_key: Some("⌥H".to_string()),
                sort_order: 0,
            }],
        };

        let json = serde_json::to_string(&prefs).unwrap();
        let decoded: PreferencesJson = serde_json::from_str(&json).unwrap();
        assert_eq!(prefs, decoded);
    }

    #[test]
    fn test_window_rule_json_conversion() {
        let rule = WindowRule {
            conditions: WindowRuleConditions {
                app_id: Some("com.apple.finder".to_string()),
                app_name: Some("Finder".to_string()),
                ..Default::default()
            },
            float: true,
        };

        let json = WindowRuleJson::from_rule(&rule);
        assert_eq!(json.bundle_id, Some("com.apple.finder".to_string()));
        assert_eq!(json.behavior, "float");

        let converted = json.to_rule();
        assert_eq!(converted.conditions.app_id, rule.conditions.app_id);
        assert_eq!(converted.float, rule.float);
    }

    #[test]
    fn test_parse_hotkey_string() {
        // Simple modifier + letter
        let hk = parse_hotkey_string("⌥H").unwrap();
        assert_eq!(hk.to_string(), "Alt + KeyH");

        // Multiple modifiers
        let hk = parse_hotkey_string("⌥⇧H").unwrap();
        assert_eq!(hk.to_string(), "Alt + Shift + KeyH");

        // Ctrl + Alt + letter
        let hk = parse_hotkey_string("⌃⌥K").unwrap();
        assert_eq!(hk.to_string(), "Ctrl + Alt + KeyK");

        // Arrow keys
        let hk = parse_hotkey_string("⌥←").unwrap();
        assert_eq!(hk.to_string(), "Alt + ArrowLeft");

        // Special keys
        let hk = parse_hotkey_string("⌥Space").unwrap();
        assert_eq!(hk.to_string(), "Alt + Space");

        let hk = parse_hotkey_string("⌥\\").unwrap();
        assert_eq!(hk.to_string(), "Alt + Backslash");

        // No modifier should fail
        assert!(parse_hotkey_string("H").is_none());
    }

    /// Key bindings. Context bindings survive the round trip through the
    /// preferences JSON, each with its own command id.
    #[test]
    fn context_bindings_survive_the_preferences_round_trip() {
        let id = serde_json::from_value(serde_json::json!(7)).unwrap();
        let bindings = [
            ("Ctrl + Alt + Digit0", ContextCommand::ShowEverything),
            (
                "Ctrl + Alt + Digit1",
                ContextCommand::SwitchContext(ContextRef::Number(1)),
            ),
            (
                "Ctrl + Alt + Digit2",
                ContextCommand::SwitchContext(ContextRef::Number(2)),
            ),
            (
                "Ctrl + Alt + KeyC",
                ContextCommand::SwitchContext(ContextRef::Name("Comms".into())),
            ),
            (
                "Ctrl + Alt + KeyI",
                ContextCommand::SwitchContext(ContextRef::Id(id)),
            ),
            ("Ctrl + Alt + Tab", ContextCommand::PreviousContext),
            (
                "Ctrl + Alt + KeyA",
                ContextCommand::AddWindowToContext(ContextRef::Number(2)),
            ),
            (
                "Ctrl + Alt + KeyM",
                ContextCommand::MoveWindowToContext(ContextRef::Name("Comms".into())),
            ),
            ("Ctrl + Alt + KeyR", ContextCommand::RemoveWindowFromContext),
            ("Ctrl + Alt + KeyP", ContextCommand::ToggleWindowPinned),
            (
                "Ctrl + Alt + KeyD",
                ContextCommand::DeleteContext(ContextRef::Number(3)),
            ),
            (
                "Ctrl + Alt + KeyN",
                ContextCommand::SetContextNumber {
                    context: ContextRef::Name("Comms".into()),
                    number: 4,
                },
            ),
            (
                "Ctrl + Alt + KeyF",
                ContextCommand::RemoveRecord {
                    context: ContextRef::Id(id),
                    record: RecordRef {
                        record: 0,
                        app: "Mail".into(),
                        title: "Inbox".into(),
                    },
                },
            ),
        ];
        let mut config = Config::default();
        config.keys = bindings
            .iter()
            .map(|(key, cmd)| {
                let cmd = WmCommand::ReactorCommand(ReactorCommand::Context(cmd.clone()));
                (Hotkey::from_str(key).unwrap(), cmd)
            })
            .collect();

        let json = serde_json::to_string(&PreferencesJson::from_config(&config)).unwrap();
        let prefs: PreferencesJson = serde_json::from_str(&json).unwrap();
        let applied = prefs.apply_to_config(&config);

        let mut ids: Vec<&str> = prefs.hotkeys.iter().map(|hk| hk.command_id.as_str()).collect();
        ids.sort();
        assert_eq!(
            vec![
                "add_window_to_context_2",
                "delete_context_3",
                "move_window_to_context_name_Comms",
                "previous_context",
                "remove_record_id_7_0",
                "remove_window_from_context",
                "set_context_number_name_Comms",
                "show_everything",
                "switch_context_1",
                "switch_context_2",
                "switch_context_id_7",
                "switch_context_name_Comms",
                "toggle_window_pinned",
            ],
            ids
        );
        let mut keys: Vec<(String, ContextCommand)> = applied
            .keys
            .iter()
            .map(|(hotkey, cmd)| match cmd {
                WmCommand::ReactorCommand(ReactorCommand::Context(cmd)) => {
                    (hotkey.to_string(), cmd.clone())
                }
                other => panic!("{other:?}"),
            })
            .collect();
        keys.sort_by(|a, b| a.0.cmp(&b.0));
        let mut expected: Vec<(String, ContextCommand)> =
            bindings.iter().map(|(key, cmd)| (key.to_string(), cmd.clone())).collect();
        expected.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(expected, keys);
    }

    /// The Preferences list keys its rows by command id, so two bindings of
    /// one command must not share an id. Each keeps its own id, and both
    /// keep their command and key through the round trip.
    #[test]
    fn two_bindings_of_one_command_get_distinct_command_ids() {
        let command = WmCommand::ReactorCommand(ReactorCommand::Context(
            ContextCommand::SwitchContext(ContextRef::Number(2)),
        ));
        let mut config = Config::default();
        config.keys = [
            ("Ctrl + Alt + Digit2", command.clone()),
            ("Ctrl + Alt + KeyT", command.clone()),
        ]
        .into_iter()
        .map(|(key, cmd)| (Hotkey::from_str(key).unwrap(), cmd))
        .collect();

        let json = serde_json::to_string(&PreferencesJson::from_config(&config)).unwrap();
        let prefs: PreferencesJson = serde_json::from_str(&json).unwrap();

        let ids: Vec<&str> = prefs.hotkeys.iter().map(|hk| hk.command_id.as_str()).collect();
        assert_eq!(vec!["switch_context_2", "switch_context_2#2"], ids);

        let applied = prefs.apply_to_config(&config);
        let keys: Vec<(String, ContextCommand)> = applied
            .keys
            .iter()
            .map(|(hotkey, cmd)| match cmd {
                WmCommand::ReactorCommand(ReactorCommand::Context(cmd)) => {
                    (hotkey.to_string(), cmd.clone())
                }
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            vec![
                (
                    "Ctrl + Alt + Digit2".to_string(),
                    ContextCommand::SwitchContext(ContextRef::Number(2))
                ),
                (
                    "Ctrl + Alt + KeyT".to_string(),
                    ContextCommand::SwitchContext(ContextRef::Number(2))
                ),
            ],
            keys
        );
    }

    /// The Preferences switch shows and sets
    /// `settings.experimental.contexts.enable` and leaves the other
    /// experimental settings alone.
    #[test]
    fn the_contexts_switch_follows_the_contexts_flag() {
        let mut config = Config::default();
        config.settings.experimental.contexts.enable = true;
        config.settings.experimental.scroll.enable = true;

        let json = serde_json::to_value(PreferencesJson::from_config(&config)).unwrap();
        assert_eq!(serde_json::json!(true), json["contextsEnable"]);

        let mut prefs: PreferencesJson = serde_json::from_value(json).unwrap();
        prefs.contexts_enable = false;
        let applied = prefs.apply_to_config(&config);
        assert!(!applied.settings.experimental.contexts.enable);
        assert!(applied.settings.experimental.scroll.enable);

        prefs.contexts_enable = true;
        assert!(prefs.apply_to_config(&applied).settings.experimental.contexts.enable);
    }

    #[test]
    fn test_format_hotkey_roundtrip() {
        // Parse a hotkey, format it, parse it back
        let original = Hotkey::from_str("Alt + Shift + KeyJ").unwrap();
        let formatted = format_hotkey(&original);
        assert_eq!(formatted, "⌥⇧J");

        let parsed_back = parse_hotkey_string(&formatted).unwrap();
        assert_eq!(parsed_back.to_string(), original.to_string());
    }
}
