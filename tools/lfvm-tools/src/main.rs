//! LFVM 测试工具。
//!
//! ```text
//! lfvm-tools gen <目录> [--scale 1.0]              生成数据集 D1（1000 个文件，标准 1 GiB）
//! lfvm-tools bench <工作目录> <D1 目录> [--skip-d1] [--out 记录.md]
//!                                                  性能测试 P-05～P-09、P-11，每项 3 次
//! lfvm-tools stress <工作目录> [--rounds 8] [--interval-secs 1800] [--out 记录.md]
//!                                                  稳定性测试 Q-06（标准为 8 轮 × 30 分钟）
//! ```
//!
//! 测试记录须写明设备型号、系统版本、软件构建号（SRS 5.2 节），工具会自动写入系统和构建信息。

mod bench;
mod datasets;
mod stress;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn header(title: &str) -> String {
    format!(
        "# {title}\n\n- 软件构建：lfvm-tools {}（{}）\n- 系统：{} / {}\n- CPU 核数：{}\n- 生成时间（UTC 毫秒）：{}\n\n",
        env!("CARGO_PKG_VERSION"),
        if cfg!(debug_assertions) { "调试构建，正式测量请用 --release" } else { "发布构建" },
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(0, |n| n.get()),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())
    )
}

fn emit(out: Option<String>, text: &str) {
    match out {
        Some(p) => match std::fs::write(&p, text) {
            Ok(()) => eprintln!("已写入 {p}"),
            Err(e) => {
                eprintln!("写入 {p} 失败：{e}");
                println!("{text}");
            }
        },
        None => println!("{text}"),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let usage = "用法：lfvm-tools gen|bench|stress …（详见源码 tools/lfvm-tools/src/main.rs）";
    let result = match pos.first().map(|s| s.as_str()) {
        Some("gen") => {
            let Some(dir) = pos.get(1) else {
                eprintln!("{usage}");
                return ExitCode::FAILURE;
            };
            let scale: f64 = flag(&args, "--scale").and_then(|s| s.parse().ok()).unwrap_or(1.0);
            datasets::gen_d1(&PathBuf::from(dir), scale)
                .map(|(n, b)| println!("已生成 D1：{n} 个文件，共 {b} 字节"))
                .map_err(|e| e.to_string())
        }
        Some("bench") => {
            let (Some(work), Some(d1)) = (pos.get(1), pos.get(2)) else {
                eprintln!("{usage}");
                return ExitCode::FAILURE;
            };
            bench::run(&PathBuf::from(work), &PathBuf::from(d1), args.iter().any(|a| a == "--skip-d1"))
                .map(|md| emit(flag(&args, "--out"), &(header("性能测试记录（SRS 表 5-1）") + &md)))
                .map_err(|e| e.message)
        }
        Some("stress") => {
            let Some(work) = pos.get(1) else {
                eprintln!("{usage}");
                return ExitCode::FAILURE;
            };
            let rounds = flag(&args, "--rounds").and_then(|s| s.parse().ok()).unwrap_or(8);
            let secs = flag(&args, "--interval-secs").and_then(|s| s.parse().ok()).unwrap_or(1800);
            stress::run(&PathBuf::from(work), rounds, Duration::from_secs(secs)).map_err(|e| e.message).and_then(
                |(md, ok)| {
                    emit(flag(&args, "--out"), &(header("稳定性测试记录（LFVM-Q-06）") + &md));
                    if ok { Ok(()) } else { Err("稳定性测试未通过".to_owned()) }
                },
            )
        }
        _ => Err(usage.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
