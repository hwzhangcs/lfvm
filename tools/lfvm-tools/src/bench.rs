//! 性能测试（SRS 5.2 节、表 5-1）：每项在相同初始状态下连续测量 3 次，3 次均不超过限值为通过。
//!
//! 本工具直接调用核心库，测得的是后端处理时间；界面渲染部分（如时间地图首次显示、
//! 点击后可见反馈）需在桌面程序中录屏计时，记录表中单独注明。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use lfvm_core::compare::FileDiff;
use lfvm_core::new_id;
use lfvm_core::ops::restore::WorkspaceOpRequest;
use lfvm_core::progress::NoProgress;
use lfvm_core::scheme::{SwitchRequest, SwitchTarget};
use lfvm_core::search::SearchQuery;
use lfvm_core::storage::ClearRequest;
use lfvm_core::version::SaveRequest;
use lfvm_core::{Core, CoreResult};

use crate::datasets::{self, Rng};

pub struct Row {
    pub id: &'static str,
    pub scene: String,
    pub limit: f64,
    pub runs: Vec<f64>,
    pub note: String,
}

impl Row {
    fn pass(&self) -> bool {
        self.runs.len() == 3 && self.runs.iter().all(|t| *t <= self.limit)
    }
}

fn timed<T>(f: impl FnOnce() -> CoreResult<T>) -> CoreResult<(f64, T)> {
    let start = Instant::now();
    let v = f()?;
    Ok((start.elapsed().as_secs_f64(), v))
}

fn save(core: &Core, pid: &str) -> CoreResult<String> {
    Ok(core
        .save_version(
            &SaveRequest { project_id: pid.into(), request_id: new_id(), name: String::new(), note: String::new() },
            &NoProgress,
        )?
        .version_id)
}

/// 进程内存峰值与当前值（MB）。只在 Linux 上可读取，其他系统请用任务管理器或活动监视器记录。
pub fn memory_mb() -> Option<(f64, f64)> {
    let s = fs::read_to_string("/proc/self/status").ok()?;
    let field = |k: &str| {
        s.lines()
            .find(|l| l.starts_with(k))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<f64>().ok())
    };
    Some((field("VmHWM:")? / 1024.0, field("VmRSS:")? / 1024.0))
}

fn fresh_core(work: &Path, name: &str) -> CoreResult<Core> {
    let dir = work.join(name);
    let _ = fs::remove_dir_all(&dir);
    Core::open(dir)
}

/// P-05 首次扫描与 P-06 保存版本（数据集 D1）。
fn bench_d1(work: &Path, d1: &Path, rows: &mut Vec<Row>) -> CoreResult<()> {
    let mut scan = Vec::new();
    let mut saving = Vec::new();
    let mut rng = Rng::new(datasets::SEED + 6);
    for run in 0..3 {
        // 每次使用全新的历史存储目录，相当于清空应用缓存后首次打开
        let core = fresh_core(work, &format!("d1-data-{run}"))?;
        let pid = core.add_project(d1, None)?.project_id;
        let (t, cs) = timed(|| core.current_changes(&pid, &NoProgress))?;
        scan.push(t);
        save(&core, &pid)?;
        // 修改 100 个文件，合计约 100 MiB（按 D1 实际规模等比例）
        let total: u64 = cs.items.iter().filter_map(|i| i.size).map(|s| s as u64).sum();
        let budget = total / 10;
        let mut files: Vec<PathBuf> = Vec::new();
        collect_files(d1, &mut files);
        files.sort();
        let per = budget / 100;
        for f in
            files.iter().filter(|f| f.extension().is_some_and(|e| e == "docx" || e == "xlsx" || e == "pdf")).take(100)
        {
            let mut buf = vec![0u8; per as usize];
            rng.fill(&mut buf);
            fs::write(f, buf).map_err(lfvm_core::CoreError::from)?;
        }
        let (t, _) = timed(|| save(&core, &pid))?;
        saving.push(t);
        drop(core);
        let _ = fs::remove_dir_all(work.join(format!("d1-data-{run}")));
    }
    rows.push(Row {
        id: "LFVM-P-05",
        scene: "首次扫描（D1）".into(),
        limit: 60.0,
        runs: scan,
        note: "显示完整变化列表所需的后端时间".into(),
    });
    rows.push(Row {
        id: "LFVM-P-06",
        scene: "修改 100 个文件后保存（D1）".into(),
        limit: 30.0,
        runs: saving,
        note: "含复制、校验和登记".into(),
    });
    Ok(())
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_files(&p, out);
            } else {
                out.push(p);
            }
        }
    }
}

