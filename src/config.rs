// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

// Design note: Config sub-types should generally not implement Default unless
// delegating to Config::default(), which parses sugarglider.default.toml. A manual
// Default impl with hardcoded values can silently diverge from the TOML file,
// causing different behavior in code paths that don't load the config file
// (tests, first run, deserialization of saved state).

#[macro_use]
mod partial;
use std::fs::File;
use std::io::Read;
use std::ops::{Deref, Range};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use livesplit_hotkey::Hotkey;
use macro_rules_attribute::derive;
use partial::{PartialConfig, ValidationError};
use regex::{Regex, RegexBuilder};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::actor::wm_controller::WmCommand;
use crate::model::LayoutKind;

pub fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".glide")
}

pub fn restore_file() -> PathBuf {
    data_dir().join("layout.ron")
}

pub fn parked_journal_file() -> PathBuf {
    data_dir().join("parked.json")
}

pub fn contexts_file() -> PathBuf {
    data_dir().join("contexts.json")
}

pub fn config_path() -> PathBuf {
    let try_paths = default_config_paths();
    for path in &try_paths {
        if path.try_exists().unwrap_or(false) {
            return path.clone();
        }
    }
    try_paths[0].clone()
}

fn default_config_paths() -> Vec<PathBuf> {
    let home = dirs::home_dir().expect("Could not determine home directory");
    let xdg_path = home.join(".config/glide/glide.toml");
    let legacy_path = home.join(".glide.toml");

    let mut paths = vec![xdg_path.clone()];
    if legacy_path != xdg_path {
        paths.push(legacy_path);
    }
    paths
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub settings: Settings,
    pub window_rules: Vec<WindowRule>,
    pub keys: Vec<(Hotkey, WmCommand)>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
#[serde(default)]
struct ConfigPartial {
    settings: SettingsPartial,
    window_rules: Option<Vec<WindowRule>>,
    keys: Option<FxHashMap<String, WmCommandOrDisable>>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum WmCommandOrDisable {
    WmCommand(WmCommand),
    Disable(Disabled),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Disabled {
    Disable,
}

#[derive(PartialConfig!)]
#[derive_args(SettingsPartial)]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub animate: bool,
    pub default_disable: bool,
    pub mouse_follows_focus: bool,
    pub mouse_hides_on_focus: bool,
    pub focus_follows_mouse: bool,
    pub outer_gap: f64,
    pub inner_gap: f64,
    pub default_keys: bool,
    pub default_layout_kind: LayoutKind,
    #[derive_args(GroupBarsPartial)]
    pub group_bars: GroupBars,
    #[derive_args(SizeShareConfigPartial)]
    pub size_share: SizeShareConfig,
    #[derive_args(StatusIconPartial)]
    pub status_icon: StatusIcon,
    #[derive_args(DragDropConfigPartial)]
    pub drag_drop: DragDropConfig,
    #[derive_args(ExperimentalPartial)]
    pub experimental: Experimental,
}

/// A [`Regex`] sourced from config. Deserializing compiles (and thus
/// validates) the pattern, so an invalid regex surfaces as a config error at
/// parse time rather than being silently ignored later. Serializes back to the
/// original pattern string, and compares by pattern.
#[derive(Debug, Clone)]
pub struct ConfigRegex(Regex);

impl Deref for ConfigRegex {
    type Target = Regex;
    fn deref(&self) -> &Regex {
        &self.0
    }
}

impl FromStr for ConfigRegex {
    type Err = regex::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RegexBuilder::new(s).case_insensitive(true).build().map(ConfigRegex)
    }
}

impl PartialEq for ConfigRegex {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_str() == other.0.as_str()
    }
}

impl<'de> Deserialize<'de> for ConfigRegex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let pattern = String::deserialize(deserializer)?;
        pattern.parse().map_err(serde::de::Error::custom)
    }
}

impl Serialize for ConfigRegex {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.as_str())
    }
}

/// A rule that overrides how a window is managed when it is first observed,
/// based on properties of the window and its application.
///
/// Conditions are nested under `if`; all specified conditions must match
/// (logical AND) for the rule to apply. Rules are evaluated in order and the
/// first matching rule wins. See `window_rules` in the configuration.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct WindowRule {
    /// Conditions a window must satisfy for this rule to apply.
    #[serde(rename = "if", default)]
    pub conditions: WindowRuleConditions,
    /// Whether matching windows should float (`true`) or tile (`false`).
    pub float: bool,
}

/// Conditions matched against a window and its application. All specified
/// conditions must match (logical AND); omitted conditions are ignored, so an
/// empty set of conditions matches every window. All conditions match
/// case-insensitively.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Default)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct WindowRuleConditions {
    /// Application bundle identifier, matched exactly (e.g. "com.apple.Safari").
    pub app_id: Option<String>,
    /// Substring match on the application's localized name.
    pub app_name: Option<String>,
    /// Regex matched against the window title. Must be a valid regex or the
    /// config is rejected.
    pub title_regex: Option<ConfigRegex>,
    /// Literal substring match on the window title.
    pub title_substring: Option<String>,
    /// Match for the macOS Accessibility AXRole (e.g. "AXWindow").
    pub ax_role: Option<String>,
    /// Match for the macOS Accessibility AXSubrole (e.g. "AXDialog").
    pub ax_subrole: Option<String>,
}

