//! 稳定性测试（LFVM-Q-06）：每隔一段时间执行一轮固定操作
//! （打开项目、扫描、保存、比较、搜索、单文件恢复、整版恢复、切换方案、展开、导出、清理），
//! 每轮结果须符合预期，最后一轮结束时的内存不超过第一轮结束时加 200 MB。
//! 标准参数为 8 轮、间隔 30 分钟（共 4 小时）。

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use lfvm_core::content::SourceRef;
use lfvm_core::new_id;
use lfvm_core::ops::OpStatus;
use lfvm_core::ops::restore::WorkspaceOpRequest;
use lfvm_core::ops::single::{FileRestoreRequest, FileTarget};
use lfvm_core::output::{OutputKind, OutputSource, OutputTarget};
use lfvm_core::progress::NoProgress;
use lfvm_core::scheme::{SwitchRequest, SwitchTarget};
use lfvm_core::search::SearchQuery;
use lfvm_core::storage::ClearRequest;
use lfvm_core::version::SaveRequest;
use lfvm_core::{Core, CoreError, CoreResult, ErrorCode};

use crate::bench::memory_mb;
use crate::datasets::{Rng, SEED, text_of};

fn expect(ok: bool, what: &str) -> CoreResult<()> {
    if ok { Ok(()) } else { Err(CoreError::new(ErrorCode::Internal, format!("结果不符合预期：{what}"))) }
}

fn round(core: &Core, pid: &str, proj: &Path, out: &Path, r: u32, rng: &mut Rng) -> CoreResult<()> {
    let io = |e: std::io::Error| CoreError::from(e);
    // 打开项目、扫描
    core.open_project(pid)?;
    // 修改并保存两个版本
    for k in 0..2 {
        for i in 0..20 {
            fs::write(proj.join(format!("docs/note_{i:02}.txt")), text_of(2_000 + rng.below(2_000), rng))
                .map_err(io)?;
        }
        fs::write(proj.join(format!("round_{r}_{k}.txt")), format!("{r}-{k}")).map_err(io)?;
        let cs = core.current_changes(pid, &NoProgress)?;
        expect(cs.has_changes, "扫描到修改")?;
        core.save_version(
            &SaveRequest {
                project_id: pid.into(),
                request_id: new_id(),
                name: format!("第{r}轮"),
                note: String::new(),
            },
            &NoProgress,
        )?;
    }
    let map = core.time_map(pid)?;
    let n = map.nodes.len();
    let (prev, last) = (map.nodes[n - 2].version_id.clone(), map.nodes[n - 1].version_id.clone());
    // 比较
    let cmp = core.compare_versions(pid, &prev, &last, None)?;
    expect(!cmp.items.is_empty(), "两版本有差异")?;
    core.diff_file(pid, &prev, &last, "docs/note_00.txt")?;
    // 搜索
    let hits = core.search_files(pid, &SearchQuery { name: Some("note_0".into()), ..Default::default() })?;
    expect(hits.total > 0, "搜索有结果")?;
    // 单文件恢复（原位，需确认替换）
    let src = SourceRef::Version { version_id: prev.clone() };
    let check = core.check_file_restore(pid, &src, "docs/note_01.txt", &FileTarget::Original)?;
    let res = core.restore_file(
        &FileRestoreRequest {
            project_id: pid.into(),
            request_id: new_id(),
            source: src,
            path: "docs/note_01.txt".into(),
            confirm_token: check.confirm_token,
        },
        &FileTarget::Original,
        &NoProgress,
    )?;
    expect(res.status == OpStatus::Succeeded, "单文件恢复成功")?;
    // 整版恢复到上一版本，再保存回来
    let plan = core.plan_restore(pid, &last, &NoProgress)?;
    let res = core.restore_version(
        &WorkspaceOpRequest {
            project_id: pid.into(),
            request_id: new_id(),
            version_id: last.clone(),
            fingerprint: plan.fingerprint,
        },
        &NoProgress,
    )?;
    expect(res.status == OpStatus::Succeeded, "整版恢复成功")?;
    // 切换方案并切回默认历史
    let scheme = core.create_scheme(pid, &prev, &format!("第{r}轮方案"))?;
    for target in [SwitchTarget::Scheme { scheme_id: scheme.scheme_id.clone() }, SwitchTarget::Default] {
        let c = core.check_switch(pid, &target, &NoProgress)?;
        expect(!c.unsaved, "切换前没有未保存的修改")?;
        let res = core.switch_scheme(
            &SwitchRequest {
                project_id: pid.into(),
                request_id: new_id(),
                target,
                fingerprint: c.impact.map(|i| i.fingerprint).unwrap_or_default(),
            },
            &NoProgress,
        )?;
        expect(res.status == OpStatus::Succeeded, "切换方案成功")?;
    }
    // 展开与导出
    for (kind, name) in [(OutputKind::Expand, format!("展开_{r}")), (OutputKind::Export, format!("导出_{r}"))] {
        let res = core.output_version(
            pid,
            &new_id(),
            kind,
            &OutputSource::Version { version_id: last.clone() },
            &OutputTarget { parent: out.to_path_buf(), folder_name: name.clone() },
            &NoProgress,
        )?;
        expect(res.status == OpStatus::Succeeded, "展开/导出成功")?;
        fs::remove_dir_all(out.join(name)).map_err(io)?;
    }
    // 清理：本轮方案、可清理的版本和全部安全备份
    let report = core.storage_report(pid)?;
    let versions: Vec<String> =
        report.versions.iter().filter(|v| v.protected.is_none()).map(|v| v.version.version_id.clone()).collect();
    let backups: Vec<String> =
        report.backups.iter().filter(|b| b.protected.is_none()).map(|b| b.backup_id.clone()).collect();
    core.clear_storage(
        pid,
        &ClearRequest { request_id: new_id(), versions, backups, schemes: vec![scheme.scheme_id] },
    )?;
    Ok(())
}

