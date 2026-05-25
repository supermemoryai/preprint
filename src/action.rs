use anyhow::Context;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Goto(String),
    Snapshot,
    Click(String),
    Fill { reference: String, text: String },
    Type { reference: String, text: String },
    Press(String),
    WaitText(String),
    WaitUrl(String),
    WaitIdle,
    Scroll { direction: String, amount: u32 },
    Back,
    Reload,
    Screenshot { name: Option<String>, annotate: bool },
    RecordStart { name: Option<String> },
    RecordStop,
}

impl Action {
    pub fn parse(input: &str) -> anyhow::Result<Self> {
        let input = input.trim().trim_end_matches(';').trim();
        if input.is_empty() {
            anyhow::bail!("empty action");
        }

        let open = input
            .find('(')
            .ok_or_else(|| anyhow::anyhow!("action must look like name(args): {input}"))?;
        let close = input
            .rfind(')')
            .ok_or_else(|| anyhow::anyhow!("action must end with ')': {input}"))?;

        if close < open {
            anyhow::bail!("malformed action: {input}");
        }

        let name = input[..open].trim();
        let trailing = input[close + 1..].trim();
        if !trailing.is_empty() {
            anyhow::bail!("unexpected text after action: {trailing}");
        }

        let args = parse_args(&input[open + 1..close])?;

        match name {
            "goto" => {
                expect_len(name, &args, 1)?;
                Ok(Self::Goto(args[0].clone()))
            }
            "snapshot" => {
                expect_len(name, &args, 0)?;
                Ok(Self::Snapshot)
            }
            "click" => {
                expect_len(name, &args, 1)?;
                Ok(Self::Click(normalize_ref(&args[0])))
            }
            "fill" => {
                expect_len(name, &args, 2)?;
                Ok(Self::Fill {
                    reference: normalize_ref(&args[0]),
                    text: args[1].clone(),
                })
            }
            "type" => {
                expect_len(name, &args, 2)?;
                Ok(Self::Type {
                    reference: normalize_ref(&args[0]),
                    text: args[1].clone(),
                })
            }
            "press" => {
                expect_len(name, &args, 1)?;
                Ok(Self::Press(args[0].clone()))
            }
            "wait_text" => {
                expect_len(name, &args, 1)?;
                Ok(Self::WaitText(args[0].clone()))
            }
            "wait_url" => {
                expect_len(name, &args, 1)?;
                Ok(Self::WaitUrl(args[0].clone()))
            }
            "wait_idle" => {
                expect_len(name, &args, 0)?;
                Ok(Self::WaitIdle)
            }
            "scroll" => {
                expect_len(name, &args, 2)?;
                let amount = args[1]
                    .parse::<u32>()
                    .with_context(|| format!("scroll amount must be an integer: {}", args[1]))?;
                Ok(Self::Scroll {
                    direction: args[0].clone(),
                    amount,
                })
            }
            "back" => {
                expect_len(name, &args, 0)?;
                Ok(Self::Back)
            }
            "reload" => {
                expect_len(name, &args, 0)?;
                Ok(Self::Reload)
            }
            "screenshot" => {
                if args.len() > 2 {
                    anyhow::bail!(
                        "screenshot expects up to 2 arguments (name, annotate), got {}",
                        args.len()
                    );
                }
                let name = args.first().and_then(|raw| parse_capture_name(raw));
                let annotate = match args.get(1).map(String::as_str) {
                    None => false,
                    Some("annotate") => true,
                    Some("plain") | Some("") => false,
                    Some(other) => anyhow::bail!(
                        "screenshot type must be `annotate` or `plain`, got `{other}`"
                    ),
                };
                Ok(Self::Screenshot { name, annotate })
            }
            "record_start" => {
                if args.len() > 1 {
                    anyhow::bail!(
                        "record_start expects up to 1 argument (name), got {}",
                        args.len()
                    );
                }
                let name = args.first().and_then(|raw| parse_capture_name(raw));
                Ok(Self::RecordStart { name })
            }
            "record_stop" => {
                expect_len(name, &args, 0)?;
                Ok(Self::RecordStop)
            }
            _ => anyhow::bail!("unsupported action: {name}"),
        }
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Goto(url) => write!(formatter, "goto({})", quote(url)),
            Self::Snapshot => write!(formatter, "snapshot()"),
            Self::Click(reference) => write!(formatter, "click({reference})"),
            Self::Fill { reference, text } => {
                write!(formatter, "fill({reference}, {})", quote(text))
            }
            Self::Type { reference, text } => {
                write!(formatter, "type({reference}, {})", quote(text))
            }
            Self::Press(key) => write!(formatter, "press({})", quote(key)),
            Self::WaitText(text) => write!(formatter, "wait_text({})", quote(text)),
            Self::WaitUrl(pattern) => write!(formatter, "wait_url({})", quote(pattern)),
            Self::WaitIdle => write!(formatter, "wait_idle()"),
            Self::Scroll { direction, amount } => {
                write!(formatter, "scroll({direction}, {amount})")
            }
            Self::Back => write!(formatter, "back()"),
            Self::Reload => write!(formatter, "reload()"),
            Self::Screenshot { name, annotate } => {
                let name_slot = name.as_deref().unwrap_or("_");
                if *annotate {
                    write!(formatter, "screenshot({name_slot}, annotate)")
                } else if name.is_some() {
                    write!(formatter, "screenshot({name_slot})")
                } else {
                    write!(formatter, "screenshot()")
                }
            }
            Self::RecordStart { name } => match name {
                Some(name) => write!(formatter, "record_start({name})"),
                None => write!(formatter, "record_start()"),
            },
            Self::RecordStop => write!(formatter, "record_stop()"),
        }
    }
}

