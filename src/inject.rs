use std::fs;

use anyhow::Context;

use crate::paths::WorkspacePaths;

const SKILL_CONTENTS: &str = include_str!("../skills/preprint-browser/SKILL.md");

pub fn write_skill_file(paths: &WorkspacePaths) -> anyhow::Result<()> {
    let dest = paths.skill_md();
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create skill directory {}", parent.display())
        })?;
    }
    fs::write(&dest, SKILL_CONTENTS)
        .with_context(|| format!("failed to write skill file {}", dest.display()))
}
