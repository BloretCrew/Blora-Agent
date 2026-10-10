// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Agent-mode learning loop: the agent maintains its own skills and searches
//! earlier sessions.

use std::path::{Path, PathBuf};

use blora_events::{KnownPayload, SkillChanged};
use blora_types::{BloraError, Result, RunId, SessionId, TurnId};

use crate::Runtime;

/// Largest skill body the agent may write.
pub const MAX_SKILL_BYTES: usize = 24_000;

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
        && name.len() <= 64
}

fn skills_root(workspace: &str, scope: &str) -> Result<PathBuf> {
    match scope {
        "workspace" => Ok(Path::new(workspace).join(".blora/skills")),
        "global" => {
            let home = std::env::var_os("BLORA_HOME")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|home| home.join(".blora")))
                .ok_or_else(|| BloraError::Other("no home directory for global skills".to_owned()))?;
            Ok(home.join("skills"))
        }
        other => Err(BloraError::Other(format!(
            "unknown skill scope {other}; use workspace or global"
        ))),
    }
}

/// Existing file for `name` under `root`: `<name>.md` or `<name>/SKILL.md`.
fn existing(root: &Path, name: &str) -> Option<PathBuf> {
    [root.join(format!("{name}.md")), root.join(name).join("SKILL.md")]
        .into_iter()
        .find(|path| path.is_file())
}

impl Runtime {
    /// Create, update, or delete a skill and record a `skill.changed` event.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn manage_skill(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        workspace: &str,
        action: &str,
        name: &str,
        content: Option<&str>,
        scope: &str,
    ) -> Result<String> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(BloraError::Other(format!(
                "invalid skill name {name:?}: use lowercase letters, digits, - or _ (max 64)"
            )));
        }
        let root = skills_root(workspace, scope)?;
        let current = existing(&root, name);
        let previous = current
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok());
        let path = match action {
            "create" | "update" => {
                let content = content.map(str::trim).unwrap_or_default();
                if content.is_empty() {
                    return Err(BloraError::Other(format!("{action} needs content")));
                }
                if content.len() > MAX_SKILL_BYTES {
                    return Err(BloraError::Other(format!(
                        "skill content exceeds {MAX_SKILL_BYTES} bytes; keep skills focused"
                    )));
                }
                if action == "create" && current.is_some() {
                    return Err(BloraError::Other(format!(
                        "skill {name} already exists; use action update"
                    )));
                }
                if action == "update" && current.is_none() {
                    return Err(BloraError::Other(format!(
                        "skill {name} does not exist; use action create"
                    )));
                }
                let path = current.unwrap_or_else(|| root.join(format!("{name}.md")));
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(BloraError::exec)?;
                }
                std::fs::write(&path, format!("{content}\n")).map_err(BloraError::exec)?;
                path
            }
            "delete" => {
                let path = current
                    .ok_or_else(|| BloraError::Other(format!("skill {name} does not exist")))?;
                std::fs::remove_file(&path).map_err(BloraError::exec)?;
                path
            }
            other => {
                return Err(BloraError::Other(format!(
                    "unknown action {other}; use create, update, or delete"
                )));
            }
        };
        let shown = path
            .strip_prefix(workspace)
            .map_or_else(|_| path.display().to_string(), |rel| rel.display().to_string());
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::SkillChanged(SkillChanged {
                name: name.to_owned(),
                action: action.to_owned(),
                scope: scope.to_owned(),
                path: shown.clone(),
                previous,
            }),
        )?;
        Ok(format!("skill {name} {action}d at {shown}"))
    }

    /// Full-text search over earlier sessions, excluding the current one.
    pub fn session_search(
        &self,
        session_id: Option<&SessionId>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<blora_storage::SearchMatch>> {
        let user = match session_id {
            Some(id) => self.store.session_user_id(id)?,
            None => None,
        };
        self.store
            .search_transcripts(query, limit, session_id, user.as_deref())
    }

    /// Search every session visible in the TUI, including the current session.
    pub fn session_search_page(
        &self,
        query: &str,
        limit: usize,
        cursor: Option<&(String, i64)>,
    ) -> Result<(Vec<blora_storage::SearchMatch>, Option<(String, i64)>)> {
        self.store
            .search_transcripts_page(query, limit, None, None, cursor)
    }

    /// Batch latest message previews with canonical last-event sequences.
    pub fn session_message_previews(
        &self,
        session_ids: &[SessionId],
    ) -> Result<Vec<(SessionId, u64, Option<String>)>> {
        self.store.session_message_previews(session_ids)
    }

    pub(crate) fn session_search_text(
        &self,
        session_id: &SessionId,
        query: &str,
        limit: usize,
    ) -> Result<String> {
        let hits = self.session_search(Some(session_id), query, limit)?;
        if hits.is_empty() {
            return Ok(format!("No earlier sessions mention \"{}\".", query.trim()));
        }
        let mut out = format!("Earlier sessions matching \"{}\":\n", query.trim());
        for hit in hits {
            let date = hit.at.get(..10).unwrap_or(&hit.at);
            out.push_str(&format!(
                "\n- {date} {} ({}) [{}]\n  {}\n",
                hit.title.as_deref().unwrap_or("untitled"),
                hit.session_id,
                hit.kind,
                hit.snippet
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_names() {
        assert!(valid_name("deploy-staging_2"));
        assert!(!valid_name("Deploy"));
        assert!(!valid_name("../x"));
        assert!(!valid_name("-x"));
        assert!(!valid_name(""));
    }
}
