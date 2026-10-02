//! 搜索历史文件（SRS 3.3.3.4、LFVM-C-04）。
//!
//! - 在内容可用的版本和就绪的安全备份中查找，多个条件同时满足才命中；
//! - 文件名和历史路径不区分大小写、按包含匹配；扩展名完整匹配，可省略前导点；
//! - 文件名或扩展名含 “*”“?” 时按通配符匹配整个名称；路径条件不解释通配符；
//! - 指定方案范围时，只在该方案末端沿父版本可追溯到的版本中查找；安全备份不受方案范围限制；
//! - 按保存时间倒序，每页 50 条；同名文件来自不同版本或备份时分行显示。

use rusqlite::{ToSql, params_from_iter};
use serde::{Deserialize, Serialize};

use crate::Core;
use crate::content::SourceRef;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::glob::has_wildcard;
use crate::project::load_project;
use crate::scheme::SwitchTarget;

pub const PAGE_SIZE: u32 = 50;

#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SearchQuery {
    pub name: Option<String>,
    pub ext: Option<String>,
    pub path: Option<String>,
    /// 方案范围；null 表示全部历史。
    pub scope: Option<SwitchTarget>,
    /// 从 0 开始的页码。
    pub page: u32,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SearchHit {
    pub source: SourceRef,
    /// 来源说明：“V12 名称”或安全备份的原因。
    pub source_label: String,
    /// 来源是版本时的序号。
    pub seq: Option<u32>,
    pub path: String,
    pub name: String,
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub size: Option<i64>,
    pub hash: String,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub created_at: i64,
    /// PNG/JPEG（按扩展名判断），界面为其显示缩略图。
    pub is_image: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    pub total: u32,
    pub page: u32,
    pub page_size: u32,
}

fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('%');
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// 把用户的 “*”“?” 通配符转成 SQLite GLOB 模式（转义 GLOB 自己的 “[”）。
fn glob_pattern(s: &str) -> String {
    s.chars().map(|c| if c == '[' { "[[]".to_owned() } else { c.to_string() }).collect()
}

fn clean(s: &Option<String>) -> Option<String> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_lowercase)
}