#[derive(PartialConfig!)]
#[derive_args(ExperimentalPartial)]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct Experimental {
    #[derive_args(StatusIconExperimentalPartial)]
    pub status_icon: StatusIconExperimental,
    #[derive_args(ScrollConfigPartial)]
    pub scroll: ScrollConfig,
    #[derive_args(ContextsConfigPartial)]
    pub contexts: ContextsConfig,
}

#[derive(PartialConfig!)]
#[derive_args(ContextsConfigPartial)]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct ContextsConfig {
    /// Named window sets that the user switches between.
    pub enable: bool,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum NewWindowPlacement {
    NewColumn,
    SameColumn,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum CenterMode {
    #[default]
    Never,
    Always,
    OnOverflow,
}

#[derive(PartialConfig!)]
#[derive_args(ScrollConfigPartial)]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct ScrollConfig {
    pub enable: bool,
    pub center_focused_column: CenterMode,
    pub visible_columns: u32,
    pub column_width_presets: Vec<f64>,
    pub new_window_in_column: NewWindowPlacement,
    pub scroll_sensitivity: f64,
    pub invert_scroll_direction: bool,
    pub infinite_loop: bool,
    pub single_column_aspect_ratio: String,
}

#[derive(PartialConfig!)]
#[derive_args(DragDropConfigPartial)]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct DragDropConfig {
    /// Enable drag-to-rearrange windows.
    pub enable: bool,
    /// Show live preview of window positions while dragging.
    /// When true, other windows move in real-time to show where they'll land.
    /// When false, windows only move after releasing the mouse button.
    pub live_preview: bool,
    /// Minimum drag distance in pixels before showing drop zones.
    pub drag_threshold: f64,
    /// Width/height of edge zones as a ratio of window dimension (0.0-0.5).
    pub edge_zone_ratio: f64,
    /// Time in milliseconds to hover in an edge zone before split activates.
    pub split_dwell_ms: u64,
}

impl Default for DragDropConfig {
    fn default() -> Self {
        Config::default().settings.drag_drop
    }
}

impl DragDropConfig {
    pub fn validated(mut self) -> Self {
        self.drag_threshold = self.drag_threshold.clamp(1.0, 100.0);
        self.edge_zone_ratio = self.edge_zone_ratio.clamp(0.05, 0.4);
        self.split_dwell_ms = self.split_dwell_ms.clamp(0, 2000);
        self
    }
}

impl Default for ScrollConfig {
    fn default() -> Self {
        Config::default().settings.experimental.scroll
    }
}

impl ScrollConfig {
    pub fn validated(mut self) -> Self {
        self.visible_columns = self.visible_columns.clamp(1, 5);
        self.scroll_sensitivity = self.scroll_sensitivity.clamp(0.0, 100.0);
        self.column_width_presets.retain(|&p| p > 0.0 && p <= 1.0);
        self
    }

    pub fn aspect_ratio(&self) -> Option<AspectRatio> {
        if self.single_column_aspect_ratio.is_empty() {
            return None;
        }
        AspectRatio::from_str(&self.single_column_aspect_ratio).ok()
    }
}

#[derive(Debug, PartialEq, Clone, Copy, Serialize)]
pub struct AspectRatio {
    pub width: f64,
    pub height: f64,
}

impl FromStr for AspectRatio {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (w, h) =
            s.split_once(':').ok_or_else(|| format!("expected 'W:H' format, got {s:?}"))?;
        let width: f64 = w.trim().parse().map_err(|_| format!("invalid width: {w:?}"))?;
        let height: f64 = h.trim().parse().map_err(|_| format!("invalid height: {h:?}"))?;
        if width <= 0.0 || height <= 0.0 {
            return Err("aspect ratio values must be positive".into());
        }
        Ok(AspectRatio { width, height })
    }
}

impl<'de> Deserialize<'de> for AspectRatio {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        AspectRatio::from_str(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(PartialConfig!)]
#[derive_args(StatusIconPartial)]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct StatusIcon {
    pub enable: bool,
}

#[derive(PartialConfig!)]
#[derive_args(StatusIconExperimentalPartial)]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct StatusIconExperimental {
    pub space_index: bool,
    pub color: bool,

    #[deprecated = "Ignored; kept for compatibility."]
    pub enable: bool,
}

#[derive(PartialConfig!)]
#[derive_args(GroupBarsPartial)]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct GroupBars {
    pub enable: bool,
    pub thickness: f64,
    pub horizontal_placement: HorizontalPlacement,
    pub vertical_placement: VerticalPlacement,
}

/// How to handle size share locks that would cover more than the screen.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum SizeShareOverflow {
    /// Refuse the new lock, keeping every existing lock exact.
    #[default]
    Reject,
    /// Accept the new lock and scale all locks down until they fit.
    Squeeze,
}

