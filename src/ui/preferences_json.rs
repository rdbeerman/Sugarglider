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
    /// The bound command as JSON, arguments included. The Swift UI sends it
    /// back unchanged, so each binding keeps its own command.
    pub command: String,
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
                // The default hotkey of each command in the default config
                let default_keys: Vec<(serde_json::Value, String)> = Config::default()
                    .keys
                    .iter()
                    .map(|(hotkey, cmd)| (command_json(cmd), format_hotkey(hotkey)))
                    .collect();

                let mut hotkeys: Vec<_> = config
                    .keys
                    .iter()
                    .map(|(hotkey, cmd)| {
                        let command = command_json(cmd);
                        let default_key = default_keys
                            .iter()
                            .find(|(default_command, _)| *default_command == command)
                            .map(|(_, key)| key.clone());
                        HotkeyBindingJson::from_binding_with_default(hotkey, cmd, default_key)
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

        let keys = self.bindings();

        Config { settings, window_rules, keys }
    }

    /// The key bindings, each with the command it carries. A binding whose
    /// key or command doesn't parse is dropped.
    pub fn bindings(&self) -> Vec<(Hotkey, WmCommand)> {
        self.hotkeys.iter().filter_map(HotkeyBindingJson::binding).collect()
    }
}

/// A command in the form that [`HotkeyBindingJson::command`] carries and that
/// the config file stores. Two commands are the same when these are equal.
pub fn command_json(cmd: &WmCommand) -> serde_json::Value {
    serde_json::to_value(cmd).expect("commands serialize to JSON")
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
        let (description, category, sort_order) = describe_command(cmd);
        Self {
            key: format_hotkey(hotkey),
            command: command_json(cmd).to_string(),
            description,
            category,
            default_key,
            sort_order,
        }
    }

    /// The hotkey and the command of this binding, or `None` if either
    /// doesn't parse.
    pub fn binding(&self) -> Option<(Hotkey, WmCommand)> {
        let Some(hotkey) = parse_hotkey_string(&self.key) else {
            tracing::warn!(
                "Invalid hotkey format '{}' for command {}, dropping binding",
                self.key,
                self.command
            );
            return None;
        };
        match serde_json::from_str(&self.command) {
            Ok(cmd) => Some((hotkey, cmd)),
            Err(e) => {
                tracing::warn!(
                    "Invalid command {} for hotkey '{}', dropping binding: {e}",
                    self.command,
                    self.key
                );
                None
            }
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

/// Get the description, category, and sort order for a WmCommand.
fn describe_command(cmd: &WmCommand) -> (String, String, u32) {
    match cmd {
        WmCommand::Wm(wm_cmd) => match wm_cmd {
            WmCmd::ToggleGlobalEnabled => {
                ("Toggle tiling globally".to_string(), "System".to_string(), 0)
            }
            WmCmd::SetGlobalEnabled(enabled) => (
                format!("Set tiling {}", if *enabled { "on" } else { "off" }),
                "System".to_string(),
                1,
            ),
            WmCmd::ToggleSpaceActivated => (
                "Toggle tiling on current space".to_string(),
                "System".to_string(),
                2,
            ),
            WmCmd::Exec(_) => ("Execute command".to_string(), "Utilities".to_string(), 0),
        },
        WmCommand::ReactorCommand(reactor_cmd) => match reactor_cmd {
            ReactorCommand::Layout(layout_cmd) => describe_layout_command(layout_cmd),
            ReactorCommand::Metrics(metrics_cmd) => match metrics_cmd {
                MetricsCommand::ShowTiming => {
                    ("Show performance timing".to_string(), "Developer".to_string(), 0)
                }
            },
            ReactorCommand::Reactor(reactor_cmd) => match reactor_cmd {
                ReactorCmd::Debug => {
                    ("Print layout debug info".to_string(), "Developer".to_string(), 0)
                }
                ReactorCmd::Serialize => {
                    ("Serialize layout state".to_string(), "Developer".to_string(), 1)
                }
                ReactorCmd::SaveAndExit => {
                    ("Save state and exit".to_string(), "System".to_string(), 10)
                }
            },
            ReactorCommand::Context(context_cmd) => describe_context_command(context_cmd),
        },
    }
}

/// Get the description, category, and sort order for a ContextCommand.
fn describe_context_command(cmd: &ContextCommand) -> (String, String, u32) {
    let category = "Contexts".to_string();
    match cmd {
        ContextCommand::ShowEverything => ("Show every window".to_string(), category, 0),
        ContextCommand::SwitchContext(ContextRef::Number(number)) => (
            format!("Switch to context {number}"),
            category,
            u32::from(*number),
        ),
        ContextCommand::PreviousContext => {
            ("Switch to the previous context".to_string(), category, 10)
        }
        ContextCommand::SwitchContext(ContextRef::Name(name)) => {
            (format!("Switch to context \"{name}\""), category, 20)
        }
        ContextCommand::SwitchContext(ContextRef::Id(id)) => {
            (format!("Switch to context with id {}", id.get()), category, 30)
        }
        ContextCommand::AddWindowToContext(reference) => {
            let (name, order) = describe_context_ref(reference);
            (format!("Add the window to {name}"), category, 40 + order)
        }
        ContextCommand::MoveWindowToContext(reference) => {
            let (name, order) = describe_context_ref(reference);
            (format!("Move the window to {name}"), category, 60 + order)
        }
        ContextCommand::RemoveWindowFromContext => (
            "Remove the window from the active context".to_string(),
            category,
            80,
        ),
        ContextCommand::ToggleWindowPinned => ("Pin or unpin the window".to_string(), category, 81),
        ContextCommand::CreateContext(name) => (
            format!("Create context \"{name}\" from the windows on screen"),
            category,
            40,
        ),
        ContextCommand::RenameContext { context, name } => {
            let (context, order) = describe_context_ref(context);
            (format!("Rename {context} to \"{name}\""), category, 100 + order)
        }
        ContextCommand::SetContextNumber { context, number } => {
            let (context, order) = describe_context_ref(context);
            (format!("Give {context} the number {number}"), category, 120 + order)
        }
        ContextCommand::DeleteContext(context) => {
            let (context, order) = describe_context_ref(context);
            (format!("Delete {context}"), category, 140 + order)
        }
        ContextCommand::EditContextMembers { context, .. } => {
            let (context, order) = describe_context_ref(context);
            (format!("Change the windows of {context}"), category, 160 + order)
        }
        ContextCommand::RemoveRecord { context, record } => {
            let (context, order) = describe_context_ref(context);
            (
                format!("Forget member record {} of {context}", record.record),
                category,
                180 + order,
            )
        }
    }
}

/// The description and the offset in the sort order of a context that a
/// command names.
fn describe_context_ref(reference: &ContextRef) -> (String, u32) {
    match reference {
        ContextRef::Number(number) => (format!("context {number}"), u32::from(*number)),
        ContextRef::Name(name) => (format!("context \"{name}\""), 10),
        ContextRef::Id(id) => (format!("context with id {}", id.get()), 11),
    }
}

/// Get the description, category, and sort order for a LayoutCommand.
fn describe_layout_command(cmd: &LayoutCommand) -> (String, String, u32) {
    match cmd {
        LayoutCommand::MoveFocus(dir) => (
            format!("Focus {}", direction_name(dir)),
            "Focus".to_string(),
            direction_sort_order(dir),
        ),
        LayoutCommand::FocusNext => ("Focus next window".to_string(), "Focus".to_string(), 10),
        LayoutCommand::FocusPrev => ("Focus previous window".to_string(), "Focus".to_string(), 11),
        LayoutCommand::Ascend => ("Select parent container".to_string(), "Focus".to_string(), 20),
        LayoutCommand::Descend => ("Select child node".to_string(), "Focus".to_string(), 21),
        LayoutCommand::MoveNode(dir) => (
            format!("Move window {}", direction_name(dir)),
            "Move".to_string(),
            direction_sort_order(dir),
        ),
        LayoutCommand::Resize { direction, percent } => (
            format!("Resize {} by {}%", direction_name(direction), percent),
            "Resize".to_string(),
            10 + direction_sort_order(direction), // After SetSizeShare
        ),
        LayoutCommand::Split(orientation) => (
            format!("Split {}", orientation_name(orientation)),
            "Layout".to_string(),
            0,
        ),
        LayoutCommand::ToggleOrientation => {
            ("Toggle split orientation".to_string(), "Layout".to_string(), 1)
        }
        LayoutCommand::Group(orientation) => (
            format!(
                "Group {} ({})",
                orientation_name(orientation),
                group_mode_name(orientation)
            ),
            "Layout".to_string(),
            10,
        ),
        LayoutCommand::Ungroup => ("Ungroup container".to_string(), "Layout".to_string(), 11),
        LayoutCommand::ToggleFocusFloating => (
            "Toggle focus between tiled/floating".to_string(),
            "Floating".to_string(),
            0,
        ),
        LayoutCommand::ToggleWindowFloating => {
            ("Toggle window floating".to_string(), "Floating".to_string(), 1)
        }
        LayoutCommand::ToggleFullscreen => {
            ("Toggle fullscreen".to_string(), "Layout".to_string(), 20)
        }
        LayoutCommand::NextLayout => (
            "Switch to next saved layout".to_string(),
            "Layout".to_string(),
            30,
        ),
        LayoutCommand::PrevLayout => (
            "Switch to previous saved layout".to_string(),
            "Layout".to_string(),
            31,
        ),
        LayoutCommand::CycleColumnWidth => (
            "Cycle column width preset".to_string(),
            "Scroll Layout".to_string(),
            0,
        ),
        LayoutCommand::ChangeLayoutKind => (
            "Change layout mode (tree/scroll)".to_string(),
            "Scroll Layout".to_string(),
            1,
        ),
        LayoutCommand::ToggleColumnTabbed => (
            "Toggle column tabbed mode".to_string(),
            "Scroll Layout".to_string(),
            2,
        ),
        LayoutCommand::CleanUpSpace => ("Clean up space".to_string(), "System".to_string(), 5),
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
            (
                format!("Toggle window size {name}"),
                "Resize".to_string(),
                sort_order,
            )
        }
        LayoutCommand::ToggleSizeLock => (
            "Lock window at current size".to_string(),
            "Resize".to_string(),
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

fn orientation_name(orientation: &Orientation) -> &'static str {
    match orientation {
        Orientation::Horizontal => "horizontally",
        Orientation::Vertical => "vertically",
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
                command: r#"{"move_focus":"left"}"#.to_string(),
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

        let hk = parse_hotkey_string("⌥/").unwrap();
        assert_eq!(hk.to_string(), "Alt + Slash");

        let hk = parse_hotkey_string("⌥=").unwrap();
        assert_eq!(hk.to_string(), "Alt + Equal");

        // Digits
        let hk = parse_hotkey_string("⌥⇧0").unwrap();
        assert_eq!(hk.to_string(), "Alt + Shift + Digit0");

        // No modifier should fail
        assert!(parse_hotkey_string("H").is_none());
    }

    /// Key bindings. Context bindings survive the round trip through the
    /// preferences JSON, each with its own command.
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

        let mut commands: Vec<&str> = prefs.hotkeys.iter().map(|hk| hk.command.as_str()).collect();
        commands.sort();
        assert_eq!(
            vec![
                r#""previous_context""#,
                r#""remove_window_from_context""#,
                r#""show_everything""#,
                r#""toggle_window_pinned""#,
                r#"{"add_window_to_context":2}"#,
                r#"{"move_window_to_context":"Comms"}"#,
                r#"{"switch_context":"Comms"}"#,
                r#"{"switch_context":1}"#,
                r#"{"switch_context":2}"#,
                r#"{"switch_context":{"id":7}}"#,
            ],
            commands
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