/// P-07 历史搜索（数据集 D2：20 个版本 + 20 个安全备份，各 500 条记录）。
fn bench_d2(work: &Path, rows: &mut Vec<Row>) -> CoreResult<()> {
    let core = fresh_core(work, "d2-data")?;
    let proj = work.join("d2-project");
    let _ = fs::remove_dir_all(&proj);
    let write_all = |round: u32| -> std::io::Result<()> {
        for i in 0..500 {
            let dir = proj.join(format!("dir_{}", i % 10));
            fs::create_dir_all(&dir)?;
            let ext = ["txt", "png", "docx", "md", "jpg"][i % 5];
            fs::write(dir.join(format!("file_{i:03}.{ext}")), format!("{round}-{i}"))?;
        }
        Ok(())
    };
    write_all(0).map_err(lfvm_core::CoreError::from)?;
    let pid = core.add_project(&proj, None)?.project_id;
    let mut versions = Vec::new();
    for round in 0..20 {
        write_all(round + 1).map_err(lfvm_core::CoreError::from)?;
        versions.push(save(&core, &pid)?);
    }
    // 20 次整版恢复，每次替换 500 个文件，产生 20 个各含 500 条记录的安全备份
    for i in 0..20 {
        let target = &versions[if i % 2 == 0 { 0 } else { 19 }];
        let plan = core.plan_restore(&pid, target, &NoProgress)?;
        core.restore_version(
            &WorkspaceOpRequest {
                project_id: pid.clone(),
                request_id: new_id(),
                version_id: target.clone(),
                fingerprint: plan.fingerprint,
            },
            &NoProgress,
        )?;
    }
    let queries: [(&str, SearchQuery); 4] = [
        ("按文件名", SearchQuery { name: Some("file_1".into()), ..Default::default() }),
        ("按扩展名", SearchQuery { ext: Some("png".into()), ..Default::default() }),
        ("按历史路径", SearchQuery { path: Some("dir_3/".into()), ..Default::default() }),
        ("无结果", SearchQuery { name: Some("不存在的文件".into()), ..Default::default() }),
    ];
    for (label, q) in queries {
        let mut runs = Vec::new();
        let mut total = 0;
        for _ in 0..3 {
            let (t, page) = timed(|| core.search_files(&pid, &q))?;
            total = page.total;
            runs.push(t);
        }
        rows.push(Row {
            id: "LFVM-P-07",
            scene: format!("历史搜索：{label}（D2）"),
            limit: 2.0,
            runs,
            note: format!("共 {total} 条，取首页 50 条"),
        });
    }
    Ok(())
}

/// P-08 版本比较（数据集 D3）。
fn bench_d3(work: &Path, rows: &mut Vec<Row>) -> CoreResult<()> {
    let core = fresh_core(work, "d3-data")?;
    let proj = work.join("d3-project");
    let _ = fs::remove_dir_all(&proj);
    fs::create_dir_all(&proj).map_err(lfvm_core::CoreError::from)?;
    let pairs = datasets::d3_pairs();
    for p in &pairs {
        fs::write(proj.join(p.name), &p.a).map_err(lfvm_core::CoreError::from)?;
    }
    let pid = core.add_project(&proj, None)?.project_id;
    let a = save(&core, &pid)?;
    for p in &pairs {
        fs::write(proj.join(p.name), &p.b).map_err(lfvm_core::CoreError::from)?;
    }
    let b = save(&core, &pid)?;
    for p in &pairs {
        let mut runs = Vec::new();
        let mut outcome = String::new();
        for _ in 0..3 {
            let (t, d) = timed(|| core.diff_file(&pid, &a, &b, p.name))?;
            outcome = match d {
                FileDiff::Text { lines, notes, .. } => {
                    let changed = lines.iter().filter(|l| l.tag != lfvm_core::compare::LineTag::Equal).count();
                    format!(
                        "逐行比较 {} 行，其中变化 {changed} 行{}",
                        lines.len(),
                        if notes.is_empty() { "" } else { "，含换行提示" }
                    )
                }
                FileDiff::Image { a, b } => {
                    let r = [a, b].into_iter().flatten().filter_map(|s| s.reason).next();
                    r.map_or("并排显示".to_owned(), |r| format!("降级：{r}"))
                }
                FileDiff::Unsupported { reason } => format!("降级：{reason}"),
            };
            runs.push(t);
        }
        rows.push(Row {
            id: "LFVM-P-08", scene: format!("比较：{}（D3）", p.name), limit: 5.0, runs, note: outcome
        });
    }
    Ok(())
}

