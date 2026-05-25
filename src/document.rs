use crate::agent_browser::BrowserSnapshot;
use crate::paths::ACTION_MARKER;
use crate::session::SessionState;

pub fn render_page(session: &SessionState, snapshot: &BrowserSnapshot) -> String {
    let url = snapshot
        .origin
        .as_deref()
        .unwrap_or(session.current_url.as_str());
    let page = normalize_snapshot(&snapshot.snapshot);
    let recording_line = match &session.recording {
        Some(state) => format!("Recording active: {} ({})\n", state.name, state.path.display()),
        None => String::new(),
    };
    let profile_display = session
        .profile_name
        .as_deref()
        .unwrap_or("none");

    format!(
        r#"# {tab_key}

URL: {url}
Initial host: {initial_host}
Tab: {tab_id}
Session: {session_name}
Profile: {profile_display}
Context: {context}
Status: ready
{recording_line}
## Page

{page}

## Last Action

{last_action}

## Actions

Append one command below. The daemon owns every other section.

{marker}
"#,
        tab_key = session.tab_key,
        url = url,
        initial_host = session.initial_host,
        tab_id = session.tab_id,
        session_name = session.session,
        profile_display = profile_display,
        context = session.context,
        recording_line = recording_line,
        page = page,
        last_action = session.last_action,
        marker = ACTION_MARKER
    )
}

pub fn render_error_page(_session: &SessionState, previous_markdown: &str, error: &str) -> String {
    let before_last_action = previous_markdown
        .split("## Last Action")
        .next()
        .unwrap_or(previous_markdown)
        .trim_end();

    format!(
        "{before_last_action}\n\n## Last Action\n\nerror {error}\n\n## Actions\n\nAppend one command below. The daemon owns every other section.\n\n{ACTION_MARKER}\n"
    )
}

fn normalize_snapshot(snapshot: &str) -> String {
    let snapshot = snapshot.trim();
    if snapshot.is_empty() {
        "_No accessible page content was returned._".to_string()
    } else {
        snapshot.to_string()
    }
}

pub fn render_tabs_manifest(sessions: &[SessionState]) -> String {
    let mut output = format!("# Tabs\n\nStatus: {} open\n", sessions.len());

    for session in sessions {
        let profile_display = session
            .profile_name
            .as_deref()
            .unwrap_or("none");
        output.push_str(&format!(
            r#"
## tab: {tab_key}
- file: preprint/{tab_key}.md
- tab_id: {tab_id}
- initial_host: {initial_host}
- session: {session_name}
- profile: {profile_display}
- current_url: {current_url}
- opened: {opened}
- context: {context}
"#,
            tab_key = session.tab_key,
            tab_id = session.tab_id,
            initial_host = session.initial_host,
            session_name = session.session,
            profile_display = profile_display,
            current_url = single_line(&session.current_url),
            opened = session.opened_at,
            context = single_line(&session.context),
        ));
    }

    output
}

fn single_line(input: &str) -> String {
    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_tabs_manifest_schema() {
        let sessions = vec![SessionState::new(
            "gmail.com-t1.default".to_string(),
            "t1".to_string(),
            "gmail.com".to_string(),
            "default".to_string(),
            "https://gmail.com".to_string(),
            "https://mail.google.com/mail/u/0/#inbox".to_string(),
            "compose marketing email".to_string(),
            "2026-05-11T15:24:00Z".to_string(),
            Some("Profile 1".to_string()),
        )];

        let manifest = render_tabs_manifest(&sessions);

        assert!(manifest.starts_with("# Tabs\n\nStatus: 1 open\n"));
        assert!(manifest.contains("## tab: gmail.com-t1.default\n"));
        assert!(manifest.contains("- file: preprint/gmail.com-t1.default.md\n"));
        assert!(manifest.contains("- session: default\n"));
        assert!(manifest.contains("- profile: Profile 1\n"));
        assert!(manifest.contains("- tab_id: t1\n"));
        assert!(manifest.contains("- initial_host: gmail.com\n"));
        assert!(manifest.contains("- context: compose marketing email\n"));
    }
}
