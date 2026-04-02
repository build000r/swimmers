pub fn exact_session_target(tmux_name: &str) -> String {
    format!("={tmux_name}")
}

pub fn exact_pane_target(tmux_name: &str) -> String {
    format!("={tmux_name}:")
}

#[cfg(test)]
mod tests {
    use super::{exact_pane_target, exact_session_target};

    #[test]
    fn exact_session_target_qualifies_numeric_names() {
        assert_eq!(exact_session_target("0"), "=0");
        assert_eq!(exact_session_target("workspace"), "=workspace");
    }

    #[test]
    fn exact_pane_target_qualifies_numeric_names() {
        assert_eq!(exact_pane_target("0"), "=0:");
        assert_eq!(exact_pane_target("workspace"), "=workspace:");
    }
}
