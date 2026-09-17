// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JSON types for preferences UI communication.
//!
//! This module defines types that are serialized to JSON for exchange between
//! the Swift preferences UI and the Rust backend.

use std::str::FromStr;

use livesplit_hotkey::Hotkey;
use serde::{Deserialize, Serialize};

use crate::actor::layout::LayoutCommand;
use crate::actor::reactor::{Command as ReactorCommand, ReactorCommand as ReactorCmd};
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
    pub drag_drop_window_drag: bool,
    pub drag_drop_live_preview: bool,

    // Layout settings
    pub default_layout_kind: String,

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
            drag_drop_window_drag: settings.drag_drop.window_drag,
            drag_drop_live_preview: settings.drag_drop.live_preview,
            default_layout_kind: match settings.default_layout_kind {
                LayoutKind::Tree => "tree".to_string(),
                LayoutKind::Scroll => "scroll".to_string(),
            },
            window_rules: config.window_rules.iter().map(WindowRuleJson::from_rule).collect(),
            hotkeys: {
                // Build a map of command_id -> default hotkey from the default config
                let default_config = Config::default();
                let default_keys: std::collections::HashMap<String, String> = default_config
                    .keys
                    .iter()
                    .map(|(hotkey, cmd)| {
                        let (_, _, command_id) = describe_command(cmd);
                        (command_id, format_hotkey(hotkey))
                    })
                    .collect();

                config
                    .keys
                    .iter()
                    .map(|(hotkey, cmd)| {
                        let (_, _, command_id) = describe_command(cmd);
                        let default_key = default_keys.get(&command_id).cloned();
                        HotkeyBindingJson::from_binding_with_default(hotkey, cmd, default_key)
                    })
                    .collect()
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
        settings.drag_drop.window_drag = self.drag_drop_window_drag;
        settings.drag_drop.live_preview = self.drag_drop_live_preview;
        settings.default_layout_kind = match self.default_layout_kind.as_str() {
            "scroll" => LayoutKind::Scroll,
            _ => LayoutKind::Tree,
        };

        let window_rules: Vec<WindowRule> =
            self.window_rules.iter().map(WindowRuleJson::to_rule).collect();

        // Build new keys from hotkeys, matching by command_id
        let keys = self.build_keys_from_hotkeys(config);

        Config { settings, window_rules, keys }
    }

    /// Build the keys vector from hotkeys JSON, using original commands from config.
    fn build_keys_from_hotkeys(&self, config: &Config) -> Vec<(Hotkey, WmCommand)> {
        // Create a map from command_id to the original command
        let command_map: std::collections::HashMap<String, &WmCommand> = config
            .keys
            .iter()
            .map(|(_, cmd)| {
                let (_, _, command_id) = describe_command(cmd);
                (command_id, cmd)
            })
            .collect();

        self.hotkeys
            .iter()
            .filter_map(|hk| {
                // Get the original command for this command_id
                let cmd = command_map.get(&hk.command_id)?;

                // Parse the hotkey string back to a Hotkey
                let hotkey = parse_hotkey_string(&hk.key)?;

                Some((hotkey, (*cmd).clone()))
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
    /// Create from a hotkey binding with an optional default key.
    pub fn from_binding_with_default(
        hotkey: &Hotkey,
        cmd: &WmCommand,
        default_key: Option<String>,
    ) -> Self {
        let (description, category, command_id) = describe_command(cmd);
        Self {
            key: format_hotkey(hotkey),
            command_id,
            description,
            category,
            default_key,
        }
    }
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
    describe_command(cmd)
}

/// Get the description, category, and command ID for a WmCommand.
fn describe_command(cmd: &WmCommand) -> (String, String, String) {
    match cmd {
        WmCommand::Wm(wm_cmd) => match wm_cmd {
            WmCmd::ToggleGlobalEnabled => (
                "Toggle tiling globally".to_string(),
                "System".to_string(),
                "toggle_global_enabled".to_string(),
            ),
            WmCmd::SetGlobalEnabled(enabled) => (
                format!("Set tiling {}", if *enabled { "on" } else { "off" }),
                "System".to_string(),
                "set_global_enabled".to_string(),
            ),
            WmCmd::ToggleSpaceActivated => (
                "Toggle tiling on current space".to_string(),
                "System".to_string(),
                "toggle_space_activated".to_string(),
            ),
            WmCmd::Exec(_) => (
                "Execute command".to_string(),
                "Utilities".to_string(),
                "exec".to_string(),
            ),
        },
        WmCommand::ReactorCommand(reactor_cmd) => match reactor_cmd {
            ReactorCommand::Layout(layout_cmd) => describe_layout_command(layout_cmd),
            ReactorCommand::Metrics(metrics_cmd) => match metrics_cmd {
                MetricsCommand::ShowTiming => (
                    "Show performance timing".to_string(),
                    "Developer".to_string(),
                    "show_timing".to_string(),
                ),
            },
            ReactorCommand::Reactor(reactor_cmd) => match reactor_cmd {
                ReactorCmd::Debug => (
                    "Print layout debug info".to_string(),
                    "Developer".to_string(),
                    "debug".to_string(),
                ),
                ReactorCmd::Serialize => (
                    "Serialize layout state".to_string(),
                    "Developer".to_string(),
                    "serialize".to_string(),
                ),
                ReactorCmd::SaveAndExit => (
                    "Save state and exit".to_string(),
                    "System".to_string(),
                    "save_and_exit".to_string(),
                ),
            },
        },
    }
}

/// Get the description, category, and command ID for a LayoutCommand.
fn describe_layout_command(cmd: &LayoutCommand) -> (String, String, String) {
    match cmd {
        LayoutCommand::MoveFocus(dir) => (
            format!("Focus {}", direction_name(dir)),
            "Focus".to_string(),
            format!("move_focus_{}", direction_id(dir)),
        ),
        LayoutCommand::FocusNext => (
            "Focus next window".to_string(),
            "Focus".to_string(),
            "focus_next".to_string(),
        ),
        LayoutCommand::FocusPrev => (
            "Focus previous window".to_string(),
            "Focus".to_string(),
            "focus_prev".to_string(),
        ),
        LayoutCommand::Ascend => (
            "Select parent container".to_string(),
            "Focus".to_string(),
            "ascend".to_string(),
        ),
        LayoutCommand::Descend => (
            "Select child node".to_string(),
            "Focus".to_string(),
            "descend".to_string(),
        ),
        LayoutCommand::MoveNode(dir) => (
            format!("Move window {}", direction_name(dir)),
            "Move".to_string(),
            format!("move_node_{}", direction_id(dir)),
        ),
        LayoutCommand::Resize { direction, percent } => (
            format!("Resize {} by {}%", direction_name(direction), percent),
            "Resize".to_string(),
            format!("resize_{}", direction_id(direction)),
        ),
        LayoutCommand::Split(orientation) => (
            format!("Split {}", orientation_name(orientation)),
            "Layout".to_string(),
            format!("split_{}", orientation_id(orientation)),
        ),
        LayoutCommand::ToggleOrientation => (
            "Toggle split orientation".to_string(),
            "Layout".to_string(),
            "toggle_orientation".to_string(),
        ),
        LayoutCommand::Group(orientation) => (
            format!(
                "Group {} ({})",
                orientation_name(orientation),
                group_mode_name(orientation)
            ),
            "Layout".to_string(),
            format!("group_{}", orientation_id(orientation)),
        ),
        LayoutCommand::Ungroup => (
            "Ungroup container".to_string(),
            "Layout".to_string(),
            "ungroup".to_string(),
        ),
        LayoutCommand::ToggleFocusFloating => (
            "Toggle focus between tiled/floating".to_string(),
            "Floating".to_string(),
            "toggle_focus_floating".to_string(),
        ),
        LayoutCommand::ToggleWindowFloating => (
            "Toggle window floating".to_string(),
            "Floating".to_string(),
            "toggle_window_floating".to_string(),
        ),
        LayoutCommand::ToggleFullscreen => (
            "Toggle fullscreen".to_string(),
            "Layout".to_string(),
            "toggle_fullscreen".to_string(),
        ),
        LayoutCommand::NextLayout => (
            "Switch to next saved layout".to_string(),
            "Layout".to_string(),
            "next_layout".to_string(),
        ),
        LayoutCommand::PrevLayout => (
            "Switch to previous saved layout".to_string(),
            "Layout".to_string(),
            "prev_layout".to_string(),
        ),
        LayoutCommand::CycleColumnWidth => (
            "Cycle column width preset".to_string(),
            "Scroll Layout".to_string(),
            "cycle_column_width".to_string(),
        ),
        LayoutCommand::ChangeLayoutKind => (
            "Change layout mode (tree/scroll)".to_string(),
            "Scroll Layout".to_string(),
            "change_layout_kind".to_string(),
        ),
        LayoutCommand::ToggleColumnTabbed => (
            "Toggle column tabbed mode".to_string(),
            "Scroll Layout".to_string(),
            "toggle_column_tabbed".to_string(),
        ),
        LayoutCommand::CleanUpSpace => (
            "Clean up space".to_string(),
            "System".to_string(),
            "clean_up_space".to_string(),
        ),
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
            drag_drop_window_drag: true,
            drag_drop_live_preview: true,
            default_layout_kind: "tree".to_string(),
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
