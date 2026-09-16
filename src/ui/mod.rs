// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! UI components.

pub mod group_bar;
pub mod permission_flow;
pub mod preferences_json;
pub mod status_bar;
pub mod swift_bridge;

pub use group_bar::{Color, GroupDisplayData, GroupIndicatorNSView, GroupKind, IndicatorConfig};
pub use swift_bridge::{
    DropZone, get_current_config, hide_drop_zones, hide_preferences, init as init_swift_bridge,
    set_config_update_sender, set_current_config, show_drop_zones, show_preferences,
};