impl Core {
    pub fn search_files(&self, project_id: &str, q: &SearchQuery) -> CoreResult<SearchPage> {
        let name = clean(&q.name);
        let ext = clean(&q.ext).map(|e| e.trim_start_matches('.').to_owned()).filter(|e| !e.is_empty());
        let path = clean(&q.path);
        if name.is_none() && ext.is_none() && path.is_none() {
            return Err(CoreError::new(ErrorCode::InvalidInput, "请至少填写文件名、扩展名或历史路径中的一项"));
        }

        let db = self.db();
        let project = load_project(&db, project_id)?;
        let mut conds = vec!["f.entry_type = 'file'".to_owned()];
        let mut args: Vec<Box<dyn ToSql>> = vec![Box::new(project_id.to_owned())];
        let arg = |v: String, args: &mut Vec<Box<dyn ToSql>>| {
            args.push(Box::new(v));
            format!("?{}", args.len())
        };
        if let Some(n) = &name {
            let p = if has_wildcard(n) {
                format!("f.name_key GLOB {}", arg(glob_pattern(n), &mut args))
            } else {
                format!("f.name_key LIKE {} ESCAPE '\\'", arg(like_escape(n), &mut args))
            };
            conds.push(p);
        }
        if let Some(e) = &ext {
            let p = if has_wildcard(e) {
                format!("f.ext_key GLOB {}", arg(glob_pattern(e), &mut args))
            } else {
                format!("f.ext_key = {}", arg(e.clone(), &mut args))
            };
            conds.push(p);
        }
        if let Some(p) = &path {
            conds.push(format!("lower(f.relative_path) LIKE {} ESCAPE '\\'", arg(like_escape(p), &mut args)));
        }
        let where_files = conds.join(" AND ");

        // 方案范围：从末端沿父版本追溯
        let scope_head = match &q.scope {
            None => None,
            Some(SwitchTarget::Default) => Some(project.default_head.clone().unwrap_or_default()),
            Some(SwitchTarget::Scheme { scheme_id }) => Some(crate::scheme::scheme_head(&db, project_id, scheme_id)?.1),
        };
        let (cte, version_scope) = match scope_head {
            Some(h) => (
                format!(
                    "WITH RECURSIVE anc(id) AS (SELECT {} UNION SELECT v.parent_version_id FROM versions v JOIN anc ON v.version_id = anc.id
                       WHERE v.parent_version_id IS NOT NULL) ",
                    arg(h, &mut args)
                ),
                " AND v.version_id IN (SELECT id FROM anc)",
            ),
            None => (String::new(), ""),
        };
        let union = format!(
            "SELECT 'v' AS kind, v.version_id AS sid, v.seq AS seq, v.name AS label, v.created_at AS created_at,
                    f.relative_path AS rel, f.size AS size, f.content_hash AS hash, f.ext_key AS ext
               FROM version_files f JOIN versions v ON v.version_id = f.version_id
              WHERE v.project_id = ?1 AND v.payload_state = 'available' AND {where_files}{version_scope}
             UNION ALL
             SELECT 'b', b.backup_id, NULL, b.reason, b.created_at, f.relative_path, f.size, f.content_hash, f.ext_key
               FROM backup_files f JOIN safety_backups b ON b.backup_id = f.backup_id
              WHERE b.project_id = ?1 AND b.status = 'ready' AND {where_files}"
        );
        let total: u32 = db.query_row(
            &format!("{cte}SELECT count(*) FROM ({union})"),
            params_from_iter(args.iter().map(|a| a.as_ref())),
            |r| r.get(0),
        )?;
        let page = q.page;
        let mut stmt = db.prepare(&format!(
            "{cte}SELECT kind, sid, seq, label, created_at, rel, size, hash, ext FROM ({union})
              ORDER BY created_at DESC, rel LIMIT {PAGE_SIZE} OFFSET {}",
            page * PAGE_SIZE
        ))?;
        let hits = stmt
            .query_map(params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
                let kind: String = r.get(0)?;
                let sid: String = r.get(1)?;
                let seq: Option<u32> = r.get(2)?;
                let label: String = r.get(3)?;
                let rel: String = r.get(5)?;
                let ext: String = r.get(8)?;
                Ok(SearchHit {
                    source: if kind == "v" {
                        SourceRef::Version { version_id: sid }
                    } else {
                        SourceRef::Backup { backup_id: sid }
                    },
                    source_label: match seq {
                        Some(s) => format!("V{s} {label}").trim().to_owned(),
                        None => label,
                    },
                    seq,
                    name: rel.rsplit('/').next().unwrap_or(&rel).to_owned(),
                    path: rel,
                    size: r.get(6)?,
                    hash: r.get(7)?,
                    created_at: r.get(4)?,
                    is_image: matches!(ext.as_str(), "png" | "jpg" | "jpeg"),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SearchPage { hits, total, page, page_size: PAGE_SIZE })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::NoProgress;
    use crate::version::SaveRequest;

    fn setup() -> (tempfile::TempDir, Core, String, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let core = Core::open(tmp.path().join("data")).unwrap();
        let work = tmp.path().join("w");
        std::fs::create_dir_all(work.join("图片")).unwrap();
        std::fs::write(work.join("图片/Cover.PNG"), b"x").unwrap();
        std::fs::write(work.join("报告_最终.docx"), b"x").unwrap();
        std::fs::write(work.join("100%_done.txt"), b"x").unwrap();
        let pid = core.add_project(&work, None).unwrap().project_id;
        (tmp, core, pid, work)
    }

    fn save(core: &Core, pid: &str) -> String {
        core.save_version(
            &SaveRequest {
                project_id: pid.into(),
                request_id: crate::new_id(),
                name: "第一版".into(),
                note: String::new(),
            },
            &NoProgress,
        )
        .unwrap()
        .version_id
    }

    fn q(name: Option<&str>, ext: Option<&str>, path: Option<&str>) -> SearchQuery {
        SearchQuery {
            name: name.map(Into::into),
            ext: ext.map(Into::into),
            path: path.map(Into::into),
            scope: None,
            page: 0,
        }
    }

    /// AC-0021：名称包含、扩展名完整匹配、路径包含、通配符、多条件、大小写、无结果。
    #[test]
    fn matching_rules() {
        let (_t, core, pid, _w) = setup();
        save(&core, &pid);
        let names = |q: SearchQuery| -> Vec<String> {
            core.search_files(&pid, &q).unwrap().hits.into_iter().map(|h| h.name).collect()
        };
        assert_eq!(names(q(Some("cover"), None, None)), ["Cover.PNG"]);
        assert_eq!(names(q(None, Some(".png"), None)), ["Cover.PNG"]);
        assert_eq!(names(q(None, Some("pn"), None)), Vec::<String>::new(), "扩展名完整匹配");
        assert_eq!(names(q(None, None, Some("图片/"))), ["Cover.PNG"]);
        assert_eq!(names(q(Some("报告*"), None, None)), ["报告_最终.docx"]);
        assert_eq!(names(q(Some("报告?最终.docx"), None, None)), ["报告_最终.docx"]);
        assert_eq!(names(q(Some("报告"), Some("txt"), None)), Vec::<String>::new(), "条件同时满足");
        assert_eq!(names(q(Some("100%"), None, None)), ["100%_done.txt"], "% 不作为通配符");
        assert_eq!(names(q(Some("_"), None, None)).len(), 2);
        let hit = &core.search_files(&pid, &q(Some("cover"), None, None)).unwrap().hits[0];
        assert!(hit.is_image);
        assert_eq!(hit.source_label, "V1 第一版");
        assert_eq!(core.search_files(&pid, &q(None, None, None)).unwrap_err().code, ErrorCode::InvalidInput);
    }

    #[test]
    fn same_file_in_several_versions_and_scheme_scope() {
        let (_t, core, pid, work) = setup();
        let v1 = save(&core, &pid);
        std::fs::write(work.join("new.txt"), b"x").unwrap();
        save(&core, &pid);
        let all = core.search_files(&pid, &q(Some("cover"), None, None)).unwrap();
        assert_eq!(all.total, 2, "同名文件来自不同版本时分行显示");
        assert!(all.hits[0].created_at >= all.hits[1].created_at);

        let s = core.create_scheme(&pid, &v1, "方案").unwrap();
        let scoped =
            SearchQuery { scope: Some(SwitchTarget::Scheme { scheme_id: s.scheme_id }), ..q(Some("new"), None, None) };
        assert_eq!(core.search_files(&pid, &scoped).unwrap().total, 0, "方案只追溯到 V1");
        let scoped = SearchQuery { scope: Some(SwitchTarget::Default), ..q(Some("new"), None, None) };
        assert_eq!(core.search_files(&pid, &scoped).unwrap().total, 1);
    }
}
