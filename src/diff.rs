pub fn render_diff(tab_key: &str, last_action: &str, before: &str, after: &str) -> String {
    let patch = diffy::create_patch(before, after);
    format!(
        r#"# {tab_key} latest diff

Last Action: {last_action}

```diff
{patch}
```
"#
    )
}