/// P-09 时间地图（数据集 D4：500 个版本节点、10 条路线、10 个已清理占位）。
fn bench_d4(work: &Path, rows: &mut Vec<Row>) -> CoreResult<()> {
    let core = fresh_core(work, "d4-data")?;
    let proj = work.join("d4-project");
    let _ = fs::remove_dir_all(&proj);
    fs::create_dir_all(&proj).map_err(lfvm_core::CoreError::from)?;
    let file = proj.join("f.txt");
    let pid = core.add_project(&proj, None)?.project_id;
    let mut n = 0u32;
    let mut bump = || -> CoreResult<String> {
        n += 1;
        fs::write(&file, format!("v{n}")).map_err(lfvm_core::CoreError::from)?;
        save(&core, &pid)
    };
    let mut default = Vec::new();
    for _ in 0..50 {
        default.push(bump()?);
    }
    for lane in 1..10 {
        let s = core.create_scheme(&pid, &default[lane * 5], &format!("方案{lane}"))?;
        let target = SwitchTarget::Scheme { scheme_id: s.scheme_id };
        let check = core.check_switch(&pid, &target, &NoProgress)?;
        let fp = check.impact.map(|i| i.fingerprint).unwrap_or_default();
        core.switch_scheme(
            &SwitchRequest { project_id: pid.clone(), request_id: new_id(), target, fingerprint: fp },
            &NoProgress,
        )?;
        for _ in 0..50 {
            bump()?;
        }
    }
    let report = core.storage_report(&pid)?;
    let clearable: Vec<String> = report
        .versions
        .iter()
        .filter(|v| v.protected.is_none())
        .take(10)
        .map(|v| v.version.version_id.clone())
        .collect();
    core.clear_storage(&pid, &ClearRequest { request_id: new_id(), versions: clearable, ..Default::default() })?;

    let mut runs = Vec::new();
    let mut nodes = 0;
    for _ in 0..3 {
        let (t, m) = timed(|| core.time_map(&pid))?;
        nodes = m.nodes.len();
        runs.push(t);
    }
    rows.push(Row {
        id: "LFVM-P-09",
        scene: "时间地图数据（D4）".into(),
        limit: 3.0,
        runs,
        note: format!("{nodes} 个节点；界面首次显示、缩放平移 ≤0.2 秒、点击节点 ≤0.5 秒需在桌面程序中录屏计时"),
    });
    Ok(())
}

/// 运行全部测试，返回 Markdown 格式的测试记录。
pub fn run(work: &Path, d1: &Path, skip_d1: bool) -> CoreResult<String> {
    fs::create_dir_all(work).map_err(lfvm_core::CoreError::from)?;
    let mut rows = Vec::new();
    if !skip_d1 {
        eprintln!("测量 P-05、P-06（D1）…");
        bench_d1(work, d1, &mut rows)?;
    }
    eprintln!("测量 P-07（D2）…");
    bench_d2(work, &mut rows)?;
    eprintln!("测量 P-08（D3）…");
    bench_d3(work, &mut rows)?;
    eprintln!("测量 P-09（D4）…");
    bench_d4(work, &mut rows)?;

    let mut md = String::from(
        "| 编号 | 场景 | 限值（秒） | 第 1 次 | 第 2 次 | 第 3 次 | 结果 | 说明 |\n|---|---|---|---|---|---|---|---|\n",
    );
    for r in &rows {
        let t = |i: usize| r.runs.get(i).map_or("—".into(), |v| format!("{v:.3}"));
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.id,
            r.scene,
            r.limit,
            t(0),
            t(1),
            t(2),
            if r.pass() { "通过" } else { "未通过" },
            r.note
        ));
    }
    if let Some((peak, _)) = memory_mb() {
        md.push_str(&format!(
            "\n**LFVM-P-11 内存**：测试进程峰值 {peak:.0} MB（限值 1024 MB；只含后端，桌面程序需另用任务管理器记录全部进程）。\n"
        ));
    }
    Ok(md)
}