/// 返回（测试记录, 是否全部通过）。
pub fn run(work: &Path, rounds: u32, interval: Duration) -> CoreResult<(String, bool)> {
    let io = |e: std::io::Error| CoreError::from(e);
    let _ = fs::remove_dir_all(work);
    let proj = work.join("project");
    let out = work.join("output");
    fs::create_dir_all(proj.join("docs")).map_err(io)?;
    fs::create_dir_all(&out).map_err(io)?;
    let mut rng = Rng::new(SEED + 99);
    for i in 0..20 {
        fs::write(proj.join(format!("docs/note_{i:02}.txt")), text_of(3_000, &mut rng)).map_err(io)?;
    }
    let core = Core::open(work.join("data"))?;
    let pid = core.add_project(&proj, None)?.project_id;

    let mut report = String::from("| 轮次 | 用时（秒） | 结果 | 内存（MB） |\n|---|---|---|---|\n");
    let mut first_mem = None;
    let mut last_mem = None;
    let mut all_ok = true;
    let started = Instant::now();
    for r in 1..=rounds {
        let t = Instant::now();
        let result = round(&core, &pid, &proj, &out, r, &mut rng);
        let mem = memory_mb().map(|(_, rss)| rss);
        if r == 1 {
            first_mem = mem;
        }
        last_mem = mem;
        all_ok &= result.is_ok();
        report.push_str(&format!(
            "| {r} | {:.1} | {} | {} |\n",
            t.elapsed().as_secs_f64(),
            result.as_ref().map_or_else(|e| format!("失败：{}", e.message), |_| "符合预期".into()),
            mem.map_or("—".into(), |m| format!("{m:.0}"))
        ));
        eprintln!("第 {r}/{rounds} 轮：{}", if result.is_ok() { "符合预期" } else { "失败" });
        if r < rounds {
            let next = interval.saturating_sub(t.elapsed());
            std::thread::sleep(next);
        }
    }
    let mem_ok = match (first_mem, last_mem) {
        (Some(a), Some(b)) => b - a <= 200.0,
        _ => true,
    };
    let growth = match (first_mem, last_mem) {
        (Some(a), Some(b)) => {
            format!("{:.0} MB（限值 200 MB）{}", b - a, if b - a <= 200.0 { "，通过" } else { "，未通过" })
        }
        _ => "当前系统无法自动读取，请用任务管理器记录".into(),
    };
    report.push_str(&format!(
        "\n共 {rounds} 轮，总用时 {:.0} 秒；全部轮次{}；内存增长：{growth}\n",
        started.elapsed().as_secs_f64(),
        if all_ok { "符合预期" } else { "**存在失败**" }
    ));
    Ok((report, all_ok && mem_ok))
}