#[derive(PartialConfig!)]
#[derive_args(SizeShareConfigPartial)]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct SizeShareConfig {
    pub overflow: SizeShareOverflow,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum HorizontalPlacement {
    Top,
    Bottom,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum VerticalPlacement {
    Left,
    Right,
}

impl GroupBars {
    /// Get the indicator thickness for layout space reservation
    pub fn indicator_thickness(&self) -> f64 {
        if self.enable { self.thickness } else { 0.0 }
    }
}

impl ConfigPartial {
    fn default() -> Self {
        toml::from_str(include_str!("../sugarglider.default.toml")).unwrap()
    }

    fn validate(self) -> Result<Config, SpannedError> {
        let mut keys = Vec::new();
        for (key, cmd) in self.keys.unwrap_or_default() {
            let cmd = match cmd {
                WmCommandOrDisable::WmCommand(wm_command) => wm_command,
                WmCommandOrDisable::Disable(_) => continue,
            };
            let Ok(key) = Hotkey::from_str(&key) else {
                return Err(SpannedError {
                    message: format!("Could not parse hotkey: {key}"),
                    span: None,
                });
            };
            keys.push((key, cmd));
        }
        Ok(Config {
            settings: self.settings.validate()?,
            window_rules: self.window_rules.unwrap_or_default(),
            keys,
        })
    }

    fn merge(low: Self, high: Self) -> Self {
        let include_default_keys = high.keys.is_none()
            || high.settings.default_keys.unwrap_or(Config::default().settings.default_keys);
        let mut keys = if include_default_keys {
            low.keys.unwrap_or_default()
        } else {
            Default::default()
        };
        keys.extend(high.keys.unwrap_or_default());
        Self {
            settings: SettingsPartial::merge(low.settings, high.settings),
            window_rules: high.window_rules.or(low.window_rules),
            keys: Some(keys),
        }
    }
}

impl Config {
    pub fn load(custom_path: Option<&Path>) -> anyhow::Result<Config> {
        let mut buf = String::new();
        let (mut file, path) = match custom_path {
            Some(path) => (File::open(path)?, path.to_path_buf()),
            None => {
                let mut selected: Option<(File, PathBuf)> = None;
                for path in default_config_paths() {
                    match File::open(&path) {
                        Ok(file) => {
                            selected = Some((file, path));
                            break;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(e) => return Err(e.into()),
                    }
                }
                match selected {
                    Some(pair) => pair,
                    None => return Ok(Config::default()),
                }
            }
        };
        file.read_to_string(&mut buf)?;
        Self::parse(&buf).map_err(|e| anyhow::anyhow!("{}", format_toml_error(e, &buf, &path)))
    }

    pub fn default() -> Config {
        ConfigPartial::default().validate().unwrap()
    }

    fn parse(buf: &str) -> Result<Self, SpannedError> {
        let c: ConfigPartial = toml::from_str(buf)?;
        let defaults = ConfigPartial::default();
        ConfigPartial::merge(defaults, c).validate()
    }
}

fn format_toml_error(error: SpannedError, input: &str, path: &Path) -> String {
    use annotate_snippets::{AnnotationKind, Level, Renderer, Snippet};

    let message = error.message;
    let Some(span) = error.span else {
        return format!("could not parse config: {}", message);
    };

    let snippet = Snippet::source(input)
        .path(path.to_string_lossy())
        .annotation(AnnotationKind::Primary.span(span.start..span.end).label(message));

    let report = Level::ERROR.primary_title("could not parse config").element(snippet);

    let renderer = Renderer::styled();
    format!("{}", renderer.render(&[report]))
}

#[derive(Debug)]
struct SpannedError {
    message: String,
    span: Option<Range<usize>>,
}

impl From<toml::de::Error> for SpannedError {
    fn from(e: toml::de::Error) -> Self {
        Self {
            message: e.message().to_owned(),
            span: e.span(),
        }
    }
}

impl From<ValidationError> for SpannedError {
    fn from(e: ValidationError) -> Self {
        Self {
            message: format!("{e}"),
            span: None, // TODO
        }
    }
}

/// Write preferences to the config file.
///
/// This function reads the existing config file (if present), updates the
/// relevant settings, and writes it back. It preserves user comments and
/// formatting where possible.
pub fn write_preferences_to_file(
    prefs: &crate::ui::preferences_json::PreferencesJson,
) -> anyhow::Result<PathBuf> {
    use std::fs;
    use std::io::Write;

    use toml_edit::{DocumentMut, value};

    let path = config_path();

    // Load existing config or create empty document
    let existing = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };

    let mut doc: DocumentMut = existing.parse().unwrap_or_default();

    // Ensure [settings] table exists
    if !doc.contains_key("settings") {
        doc["settings"] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    // Update settings
    doc["settings"]["animate"] = value(prefs.animate);
    doc["settings"]["focus_follows_mouse"] = value(prefs.focus_follows_mouse);
    doc["settings"]["mouse_follows_focus"] = value(prefs.mouse_follows_focus);
    doc["settings"]["outer_gap"] = value(prefs.outer_gap);
    doc["settings"]["inner_gap"] = value(prefs.inner_gap);
    doc["settings"]["default_layout_kind"] = value(&prefs.default_layout_kind);

    // Ensure [settings.status_icon] table exists
    if !doc["settings"].as_table().map_or(false, |t| t.contains_key("status_icon")) {
        doc["settings"]["status_icon"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc["settings"]["status_icon"]["enable"] = value(prefs.status_icon_enable);

    // Update window_rules as an array of tables
    let mut rules_array = toml_edit::ArrayOfTables::new();
    for rule in &prefs.window_rules {
        let mut table = toml_edit::Table::new();

        // Build the 'if' conditions table
        let mut conditions = toml_edit::Table::new();
        if let Some(ref app_id) = rule.bundle_id {
            if !app_id.is_empty() {
                conditions["app_id"] = value(app_id);
            }
        }
        if let Some(ref app_name) = rule.app_name {
            if !app_name.is_empty() {
                conditions["app_name"] = value(app_name);
            }
        }
        if !conditions.is_empty() {
            table["if"] = toml_edit::Item::Table(conditions);
        }

        table["float"] = value(rule.behavior == "float");
        rules_array.push(table);
    }

    if !rules_array.is_empty() {
        doc["window_rules"] = toml_edit::Item::ArrayOfTables(rules_array);
    } else if doc.contains_key("window_rules") {
        doc.remove("window_rules");
    }

    // Update [keys] section
    // We need to convert macOS symbol format back to TOML format
    if !prefs.hotkeys.is_empty() {
        // Build a new keys table
        let mut keys_table = toml_edit::Table::new();

        // Build command serializations from BOTH default config AND current loaded config.
        // This ensures we have serializations for all standard commands (from defaults)
        // plus any custom commands like `exec` (from the loaded config).
        let default_config = Config::default();
        let current_config = Config::load(None).unwrap_or_else(|_| Config::default());

        let mut command_serializations: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        // Add all commands from the default config first
        for (_hotkey, cmd) in &default_config.keys {
            let (_, _, command_id) = crate::ui::preferences_json::describe_command_for_toml(cmd);
            if let Ok(serialized) = serde_json::to_string(cmd) {
                command_serializations.insert(command_id, serialized);
            }
        }

        // Override/extend with commands from the current config (for custom exec commands, etc.)
        for (_hotkey, cmd) in &current_config.keys {
            let (_, _, command_id) = crate::ui::preferences_json::describe_command_for_toml(cmd);
            if let Ok(serialized) = serde_json::to_string(cmd) {
                command_serializations.insert(command_id, serialized);
            }
        }

        for hk in &prefs.hotkeys {
            // Convert macOS symbol format to TOML key format
            let toml_key = macos_symbols_to_toml_key(&hk.key);

            // Validate that the hotkey can actually be parsed before writing.
            // This prevents writing invalid keys that will fail to load on restart.
            if Hotkey::from_str(&toml_key).is_err() {
                tracing::warn!(
                    "Skipping invalid hotkey format for {}: '{}' (converted from '{}')",
                    hk.command_id,
                    toml_key,
                    hk.key
                );
                continue;
            }

            // Get the command serialization for this command_id
            if let Some(cmd_json) = command_serializations.get(&hk.command_id) {
                // Parse the command JSON and convert to TOML value
                if let Ok(cmd_value) = serde_json::from_str::<serde_json::Value>(cmd_json) {
                    let toml_value = json_to_toml_value(&cmd_value);
                    keys_table[&toml_key] = toml_value;
                }
            } else {
                tracing::warn!(
                    "Unknown command_id '{}' for hotkey '{}', skipping",
                    hk.command_id,
                    hk.key
                );
            }
        }

        if !keys_table.is_empty() {
            doc["keys"] = toml_edit::Item::Table(keys_table);
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Write atomically using a temp file
    let tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
    write!(tmp.as_file(), "{}", doc)?;
    tmp.persist(&path)?;

    Ok(path)
}

/// Convert macOS symbol hotkey format (⌥⇧H) to TOML key format (Alt + Shift + KeyH).
fn macos_symbols_to_toml_key(s: &str) -> String {
    let mut modifiers = Vec::new();
    let mut key_part = String::new();

    for c in s.chars() {
        match c {
            '⌃' => modifiers.push("Ctrl"),
            '⌥' => modifiers.push("Alt"),
            '⇧' => modifiers.push("Shift"),
            '⌘' => modifiers.push("Cmd"),
            _ => key_part.push(c),
        }
    }

    // Convert special key symbols back to names that livesplit_hotkey understands
    let key_name: String = match key_part.as_str() {
        "←" => "ArrowLeft".to_string(),
        "→" => "ArrowRight".to_string(),
        "↑" => "ArrowUp".to_string(),
        "↓" => "ArrowDown".to_string(),
        "⌫" => "Backspace".to_string(),
        "↩" => "Return".to_string(),
        "⇥" => "Tab".to_string(),
        "\\" => "Backslash".to_string(),
        "/" => "Slash".to_string(),
        "=" => "Equal".to_string(),
        "-" => "Minus".to_string(),
        "[" => "BracketLeft".to_string(),
        "]" => "BracketRight".to_string(),
        "'" => "Quote".to_string(),
        ";" => "Semicolon".to_string(),
        "," => "Comma".to_string(),
        "." => "Period".to_string(),
        "`" => "Backquote".to_string(),
        "Space" => "Space".to_string(),
        "Esc" => "Escape".to_string(),
        other => {
            // Single letters need "Key" prefix, single digits need "Digit" prefix
            if other.len() == 1 {
                let c = other.chars().next().unwrap();
                if c.is_ascii_alphabetic() {
                    format!("Key{}", c.to_ascii_uppercase())
                } else if c.is_ascii_digit() {
                    format!("Digit{}", c)
                } else {
                    other.to_string()
                }
            } else {
                other.to_string()
            }
        }
    };

    let mut parts: Vec<String> = modifiers.iter().map(|s| s.to_string()).collect();
    parts.push(key_name);
    parts.join(" + ")
}

/// Convert a JSON value to a TOML value.
fn json_to_toml_value(json: &serde_json::Value) -> toml_edit::Item {
    use toml_edit::{Item, Value, value};

    match json {
        serde_json::Value::Null => Item::None,
        serde_json::Value::Bool(b) => value(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                value(i)
            } else if let Some(f) = n.as_f64() {
                value(f)
            } else {
                Item::None
            }
        }
        serde_json::Value::String(s) => value(s.as_str()),
        serde_json::Value::Array(arr) => {
            let mut toml_arr = toml_edit::Array::new();
            for item in arr {
                if let Item::Value(v) = json_to_toml_value(item) {
                    toml_arr.push(v);
                }
            }
            Item::Value(Value::Array(toml_arr))
        }
        serde_json::Value::Object(obj) => {
            let mut table = toml_edit::InlineTable::new();
            for (k, v) in obj {
                if let Item::Value(val) = json_to_toml_value(v) {
                    table.insert(k, val);
                }
            }
            Item::Value(Value::InlineTable(table))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::layout::LayoutCommand;
    use crate::actor::reactor::Command as ReactorCommand;
    use crate::actor::wm_controller::WmCmd;

    #[test]
    fn default_config_is_valid() {
        Config::default();
    }

    #[test]
    fn the_parked_window_journal_lives_next_to_the_layout() {
        assert_eq!(
            restore_file().with_file_name("parked.json"),
            parked_journal_file()
        );
        assert_eq!(data_dir().join("parked.json"), parked_journal_file());
    }

    #[test]
    fn contexts_live_next_to_the_layout() {
        assert_eq!(data_dir().join("contexts.json"), contexts_file());
    }

    #[test]
    fn toggle_global_enabled_is_default_key() {
        let config = Config::default();
        assert!(
            config.keys.iter().any(|(hk, cmd)| {
                hk.to_string() == "Alt + KeyZ"
                    && matches!(cmd, WmCommand::Wm(WmCmd::ToggleGlobalEnabled))
            }),
            "Alt+Z should be bound to toggle_global_enabled by default"
        );
    }

    #[test]
    fn toggle_space_activated_is_ctrl_alt_z() {
        let config = Config::default();
        assert!(
            config.keys.iter().any(|(hk, cmd)| {
                hk.to_string() == "Ctrl + Alt + KeyZ"
                    && matches!(cmd, WmCommand::Wm(WmCmd::ToggleSpaceActivated))
            }),
            "Ctrl+Alt+Z should be bound to toggle_space_activated by default"
        );
    }

    #[test]
    fn default_settings_match_unspecified_setting_values() {
        assert_eq!(Config::default().settings, Config::parse("").unwrap().settings);
    }

    #[test]
    fn size_share_bindings_parse() {
        let config = Config::default();
        let shares = config
            .keys
            .iter()
            .filter_map(|(hk, cmd)| match cmd {
                WmCommand::ReactorCommand(ReactorCommand::Layout(LayoutCommand::SetSizeShare(
                    share,
                ))) => Some((hk.to_string(), *share)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            shares.len(),
            3,
            "expected three default size share bindings: {shares:?}"
        );
        assert!(
            shares
                .iter()
                .any(|(hk, share)| hk == "Ctrl + Shift + Digit2" && share.fraction() == Some(0.5)),
            "Ctrl+Shift+2 should take half the screen: {shares:?}"
        );
        assert!(
            shares
                .iter()
                .any(|(hk, share)| hk == "Ctrl + Shift + Digit3"
                    && share.fraction() == Some(1.0 / 3.0)),
            "Ctrl+Shift+3 should take a third of the screen: {shares:?}"
        );
        assert!(
            shares
                .iter()
                .any(|(hk, share)| hk == "Ctrl + Shift + Digit4" && share.fraction() == Some(0.25)),
            "Ctrl+Shift+4 should take a quarter of the screen: {shares:?}"
        );
    }

    #[test]
    fn size_share_overflow_defaults_to_reject() {
        assert_eq!(
            Config::default().settings.size_share.overflow,
            SizeShareOverflow::Reject
        );
        assert_eq!(
            Config::parse("settings.size_share.overflow = \"squeeze\"")
                .unwrap()
                .settings
                .size_share
                .overflow,
            SizeShareOverflow::Squeeze
        );
    }

    #[test]
    fn scroll_gate_is_disabled_by_default() {
        assert!(!Config::default().settings.experimental.scroll.enable);
    }

    /// In TOML, a bare integer names a context by number, a string by name,
    /// and `{ id = 7 }` by id.
    #[test]
    fn context_commands_parse() {
        use crate::actor::reactor::{ContextCommand, ContextRef};

        let config = Config::parse(
            r#"
            [keys]
            "Ctrl + Alt + Digit1" = { switch_context = 1 }
            "Ctrl + Alt + KeyC" = { switch_context = "Comms" }
            "Ctrl + Alt + KeyI" = { switch_context = { id = 7 } }
            "Ctrl + Alt + Digit0" = "show_everything"
            "Ctrl + Alt + Tab" = "previous_context"
            "#,
        )
        .unwrap();
        let command = |key: &str| {
            config
                .keys
                .iter()
                .find(|(hotkey, _)| hotkey.to_string() == key)
                .map(|(_, cmd)| match cmd {
                    WmCommand::ReactorCommand(ReactorCommand::Context(cmd)) => cmd.clone(),
                    other => panic!("{other:?}"),
                })
                .unwrap()
        };
        let id = serde_json::from_value(serde_json::json!(7)).unwrap();
        assert_eq!(
            ContextCommand::SwitchContext(ContextRef::Number(1)),
            command("Ctrl + Alt + Digit1")
        );
        assert_eq!(
            ContextCommand::SwitchContext(ContextRef::Name("Comms".into())),
            command("Ctrl + Alt + KeyC")
        );
        assert_eq!(
            ContextCommand::SwitchContext(ContextRef::Id(id)),
            command("Ctrl + Alt + KeyI")
        );
        assert_eq!(ContextCommand::ShowEverything, command("Ctrl + Alt + Digit0"));
        assert_eq!(ContextCommand::PreviousContext, command("Ctrl + Alt + Tab"));
    }

    /// Key bindings. The default config ships the context bindings of the
    /// spec commented out, so none is bound, and the shipped lines parse
    /// once uncommented. `open_context_switcher` is left out of the parse
    /// until the switcher's command exists.
    #[test]
    fn the_default_config_ships_the_context_bindings_commented_out() {
        use crate::actor::reactor::{ContextCommand, ContextRef};

        let shipped = [
            r#"# "Ctrl + Alt + Space" = "open_context_switcher""#,
            r#"# "Ctrl + Alt + 0" = "show_everything""#,
            r#"# "Ctrl + Alt + 1" = { switch_context = 1 }"#,
            r#"# "Ctrl + Alt + 2" = { switch_context = 2 }"#,
            "# ... through 9",
            r#"# "Ctrl + Alt + Tab" = "previous_context""#,
        ]
        .join("\n");
        assert!(include_str!("../sugarglider.default.toml").contains(&shipped));
        let context_bindings = |config: &Config| -> Vec<(String, ContextCommand)> {
            let mut bindings: Vec<_> = config
                .keys
                .iter()
                .filter_map(|(hotkey, cmd)| match cmd {
                    WmCommand::ReactorCommand(ReactorCommand::Context(cmd)) => {
                        Some((hotkey.to_string(), cmd.clone()))
                    }
                    _ => None,
                })
                .collect();
            bindings.sort_by(|a, b| a.0.cmp(&b.0));
            bindings
        };
        assert!(context_bindings(&Config::default()).is_empty());

        let uncommented: Vec<&str> = shipped
            .lines()
            .filter(|line| line.starts_with("# \"") && !line.contains("open_context_switcher"))
            .map(|line| &line[2..])
            .collect();
        let config = Config::parse(&format!("[keys]\n{}", uncommented.join("\n"))).unwrap();
        assert_eq!(
            vec![
                ("Ctrl + Alt + Digit0".to_string(), ContextCommand::ShowEverything),
                (
                    "Ctrl + Alt + Digit1".to_string(),
                    ContextCommand::SwitchContext(ContextRef::Number(1))
                ),
                (
                    "Ctrl + Alt + Digit2".to_string(),
                    ContextCommand::SwitchContext(ContextRef::Number(2))
                ),
                ("Ctrl + Alt + Tab".to_string(), ContextCommand::PreviousContext),
            ],
            context_bindings(&config)
        );
    }

    /// R28.
    #[test]
    fn contexts_are_off_by_default_and_turn_on_with_their_flag() {
        assert!(!Config::default().settings.experimental.contexts.enable);
        let config = Config::parse("settings.experimental.contexts.enable = true").unwrap();
        assert!(config.settings.experimental.contexts.enable);
        assert!(Config::parse("settings.experimental.contexts.scope = \"global\"").is_err());
    }

    #[test]
    fn window_rules_are_empty_by_default() {
        assert!(Config::default().window_rules.is_empty());
    }

    #[test]
    fn window_rules_parse() {
        let config = Config::parse(
            r#"
            window_rules = [
              { if = { app_id = "com.example.X", title_regex = "Dialog" }, float = true },
              { if = { title_substring = "Preferences", ax_subrole = "AXDialog" }, float = true },
            ]
            "#,
        )
        .unwrap();
        assert_eq!(
            config.window_rules,
            vec![
                WindowRule {
                    conditions: WindowRuleConditions {
                        app_id: Some("com.example.X".into()),
                        title_regex: Some("Dialog".parse().unwrap()),
                        ..Default::default()
                    },
                    float: true,
                },
                WindowRule {
                    conditions: WindowRuleConditions {
                        title_substring: Some("Preferences".into()),
                        ax_subrole: Some("AXDialog".into()),
                        ..Default::default()
                    },
                    float: true,
                },
            ]
        );
    }

    #[test]
    fn window_rules_parse_with_dotted_if_keys() {
        // The `if.app_id` dotted-key form from the aerospace-style API.
        let config = Config::parse(
            r#"
            [[window_rules]]
            if.app_id = "com.example.X"
            float = true
            "#,
        )
        .unwrap();
        assert_eq!(
            config.window_rules,
            vec![WindowRule {
                conditions: WindowRuleConditions {
                    app_id: Some("com.example.X".into()),
                    ..Default::default()
                },
                float: true,
            }]
        );
    }

    #[test]
    fn window_rule_invalid_regex_is_rejected() {
        let err = Config::parse(
            r#"
            window_rules = [{ if = { title_regex = "(unterminated" }, float = true }]
            "#,
        )
        .unwrap_err();
        assert!(
            err.message.contains("regex"),
            "unexpected error: {}",
            err.message
        );
    }

    #[test]
    fn default_keys_exclude_scroll_experimental_commands() {
        let config = Config::default();
        assert!(!config.keys.iter().any(|(_, cmd)| {
            matches!(
                cmd,
                WmCommand::ReactorCommand(ReactorCommand::Layout(
                    LayoutCommand::ChangeLayoutKind
                        | LayoutCommand::ToggleColumnTabbed
                        | LayoutCommand::CycleColumnWidth
                ))
            )
        }));
    }

    #[test]
    fn default_keys_false_excludes_default_bindings() {
        let config = Config::parse(
            r#"
            [settings]
            default_keys = false

            [keys]
            "Alt + Q" = "debug"
            "#,
        )
        .unwrap();

        // Should only have our custom key, not the defaults
        assert_eq!(config.keys.len(), 1);
        let (hotkey, _cmd) = &config.keys[0];
        assert_eq!(hotkey.to_string(), "Alt + KeyQ");
    }

    #[test]
    fn default_keys_true_includes_default_bindings() {
        let config = Config::parse(
            r#"
            [settings]
            default_keys = true

            [keys]
            "Alt + Q" = "debug"
            "#,
        )
        .unwrap();

        // Should have default keys plus our custom key
        let default_key_count = Config::default().keys.len();
        assert_eq!(config.keys.len(), default_key_count + 1);

        // Our custom key should be present
        assert!(config.keys.iter().any(|(hk, _)| hk.to_string() == "Alt + KeyQ"));
    }

    #[test]
    fn missing_keys_section_includes_default_bindings() {
        let config = Config::parse(
            r#"
            [settings]
            animate = false
            "#,
        )
        .unwrap();

        assert_eq!(config.keys.len(), Config::default().keys.len());
    }

    #[test]
    fn disable_removes_key_binding() {
        let config = Config::parse(
            r#"
            [settings]
            default_keys = false

            [keys]
            "Alt + Q" = "debug"
            "Alt + W" = "disable"
            "#,
        )
        .unwrap();

        // "disable" key should not appear in final config
        assert_eq!(config.keys.len(), 1);
        assert!(config.keys.iter().any(|(hk, _)| hk.to_string() == "Alt + KeyQ"));
        assert!(!config.keys.iter().any(|(hk, _)| hk.to_string() == "Alt + KeyW"));
    }

    #[test]
    fn disable_can_override_default_key() {
        // First verify Alt+T exists in defaults
        let default_config = Config::default();
        assert!(
            default_config.keys.iter().any(|(hk, _)| hk.to_string() == "Alt + KeyT"),
            "Alt+T should be a default key binding"
        );

        let config = Config::parse(
            r#"
            [settings]
            default_keys = true

            [keys]
            "Alt + T" = "disable"
            "#,
        )
        .unwrap();

        // Alt+T should be removed even though it's in defaults
        assert!(!config.keys.iter().any(|(hk, _)| hk.to_string() == "Alt + KeyT"));
        // But other default keys should still be present
        assert!(config.keys.iter().any(|(hk, _)| hk.to_string() == "Alt + Slash"));
    }

    #[test]
    fn exec_cmd_options_parse() {
        let config = Config::parse(
            r#"
            [settings]
            default_keys = false

            [keys]
            "Alt + Q" = { exec = ["bash", "-c", "echo hi"] }
            "Alt + W" = { exec = { cmd = ["bash", "-c", "echo hi"], unsafe_privileged = true } }
            "#,
        )
        .unwrap();

        let cmd_q = &config
            .keys
            .iter()
            .find(|(hk, _)| hk.to_string() == "Alt + KeyQ")
            .expect("Alt + KeyQ should be present")
            .1;
        let WmCommand::Wm(WmCmd::Exec(exec_cmd)) = cmd_q else {
            panic!("Expected exec command; got {cmd_q:?}");
        };
        assert_eq!(
            exec_cmd.clone().normalize(),
            crate::actor::wm_controller::NormalizedExecCmd {
                cmd_args: vec!["bash".to_owned(), "-c".to_owned(), "echo hi".to_owned()],
                unsafe_privileged: false,
            }
        );

        let cmd_w = &config
            .keys
            .iter()
            .find(|(hk, _)| hk.to_string() == "Alt + KeyW")
            .expect("Alt + KeyW should be present")
            .1;
        let WmCommand::Wm(WmCmd::Exec(exec_cmd)) = cmd_w else {
            panic!("Expected exec command; got {cmd_w:?}");
        };
        assert_eq!(
            exec_cmd.clone().normalize(),
            crate::actor::wm_controller::NormalizedExecCmd {
                cmd_args: vec!["bash".to_owned(), "-c".to_owned(), "echo hi".to_owned()],
                unsafe_privileged: true,
            }
        );
    }

    #[test]
    fn aspect_ratio_from_str_valid() {
        let ar = AspectRatio::from_str("16:9").unwrap();
        assert_eq!(ar.width, 16.0);
        assert_eq!(ar.height, 9.0);
    }

    #[test]
    fn aspect_ratio_from_str_with_spaces() {
        let ar = AspectRatio::from_str(" 4 : 3 ").unwrap();
        assert_eq!(ar.width, 4.0);
        assert_eq!(ar.height, 3.0);
    }

    #[test]
    fn aspect_ratio_from_str_invalid() {
        assert!(AspectRatio::from_str("16x9").is_err());
        assert!(AspectRatio::from_str("0:9").is_err());
        assert!(AspectRatio::from_str("16:-1").is_err());
        assert!(AspectRatio::from_str("abc:def").is_err());
    }

    #[test]
    fn arrow_keys_parse_correctly() {
        let config = Config::parse(
            r#"
            [settings]
            default_keys = false

            [keys]
            "Alt + ArrowLeft" = { move_focus = "left" }
            "Alt + ArrowDown" = { move_focus = "down" }
            "Alt + ArrowUp" = { move_focus = "up" }
            "Alt + ArrowRight" = { move_focus = "right" }
            "#,
        )
        .unwrap();

        // Should have all 4 arrow key bindings
        assert_eq!(config.keys.len(), 4);

        // Verify all arrow keys are present
        assert!(config.keys.iter().any(|(hk, _)| hk.to_string() == "Alt + ArrowLeft"));
        assert!(config.keys.iter().any(|(hk, _)| hk.to_string() == "Alt + ArrowDown"));
        assert!(config.keys.iter().any(|(hk, _)| hk.to_string() == "Alt + ArrowUp"));
        assert!(config.keys.iter().any(|(hk, _)| hk.to_string() == "Alt + ArrowRight"));
    }

    #[test]
    fn clean_up_space_is_default_key() {
        let config = Config::default();
        assert!(
            config.keys.iter().any(|(hk, cmd)| {
                hk.to_string() == "Alt + Shift + KeyC"
                    && matches!(
                        cmd,
                        WmCommand::ReactorCommand(ReactorCommand::Layout(
                            LayoutCommand::CleanUpSpace
                        ))
                    )
            }),
            "Alt+Shift+C should be bound to clean_up_space by default"
        );
    }

    #[test]
    fn macos_symbols_to_toml_key_converts_letters() {
        // Single letters should be converted to "Key" + uppercase
        assert_eq!(macos_symbols_to_toml_key("⌥H"), "Alt + KeyH");
        assert_eq!(macos_symbols_to_toml_key("⌥⇧J"), "Alt + Shift + KeyJ");
        assert_eq!(macos_symbols_to_toml_key("⌃⌥K"), "Ctrl + Alt + KeyK");

        // Converted keys should be parseable
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌥H")).is_ok());
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌥⇧J")).is_ok());
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌃⌥K")).is_ok());
    }

    #[test]
    fn macos_symbols_to_toml_key_converts_digits() {
        assert_eq!(macos_symbols_to_toml_key("⌥1"), "Alt + Digit1");
        assert_eq!(macos_symbols_to_toml_key("⌥⇧0"), "Alt + Shift + Digit0");

        // Converted keys should be parseable
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌥1")).is_ok());
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌥⇧0")).is_ok());
    }

    #[test]
    fn macos_symbols_to_toml_key_converts_arrows() {
        assert_eq!(macos_symbols_to_toml_key("⌥←"), "Alt + ArrowLeft");
        assert_eq!(macos_symbols_to_toml_key("⌥→"), "Alt + ArrowRight");
        assert_eq!(macos_symbols_to_toml_key("⌥↑"), "Alt + ArrowUp");
        assert_eq!(macos_symbols_to_toml_key("⌥↓"), "Alt + ArrowDown");

        // All should be parseable
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌥←")).is_ok());
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌥→")).is_ok());
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌥↑")).is_ok());
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌥↓")).is_ok());
    }

    #[test]
    fn macos_symbols_to_toml_key_converts_special_keys() {
        assert_eq!(macos_symbols_to_toml_key("⌥\\"), "Alt + Backslash");
        assert_eq!(macos_symbols_to_toml_key("⌥/"), "Alt + Slash");
        assert_eq!(macos_symbols_to_toml_key("⌥="), "Alt + Equal");
        assert_eq!(macos_symbols_to_toml_key("⌥Space"), "Alt + Space");

        // All should be parseable
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌥\\")).is_ok());
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌥/")).is_ok());
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌥=")).is_ok());
        assert!(Hotkey::from_str(&macos_symbols_to_toml_key("⌥Space")).is_ok());
    }

    #[test]
    fn macos_symbols_without_modifiers_still_parseable() {
        // Note: livesplit_hotkey accepts keys without modifiers (like "KeyH")
        // but these will be rejected by parse_hotkey_string in preferences_json.rs
        // which requires at least one modifier for a valid hotkey
        let toml_key = macos_symbols_to_toml_key("H");
        assert_eq!(toml_key, "KeyH");
        // This parses successfully with livesplit_hotkey
        assert!(Hotkey::from_str(&toml_key).is_ok());
    }

    #[test]
    fn parse_hotkey_string_requires_modifiers() {
        // parse_hotkey_string (in preferences_json.rs) requires at least one modifier
        // This test verifies that behavior through the public API
        use crate::ui::preferences_json::PreferencesJson;

        // Create a config with default keys
        let config = Config::default();
        let prefs_json = PreferencesJson::from_config(&config);

        // Verify all default hotkeys have modifiers (contain a modifier symbol)
        for hk in &prefs_json.hotkeys {
            assert!(
                hk.key.contains('⌥')
                    || hk.key.contains('⌃')
                    || hk.key.contains('⇧')
                    || hk.key.contains('⌘'),
                "Hotkey '{}' for command '{}' should have at least one modifier",
                hk.key,
                hk.command_id
            );
        }
    }
}
