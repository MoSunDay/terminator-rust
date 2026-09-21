//! JSON model of the global [`crate::state::Settings`] plus the
//! save/load conversions. Split from the parent module so the growing
//! settings surface keeps every file inside the size budget.

use layout_tree::Axis;
use serde::{Deserialize, Serialize};

/// Persisted [`crate::state::Settings`]. Unknown keys in older files are
/// ignored; every new field defaults so pre-change files keep loading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PSettings {
    /// "v" = left/right split, "h" = top/bottom.
    pub split_axis: String,
    /// Window opacity; absent in pre-transparency state.json files.
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    /// Terminal font size (points), uniform for every window; absent in
    /// per-window-font files.
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    /// Terminal background glass (0 = opaque, 1 = see-through), uniform
    /// for all panes; absent in per-pane-glass files.
    #[serde(default)]
    pub transparency: f32,
    /// Terminal background override as "#rrggbb"; None = theme bg.
    #[serde(default)]
    pub bg: Option<String>,
}

fn default_opacity() -> f32 {
    // Fully opaque by default: without a compositor (bare X sessions)
    // transparent pixels render BLACK; transparency is opt-in via the
    // settings slider (clamped 0.5..=1.0).
    1.0
}

fn default_font_size() -> f32 {
    crate::state::DEFAULT_FONT_SIZE
}

impl Default for PSettings {
    fn default() -> Self {
        Self {
            split_axis: "v".to_string(),
            opacity: default_opacity(),
            font_size: default_font_size(),
            transparency: 0.0,
            bg: None,
        }
    }
}

fn hex_of(c: theme::Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

impl PSettings {
    pub(crate) fn of(s: &crate::state::Settings) -> Self {
        Self {
            split_axis: match s.split_axis {
                Axis::Horizontal => "h".to_string(),
                Axis::Vertical => "v".to_string(),
            },
            opacity: s.opacity,
            font_size: s.font_size,
            transparency: s.transparency,
            bg: s.bg_color.map(hex_of),
        }
    }

    pub(crate) fn to_settings(&self) -> crate::state::Settings {
        let axis = if self.split_axis == "h" {
            Axis::Horizontal
        } else {
            Axis::Vertical
        };
        let opacity = if self.opacity.is_finite() {
            self.opacity.clamp(0.5, 1.0)
        } else {
            default_opacity()
        };
        let font_size = if self.font_size.is_finite() {
            self.font_size.clamp(10.0, 24.0)
        } else {
            default_font_size()
        };
        let transparency = if self.transparency.is_finite() {
            self.transparency.clamp(0.0, 1.0)
        } else {
            0.0
        };
        // A stored bg that no longer parses (hand-edited file) degrades
        // to the theme background, never a load failure.
        let bg_color = self.bg.as_deref().and_then(|s| theme::parse_hex(s).ok());
        crate::state::Settings {
            split_axis: axis,
            opacity,
            font_size,
            transparency,
            bg_color,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Settings;

    #[test]
    fn appearance_roundtrips_through_json() {
        let s = Settings {
            font_size: 18.0,
            transparency: 0.5,
            bg_color: Some(theme::Rgb {
                r: 0x20,
                g: 0x30,
                b: 0x40,
            }),
            ..Settings::default()
        };
        let p = PSettings::of(&s);
        let json = serde_json::to_string(&p).unwrap_or_default();
        let back: PSettings = serde_json::from_str(&json).unwrap_or_else(|_| PSettings::default());
        let out = back.to_settings();
        assert_eq!(out.font_size, 18.0);
        assert!((out.transparency - 0.5).abs() < 1e-6);
        assert_eq!(
            out.bg_color,
            Some(theme::Rgb {
                r: 0x20,
                g: 0x30,
                b: 0x40
            })
        );
    }

    #[test]
    fn appearance_clamps_out_of_range() {
        let p = PSettings {
            font_size: 99.0,
            transparency: 7.0,
            bg: Some("not-a-color".to_string()),
            ..PSettings::default()
        };
        let s = p.to_settings();
        assert_eq!(s.font_size, 24.0);
        assert_eq!(s.transparency, 1.0);
        assert_eq!(s.bg_color, None, "unparseable bg degrades to None");
    }

    #[test]
    fn legacy_settings_keys_default() {
        // A pre-unification settings block (no font/glass/bg keys) loads
        // with defaults; the old per-pane bg/transparency lived in PMeta,
        // whose unknown keys serde ignores on load (covered in the
        // parent module's full-document tests). The removed
        // "split_ratio" key (splits are always equal now) is likewise an
        // unknown field that serde silently ignores in old state.json
        // files - kept here as a load-compat pin.
        let p: PSettings = serde_json::from_str(r#"{"split_axis":"v","split_ratio":0.5}"#).unwrap();
        assert_eq!(p.split_axis, "v");
        assert_eq!(p.font_size, crate::state::DEFAULT_FONT_SIZE);
        assert_eq!(p.transparency, 0.0);
        assert_eq!(p.bg, None);
    }
}
