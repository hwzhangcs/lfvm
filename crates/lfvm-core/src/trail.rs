//! 历史文件轨迹（SRS 3.3.3.5、规则 R-04）。
//!
//! 轨迹按“相同相对路径”在一条历史路线上追踪，不推断改名、移动或复制：
//! - 沿路线从末端向前追溯父版本，列出包含该路径的版本，标出内容发生变化的版本；
//! - “首次缺失版本”只依据版本清单，不代表真实的删除时间；删除后又出现时分段显示；
//! - 缺失发生在因排除未纳入的范围内时标为“因排除未纳入”；中间版本内容已清理时提示“记录不完整”；
//! - 其他路径有完全相同的内容时只作提示，不合并为同一文件的轨迹。

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::exclude::{self, Matcher};
use crate::paths::{RelPath, path_key};
use crate::project::{VersionBrief, load_project};
use crate::scheme::{SwitchTarget, scheme_head};
use crate::Core;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum TrailState {
    /// 包含该路径，内容与上一次出现时相同。
    Present,
    /// 包含该路径，且内容与上一次出现时不同（或首次出现）。
    Changed,
    /// 不包含该路径。
    Missing,
    /// 不包含该路径，且该路径在这个版本保存时被排除。
    Excluded,
    /// 版本内容已清理，无法判断。
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct TrailEntry {
    pub version: VersionBrief,
    pub state: TrailState,
    pub hash: Option<String>,
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub size: Option<i64>,
}

/// 一段连续出现。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct TrailSegment {
    pub first: VersionBrief,
    pub last: VersionBrief,
    /// 这段之后第一个不含该路径的版本；末端仍包含时为 null。
    pub first_missing: Option<VersionBrief>,
    /// 首次缺失是因为排除未纳入。
    pub missing_by_exclusion: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SameContent {
    pub path: String,
    pub version: VersionBrief,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Trail {
    pub path: String,
    pub route_label: String,
    /// 沿路线由早到晚。
    pub entries: Vec<TrailEntry>,
    pub segments: Vec<TrailSegment>,
    /// 末端版本仍包含该路径（“尚未缺失”）。
    pub still_present: bool,
    /// 路线中有内容已清理的版本，记录不完整。
    pub incomplete: bool,
    /// 包含该路径的版本内容都已清理，无法预览或恢复。
    pub all_cleared: bool,
    /// 其他路径中有完全相同内容的文件（只作提示）。
    pub same_content: Vec<SameContent>,
}

impl Core {
    pub fn file_trail(&self, project_id: &str, path: &str, route: &SwitchTarget) -> CoreResult<Trail> {
        let rel = RelPath::parse(path)?;
        let db = self.db();
        let project = load_project(&db, project_id)?;
        let key = path_key(rel.as_str(), project.policy);
        let (head, route_label) = match route {
            SwitchTarget::Default => (project.default_head.clone(), "默认历史".to_owned()),
            SwitchTarget::Scheme { scheme_id } => {
                let (name, head) = scheme_head(&db, project_id, scheme_id)?;
                (Some(head), format!("方案“{name}”"))
            }
        };
        let Some(head) = head else {
            return Err(CoreError::new(ErrorCode::NotFound, "这条路线上还没有版本"));
        };

        // 沿父版本追溯，得到由早到晚的路线
        let mut chain = Vec::new();
        let mut cur = Some(head);
        while let Some(v) = cur {
            let row: Option<(Option<String>, String, String)> = db
                .query_row(
                    "SELECT parent_version_id, payload_state, exclusion_rules_snapshot FROM versions WHERE version_id = ?1",
                    [&v],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((parent, payload, rules)) = row else { break };
            chain.push((v, payload == "cleared", rules));
            cur = parent;
        }
        chain.reverse();

        let mut entries = Vec::with_capacity(chain.len());
        let mut last_hash: Option<String> = None;
        for (v, cleared, rules) in &chain {
            let version = crate::project::version_brief(&db, v)?;
            if *cleared {
                entries.push(TrailEntry { version, state: TrailState::Unknown, hash: None, size: None });
                continue;
            }
            let found: Option<(Option<String>, Option<i64>)> = db
                .query_row(
                    "SELECT content_hash, size FROM version_files WHERE version_id = ?1 AND path_key = ?2 AND entry_type = 'file'",
                    params![v, key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let entry = match found {
                Some((hash, size)) => {
                    let state = if hash != last_hash { TrailState::Changed } else { TrailState::Present };
                    last_hash.clone_from(&hash);
                    TrailEntry { version, state, hash, size }
                }
                None => {
                    let excluded = Matcher::new(&exclude::parse_snapshot(rules), project.policy)
                        .is_ok_and(|m| m.is_excluded(&rel, false));
                    last_hash = None;
                    TrailEntry {
                        version,
                        state: if excluded { TrailState::Excluded } else { TrailState::Missing },
                        hash: None,
                        size: None,
                    }
                }
            };
            entries.push(entry);
        }

        // 分段：连续出现的版本为一段，段后第一个不含该路径的版本为“首次缺失”
        let present = |s: TrailState| matches!(s, TrailState::Present | TrailState::Changed);
        let mut segments = Vec::new();
        let mut i = 0;
        while i < entries.len() {
            if !present(entries[i].state) {
                i += 1;
                continue;
            }
            let start = i;
            while i + 1 < entries.len() && present(entries[i + 1].state) {
                i += 1;
            }
            let after = entries.get(i + 1).filter(|e| matches!(e.state, TrailState::Missing | TrailState::Excluded));
            segments.push(TrailSegment {
                first: entries[start].version.clone(),
                last: entries[i].version.clone(),
                first_missing: after.map(|e| e.version.clone()),
                missing_by_exclusion: after.is_some_and(|e| e.state == TrailState::Excluded),
            });
            i += 1;
        }

        let hashes: Vec<String> = entries.iter().filter_map(|e| e.hash.clone()).collect();
        let mut same_content = Vec::new();
        if !hashes.is_empty() {
            let ids: Vec<&str> = chain.iter().map(|(v, _, _)| v.as_str()).collect();
            let mut stmt = db.prepare(
                "SELECT f.relative_path, f.version_id FROM version_files f
                  WHERE f.content_hash = ?1 AND f.path_key != ?2 AND f.version_id = ?3",
            )?;
            let mut seen = std::collections::HashSet::new();
            'outer: for h in hashes.iter().collect::<std::collections::HashSet<_>>() {
                for v in &ids {
                    for row in stmt.query_map(params![h, key, v], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                        let (p, vid) = row?;
                        if seen.insert(p.clone()) {
                            same_content.push(SameContent { path: p, version: crate::project::version_brief(&db, &vid)? });
                            if same_content.len() >= 20 {
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }

        let incomplete = entries.iter().any(|e| e.state == TrailState::Unknown);
        let still_present = entries.last().is_some_and(|e| present(e.state));
        let all_cleared = entries.iter().all(|e| !present(e.state)) && incomplete;
        Ok(Trail {
            path: rel.to_string(),
            route_label,
            entries,
            segments,
            still_present,
            incomplete,
            all_cleared,
            same_content,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exclude::{ExclusionRule, RuleType};
    use crate::progress::NoProgress;
    use crate::version::SaveRequest;

    /// AC-0023：出现→删除→再出现、复制、排除：轨迹按路径分段，首次缺失版本正确，相同内容只作提示。
    #[test]
    fn segments_and_hints() {
        let tmp = tempfile::tempdir().unwrap();
        let core = Core::open(tmp.path().join("data")).unwrap();
        let w = tmp.path().join("w");
        std::fs::create_dir_all(&w).unwrap();
        let pid = core.add_project(&w, None).unwrap().project_id;
        let save = || {
            core.save_version(
                &SaveRequest { project_id: pid.clone(), request_id: crate::new_id(), name: String::new(), note: String::new() },
                &NoProgress,
            )
            .unwrap()
            .seq
        };
        std::fs::write(w.join("a.txt"), "1").unwrap();
        save(); // V1 出现
        std::fs::write(w.join("a.txt"), "2").unwrap();
        save(); // V2 修改
        std::fs::remove_file(w.join("a.txt")).unwrap();
        std::fs::write(w.join("copy.txt"), "2").unwrap();
        save(); // V3 缺失（同内容在另一路径）
        std::fs::write(w.join("a.txt"), "3").unwrap();
        save(); // V4 再出现
        let mut rules: Vec<_> = core.exclusion_rules(&pid).unwrap().into_iter().map(|v| v.rule).collect();
        rules.push(ExclusionRule { relative_path: "a.txt".into(), entry_type: RuleType::File, is_system_default: false, enabled: true });
        core.save_exclusion_rules(&pid, &rules).unwrap();
        save(); // V5 因排除未纳入

        let t = core.file_trail(&pid, "a.txt", &SwitchTarget::Default).unwrap();
        let states: Vec<TrailState> = t.entries.iter().map(|e| e.state).collect();
        use TrailState::*;
        assert_eq!(states, [Changed, Changed, Missing, Changed, Excluded]);
        assert_eq!(t.segments.len(), 2);
        assert_eq!(t.segments[0].last.seq, 2);
        assert_eq!(t.segments[0].first_missing.as_ref().unwrap().seq, 3);
        assert!(!t.segments[0].missing_by_exclusion);
        assert!(t.segments[1].missing_by_exclusion);
        assert!(!t.still_present);
        assert_eq!(t.same_content[0].path, "copy.txt");
    }
}
