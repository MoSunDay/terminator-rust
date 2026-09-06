//! Pure shortcut-routing table: key + modifier snapshot -> app action.
//!
//! Own key/modifier enums (no egui dependency) so the table stays
//! unit-testable headlessly. `None` means "pass to the terminal".

/// App-level actions triggered by keyboard shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewTab,
    SplitHorizontal,
    SplitVertical,
    SplitDefault,
    ClosePane,
    CycleFocus(bool),
    PrevTab,
    NextTab,
    FocusUp,
    FocusDown,
    FocusLeft,
    FocusRight,
    ToggleZoom,
    Respawn,
    Paste,
    Copy,
}

/// Context-menu / header actions on a specific pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneAction {
    SplitHorizontal,
    SplitVertical,
    Close,
    Respawn,
}

/// Shortcut-relevant keys (own enum: no egui dependency).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SKey {
    T,
    O,
    E,
    D,
    W,
    R,
    F,
    C,
    V,
    Tab,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
}

/// Modifier snapshot for shortcut routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SMods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Route a keypress to an app action; `None` means "pass to the terminal".
pub fn route_shortcut(m: SMods, k: SKey) -> Option<Action> {
    if !m.ctrl {
        return None;
    }
    match (m.shift, k) {
        (true, SKey::T) => Some(Action::NewTab),
        (true, SKey::O) => Some(Action::SplitHorizontal),
        (true, SKey::E) => Some(Action::SplitVertical),
        (true, SKey::D) => Some(Action::SplitDefault),
        (true, SKey::W) => Some(Action::ClosePane),
        (true, SKey::R) => Some(Action::Respawn),
        (true, SKey::F) => Some(Action::ToggleZoom),
        (true, SKey::C) => Some(Action::Copy),
        (true, SKey::V) => Some(Action::Paste),
        (true, SKey::Tab) => Some(Action::CycleFocus(false)),
        (false, SKey::Tab) => Some(Action::CycleFocus(true)),
        (true, SKey::ArrowUp) => Some(Action::FocusUp),
        (true, SKey::ArrowDown) => Some(Action::FocusDown),
        (true, SKey::ArrowLeft) => Some(Action::FocusLeft),
        (true, SKey::ArrowRight) => Some(Action::FocusRight),
        (_, SKey::PageUp) => Some(Action::PrevTab),
        (_, SKey::PageDown) => Some(Action::NextTab),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, shift: bool) -> SMods {
        SMods {
            ctrl,
            shift,
            alt: false,
        }
    }

    #[test]
    fn shortcut_table_matches_spec() {
        assert_eq!(
            route_shortcut(mods(true, true), SKey::T),
            Some(Action::NewTab)
        );
        assert_eq!(
            route_shortcut(mods(true, true), SKey::O),
            Some(Action::SplitHorizontal)
        );
        assert_eq!(
            route_shortcut(mods(true, true), SKey::E),
            Some(Action::SplitVertical)
        );
        assert_eq!(
            route_shortcut(mods(true, true), SKey::D),
            Some(Action::SplitDefault)
        );
        assert_eq!(route_shortcut(mods(true, false), SKey::D), None);
        assert_eq!(
            route_shortcut(mods(true, true), SKey::W),
            Some(Action::ClosePane)
        );
        assert_eq!(
            route_shortcut(mods(true, true), SKey::F),
            Some(Action::ToggleZoom)
        );
        assert_eq!(
            route_shortcut(mods(true, true), SKey::R),
            Some(Action::Respawn)
        );
        assert_eq!(route_shortcut(mods(false, true), SKey::T), None);
        assert_eq!(route_shortcut(mods(true, false), SKey::V), None);
        assert_eq!(
            route_shortcut(mods(true, true), SKey::V),
            Some(Action::Paste)
        );
        assert_eq!(
            route_shortcut(mods(true, false), SKey::Tab),
            Some(Action::CycleFocus(true))
        );
        assert_eq!(
            route_shortcut(mods(true, true), SKey::Tab),
            Some(Action::CycleFocus(false))
        );
        assert_eq!(
            route_shortcut(mods(true, false), SKey::PageUp),
            Some(Action::PrevTab)
        );
        assert_eq!(
            route_shortcut(mods(true, false), SKey::PageDown),
            Some(Action::NextTab)
        );
        assert_eq!(
            route_shortcut(mods(true, true), SKey::ArrowLeft),
            Some(Action::FocusLeft)
        );
        assert_eq!(
            route_shortcut(mods(true, true), SKey::ArrowDown),
            Some(Action::FocusDown)
        );
    }
}
