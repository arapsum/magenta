pub mod agent_workbench;
mod code_fence;
pub mod conversation;
mod inline_code;
mod markdown;
mod math;
mod premium_markdown;
pub mod prompt_input;
mod provider_icon;
pub mod sidebar;
pub mod titlebar;
mod visual;
pub mod workspace;

pub use provider_icon::provider_icon;

pub fn select_second_segment(checks: &[bool], second_selected: bool) -> bool {
    match (
        checks.first().copied().unwrap_or(false),
        checks.get(1).copied().unwrap_or(false),
    ) {
        (true, false) => false,
        (false, true) => true,
        (false, false) => second_selected,
        (true, true) => !second_selected,
    }
}

#[cfg(test)]
mod tests {
    use super::select_second_segment;

    #[test]
    fn segmented_pair_resolves_clicks_as_exclusive_selection() {
        assert!(!select_second_segment(&[false, false], false));
        assert!(select_second_segment(&[false, false], true));
        assert!(!select_second_segment(&[true, true], true));
        assert!(select_second_segment(&[true, true], false));
        assert!(!select_second_segment(&[true, false], true));
        assert!(select_second_segment(&[false, true], false));
    }
}