pub fn first_pending_action(markdown: &str, marker: &str) -> anyhow::Result<Option<String>> {
    let Some(marker_start) = markdown.find(marker) else {
        return Ok(None);
    };
    let pending = &markdown[marker_start + marker.len()..];
    let lines = pending
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("<!--"))
        .collect::<Vec<_>>();

    if lines.is_empty() {
        return Ok(None);
    }

    if lines.len() > 1 {
        anyhow::bail!("only one action can be appended at a time");
    }

    Ok(Some(lines[0].to_string()))
}

fn parse_args(input: &str) -> anyhow::Result<Vec<String>> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let mut args = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_quote = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if !in_quote => {
                in_quote = true;
            }
            '"' if in_quote => {
                in_quote = false;
            }
            '\\' if in_quote => {
                let next = chars
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("dangling escape in quoted argument"))?;
                current.push(match next {
                    'n' => '\n',
                    't' => '\t',
                    '"' => '"',
                    '\\' => '\\',
                    other => other,
                });
            }
            ',' if !in_quote => {
                args.push(clean_arg(&current));
                current.clear();
            }
            other => current.push(other),
        }
    }

    if in_quote {
        anyhow::bail!("unterminated quoted argument");
    }

    args.push(clean_arg(&current));
    Ok(args)
}

fn clean_arg(input: &str) -> String {
    input.trim().to_string()
}

fn normalize_ref(input: &str) -> String {
    input.trim().trim_start_matches('@').to_string()
}

fn parse_capture_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "_" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn expect_len(name: &str, args: &[String], expected: usize) -> anyhow::Result<()> {
    if args.len() == expected {
        Ok(())
    } else {
        anyhow::bail!("{name} expects {expected} argument(s), got {}", args.len())
    }
}

fn quote(input: &str) -> String {
    let escaped = input.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fill() {
        assert_eq!(
            Action::parse(r#"fill(e5, "person@example.com")"#).unwrap(),
            Action::Fill {
                reference: "e5".to_string(),
                text: "person@example.com".to_string()
            }
        );
    }

    #[test]
    fn extracts_single_pending_action() {
        let doc = "## Actions\n<!-- preprint:actions -->\nclick(e3)\n";
        assert_eq!(
            first_pending_action(doc, "<!-- preprint:actions -->")
                .unwrap()
                .unwrap(),
            "click(e3)"
        );
    }

    #[test]
    fn parses_screenshot_variants() {
        assert_eq!(
            Action::parse("screenshot()").unwrap(),
            Action::Screenshot { name: None, annotate: false }
        );
        assert_eq!(
            Action::parse("screenshot(login)").unwrap(),
            Action::Screenshot {
                name: Some("login".to_string()),
                annotate: false,
            }
        );
        assert_eq!(
            Action::parse("screenshot(login, annotate)").unwrap(),
            Action::Screenshot {
                name: Some("login".to_string()),
                annotate: true,
            }
        );
        assert_eq!(
            Action::parse("screenshot(_, annotate)").unwrap(),
            Action::Screenshot { name: None, annotate: true }
        );
    }

    #[test]
    fn rejects_unknown_screenshot_type() {
        assert!(Action::parse("screenshot(login, weird)").is_err());
    }

    #[test]
    fn parses_record_variants() {
        assert_eq!(
            Action::parse("record_start()").unwrap(),
            Action::RecordStart { name: None }
        );
        assert_eq!(
            Action::parse("record_start(demo)").unwrap(),
            Action::RecordStart { name: Some("demo".to_string()) }
        );
        assert_eq!(
            Action::parse("record_stop()").unwrap(),
            Action::RecordStop
        );
    }

    #[test]
    fn screenshot_display_round_trips() {
        let cases = [
            Action::Screenshot { name: None, annotate: false },
            Action::Screenshot { name: Some("foo".to_string()), annotate: false },
            Action::Screenshot { name: Some("foo".to_string()), annotate: true },
            Action::Screenshot { name: None, annotate: true },
            Action::RecordStart { name: None },
            Action::RecordStart { name: Some("demo".to_string()) },
            Action::RecordStop,
        ];
        for action in cases {
            let rendered = action.to_string();
            let parsed = Action::parse(&rendered)
                .unwrap_or_else(|err| panic!("failed to round-trip `{rendered}`: {err}"));
            assert_eq!(parsed, action, "round-trip mismatch for `{rendered}`");
        }
    }
}
