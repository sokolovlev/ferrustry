//! Headless-запуск Ferrustry: сценарии, бенчмарки и проверка сохранений.

mod alloc;
mod balance;

use std::process::ExitCode;
use std::time::Instant;

use ferrustry_content::ContentRegistry;
use ferrustry_format::{
    PlayOutcome, Recorder, play, read_replay, read_save, write_replay, write_save,
};
use ferrustry_sim::{TICKS_PER_SECOND, World, scenario, state_hash};
use serde::Serialize;

#[global_allocator]
static GLOBAL: alloc::Counting = alloc::Counting;

const USAGE: &str = "\
ferrustry-headless — headless-запуск симуляции Ferrustry

ИСПОЛЬЗОВАНИЕ:
  ferrustry-headless run [--ticks N] [--save FILE]   вертикальный срез, сводка
  ferrustry-headless resume FILE [--ticks N]          продолжить сохранение
  ferrustry-headless save-check [--ticks N]           save → load → сравнить продолжение
  ferrustry-headless bench <b1|b2|b2-blocked|b2-comb|b3|b5|b8> [--ticks N] [--warmup N] [--conveyors N]
                           [--full-flow] [--out FILE]   --full-flow: поле путей всегда целиком (эталон)
  ferrustry-headless suite [--runs N] [--quick] [--out FILE]  полный набор бенчмарков (медианы)
  ferrustry-headless record --out FILE [--seed S] [--ticks N] [--size N] [--checkpoint-every N]
                                                    записать партию (случайная стройка)
  ferrustry-headless replay FILE                      воспроизвести и сверить контрольные точки
  ferrustry-headless balance [--difficulty easy|normal|hard] [--waves N] [--lang ru|en] [--out FILE]
                                                    отчёт о балансе: кривая сложности
";

struct Args {
    rest: Vec<String>,
}

impl Args {
    fn value(&mut self, name: &str) -> Result<Option<String>, String> {
        if let Some(i) = self.rest.iter().position(|a| a == name) {
            if i + 1 >= self.rest.len() {
                return Err(format!("{name} requires a value"));
            }
            let v = self.rest.remove(i + 1);
            self.rest.remove(i);
            Ok(Some(v))
        } else {
            Ok(None)
        }
    }

    fn number(&mut self, name: &str, default: u64) -> Result<u64, String> {
        match self.value(name)? {
            Some(v) => v
                .parse()
                .map_err(|_| format!("{name}: `{v}` is not a number")),
            None => Ok(default),
        }
    }

    fn flag(&mut self, name: &str) -> bool {
        match self.rest.iter().position(|a| a == name) {
            Some(i) => {
                self.rest.remove(i);
                true
            }
            None => false,
        }
    }

    fn positional(&mut self) -> Option<String> {
        (!self.rest.is_empty()).then(|| self.rest.remove(0))
    }

    fn finish(self) -> Result<(), String> {
        match self.rest.first() {
            Some(a) => Err(format!("unexpected argument `{a}`")),
            None => Ok(()),
        }
    }
}

fn main() -> ExitCode {
    #[cfg(target_arch = "x86_64")]
    if !std::arch::is_x86_feature_detected!("avx2") {
        eprintln!(
            "error: Ferrustry requires a CPU with AVX2 (Intel Haswell 2013+ / AMD Zen or newer)"
        );
        return ExitCode::FAILURE;
    }
    let mut args = Args {
        rest: std::env::args().skip(1).collect(),
    };
    match real_main(&mut args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn real_main(args: &mut Args) -> Result<(), String> {
    let reg = ContentRegistry::base().map_err(|e| e.to_string())?;
    let cmd = args.positional().ok_or("no command")?;
    let mut args = Args {
        rest: std::mem::take(&mut args.rest),
    };
    match cmd.as_str() {
        "run" => {
            let ticks = args.number("--ticks", 3000)?;
            let save = args.value("--save")?;
            args.finish()?;
            let (mut w, _) = scenario::vertical_slice(&reg).map_err(|e| e.to_string())?;
            for _ in 0..ticks {
                w.step(&reg);
            }
            summary(&w, &reg);
            if let Some(path) = save {
                let bytes = write_save(&w, &reg).map_err(|e| e.to_string())?;
                std::fs::write(&path, &bytes).map_err(|e| format!("{path}: {e}"))?;
                println!("saved {} bytes to {path}", bytes.len());
            }
            Ok(())
        }
        "resume" => {
            let path = args.positional().ok_or("resume requires FILE")?;
            let ticks = args.number("--ticks", 3000)?;
            args.finish()?;
            let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
            let mut w = read_save(&bytes, &reg).map_err(|e| e.to_string())?;
            for _ in 0..ticks {
                w.step(&reg);
            }
            summary(&w, &reg);
            Ok(())
        }
        "save-check" => {
            let ticks = args.number("--ticks", 1000)?;
            args.finish()?;
            let (mut a, _) = scenario::vertical_slice(&reg).map_err(|e| e.to_string())?;
            for _ in 0..ticks {
                a.step(&reg);
            }
            let bytes = write_save(&a, &reg).map_err(|e| e.to_string())?;
            let mut b = read_save(&bytes, &reg).map_err(|e| e.to_string())?;
            for _ in 0..ticks {
                a.step(&reg);
                b.step(&reg);
            }
            let (ha, hb) = (state_hash(&a), state_hash(&b));
            println!("save size: {} bytes", bytes.len());
            println!("continuous: {ha:016x}\nreloaded:   {hb:016x}");
            if ha == hb {
                Ok(())
            } else {
                Err("hash mismatch".into())
            }
        }
        "balance" => {
            let difficulty = match args.value("--difficulty")?.as_deref() {
                None | Some("normal") => ferrustry_sim::mapgen::Difficulty::Normal,
                Some("easy") => ferrustry_sim::mapgen::Difficulty::Easy,
                Some("hard") => ferrustry_sim::mapgen::Difficulty::Hard,
                Some(other) => return Err(format!("unknown difficulty `{other}`")),
            };
            let waves = u32::try_from(args.number("--waves", 20)?).map_err(|e| e.to_string())?;
            let en = match args.value("--lang")?.as_deref() {
                None | Some("ru") => false,
                Some("en") => true,
                Some(other) => return Err(format!("unknown language `{other}`")),
            };
            let out = args.value("--out")?;
            args.finish()?;
            let text = balance::report(&reg, difficulty, waves, en)?;
            match out {
                Some(path) => std::fs::write(&path, &text).map_err(|e| format!("{path}: {e}"))?,
                None => print!("{text}"),
            }
            Ok(())
        }
        "bench" => {
            let name = args.positional().ok_or("bench requires a scenario name")?;
            let ticks = args.number("--ticks", 3000)?;
            let warmup = args.number("--warmup", 300)?;
            let conveyors = args.number("--conveyors", 10_000)?;
            let out = args.value("--out")?;
            let full_flow = args.flag("--full-flow");
            args.finish()?;
            let report = bench(&reg, &name, ticks, warmup, conveyors, full_flow)?;
            let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            println!("{json}");
            if let Some(path) = out {
                std::fs::write(&path, json).map_err(|e| format!("{path}: {e}"))?;
            }
            Ok(())
        }
        "record" => {
            let seed = args.number("--seed", 1)?;
            let ticks = args.number("--ticks", 3000)?;
            let size = args.number("--size", 32)?;
            let every = args.number("--checkpoint-every", 100)?;
            let out = args.value("--out")?.ok_or("record requires --out FILE")?;
            args.finish()?;
            let size = u32::try_from(size).map_err(|e| e.to_string())?;
            let (mut w, mut builder) =
                scenario::RandomBuilder::new(&reg, size, seed).map_err(|e| e.to_string())?;
            let mut rec = Recorder::new(&w, every);
            for _ in 0..ticks {
                let cmds = builder.commands(w.tick_count());
                rec.tick(&mut w, &reg, &cmds);
            }
            let replay = rec.finish(&w);
            let bytes = write_replay(&replay, &reg).map_err(|e| e.to_string())?;
            std::fs::write(&out, &bytes).map_err(|e| format!("{out}: {e}"))?;
            println!(
                "recorded {} ticks, {} command ticks, {} checkpoints, {} bytes -> {out}",
                replay.ticks(),
                replay.inputs().len(),
                replay.checkpoints().len(),
                bytes.len()
            );
            println!("final hash: {:016x}", state_hash(&w));
            Ok(())
        }
        "replay" => {
            let path = args.positional().ok_or("replay requires FILE")?;
            args.finish()?;
            let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
            let replay = read_replay(&bytes, &reg).map_err(|e| e.to_string())?;
            let t = Instant::now();
            let (outcome, _) = play(&replay, &reg);
            match outcome {
                PlayOutcome::Verified {
                    checkpoints,
                    final_hash,
                } => {
                    println!(
                        "OK: {} ticks, {checkpoints} checkpoints verified in {:.1} ms",
                        replay.ticks(),
                        t.elapsed().as_secs_f64() * 1e3
                    );
                    println!("final hash: {final_hash:016x}");
                    Ok(())
                }
                PlayOutcome::Diverged {
                    tick,
                    expected,
                    actual,
                    last_good_tick,
                } => Err(format!(
                    "DESYNC at tick {tick}: expected {expected:016x}, got {actual:016x}; last matching checkpoint: {}",
                    last_good_tick.map_or("none".to_owned(), |t| t.to_string())
                )),
            }
        }
        "suite" => {
            let runs = args.number("--runs", 3)?;
            let quick = args.rest.iter().any(|a| a == "--quick");
            args.rest.retain(|a| a != "--quick");
            let out = args
                .value("--out")?
                .unwrap_or_else(|| "bench-suite.json".to_owned());
            args.finish()?;
            suite(runs, quick, &out)
        }
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

fn summary(w: &World, reg: &ContentRegistry) {
    println!("tick: {}", w.tick_count());
    println!(
        "buildings: {}, conveyor segments: {}",
        w.buildings().len(),
        w.segments().count()
    );
    let core = w.total_core_items();
    for (i, def) in reg.items().iter().enumerate() {
        println!(
            "  {:<12} core {:>6}  produced {:>6}  consumed {:>6}",
            def.name,
            core[i],
            w.stats().produced[i],
            w.stats().consumed[i]
        );
    }
    println!("state hash: {:016x}", state_hash(w));
}

#[derive(Serialize)]
struct TickStats {
    mean_us: f64,
    p50_us: f64,
    p99_us: f64,
    max_us: f64,
    over_budget: usize,
}

#[derive(Serialize)]
struct Machine {
    os: &'static str,
    arch: &'static str,
    cpu: Option<String>,
    logical_cpus: usize,
}

#[derive(Serialize)]
struct Report {
    scenario: String,
    ferrustry_version: &'static str,
    commit: Option<String>,
    profile: &'static str,
    tick_rate_hz: u32,
    budget_us: f64,
    target_cpu: &'static str,
    warmup_ticks: u64,
    measured_ticks: u64,
    buildings: usize,
    conveyor_segments: usize,
    awake_segments: usize,
    awake_drills: usize,
    items_on_conveyors: u64,
    items_in_cores: u64,
    units: usize,
    kills: u64,
    setup_ms: f64,
    tick: TickStats,
    allocations_per_tick: f64,
    peak_rss_kib: Option<u64>,
    state_hash: String,
    machine: Machine,
}

fn bench(
    reg: &ContentRegistry,
    name: &str,
    ticks: u64,
    warmup: u64,
    conveyors: u64,
    full_flow: bool,
) -> Result<Report, String> {
    let n = u32::try_from(conveyors).map_err(|e| e.to_string())?;
    let setup = Instant::now();
    let mut w = match name {
        "b1" => scenario::bench_empty(reg),
        "b2" => scenario::bench_conveyor_loops(reg, n),
        "b2-blocked" => scenario::bench_blocked_lines(reg, n),
        "b2-comb" => scenario::bench_combs(reg, n),
        "b3" => scenario::bench_megabase(reg, n),
        "b5" | "b8" => scenario::bench_battle(reg, n, 300),
        other => {
            return Err(format!(
                "unknown benchmark `{other}` (available: b1, b2, b2-blocked, b2-comb, b3, b5, b8)"
            ));
        }
    }
    .map_err(|e| e.to_string())?;
    w.set_incremental_flow(!full_flow);
    // B8: бой, во время которого игрок строит и сносит стены на пути врагов
    let building = name == "b8";
    let setup_ms = setup.elapsed().as_secs_f64() * 1e3;

    let step = |w: &mut World| {
        if building && let Some(cmd) = scenario::battle_build_command(w, reg) {
            let _ = w.apply(reg, &cmd);
        }
        w.step(reg);
    };
    for _ in 0..warmup {
        step(&mut w);
    }
    let mut samples: Vec<f64> = Vec::with_capacity(usize::try_from(ticks).unwrap_or(0));
    let allocs_before = alloc::count();
    for _ in 0..ticks {
        let t = Instant::now();
        step(&mut w);
        samples.push(t.elapsed().as_secs_f64() * 1e6);
    }
    let allocs = alloc::count() - allocs_before;

    let budget_us = 1e6 / f64::from(TICKS_PER_SECOND);
    let over_budget = samples.iter().filter(|&&s| s > budget_us).count();
    let mean = samples.iter().sum::<f64>() / samples.len().max(1) as f64;
    samples.sort_by(f64::total_cmp);
    let pct = |p: f64| {
        if samples.is_empty() {
            0.0
        } else {
            samples[((samples.len() - 1) as f64 * p).round() as usize]
        }
    };

    Ok(Report {
        scenario: name.to_owned(),
        ferrustry_version: env!("CARGO_PKG_VERSION"),
        commit: std::env::var("FERRUSTRY_COMMIT").ok(),
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        tick_rate_hz: TICKS_PER_SECOND,
        budget_us,
        target_cpu: if cfg!(target_feature = "avx2") {
            "avx2"
        } else {
            "baseline"
        },
        warmup_ticks: warmup,
        measured_ticks: ticks,
        buildings: w.buildings().len(),
        conveyor_segments: w.segments().count(),
        awake_segments: w.awake_segments(),
        awake_drills: w.awake_drills(),
        items_on_conveyors: w.items_on_conveyors().iter().sum(),
        items_in_cores: w.total_core_items().iter().sum(),
        units: w.units().count(),
        kills: w.kills(),
        setup_ms,
        tick: TickStats {
            mean_us: mean,
            p50_us: pct(0.5),
            p99_us: pct(0.99),
            max_us: pct(1.0),
            over_budget,
        },
        allocations_per_tick: allocs as f64 / ticks.max(1) as f64,
        peak_rss_kib: peak_rss_kib(),
        state_hash: format!("{:016x}", state_hash(&w)),
        machine: Machine {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            cpu: cpu_model(),
            logical_cpus: std::thread::available_parallelism().map_or(0, |n| n.get()),
        },
    })
}

/// Пиковый RSS на Linux (`VmHWM`); на остальных ОС — текущий RSS в конце прогона
/// (после прогрева и измерения он практически равен пиковому для этих сценариев).
fn peak_rss_kib() -> Option<u64> {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status")
        && let Some(line) = status.lines().find(|l| l.starts_with("VmHWM:"))
        && let Some(v) = line.split_whitespace().nth(1).and_then(|v| v.parse().ok())
    {
        return Some(v);
    }
    memory_stats::memory_stats().map(|m| (m.physical_mem / 1024) as u64)
}

fn cpu_model() -> Option<String> {
    let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let line = info.lines().find(|l| l.starts_with("model name"))?;
    Some(line.split(':').nth(1)?.trim().to_owned())
}

/// Полный набор бенчмарков: каждый прогон — отдельный процесс (чистый RSS, без влияния
/// предыдущих сценариев); итог — медианы по прогонам.
fn suite(runs: u64, quick: bool, out: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (warmup, ticks) = if quick { (1000, 500) } else { (6000, 3000) };
    let sizes: &[u64] = if quick { &[10_000] } else { &[10_000, 100_000] };
    let mut cases: Vec<(&str, u64, u64, u64)> = vec![("b1", 0, warmup, ticks)];
    for s in ["b2", "b2-blocked", "b2-comb", "b3"] {
        for &n in sizes {
            cases.push((s, n, warmup, ticks));
        }
    }
    // B5: бой не установившийся — марш и сражение, первые 1500 тиков без прогрева
    cases.push(("b5", if quick { 300 } else { 1000 }, 0, 1500));
    // B8: тот же бой со стройкой и сносом стен каждые 5 тиков (локальная починка поля путей)
    cases.push(("b8", if quick { 300 } else { 1000 }, 0, 1500));
    let mut results = Vec::new();
    println!(
        "{:<11} {:>7} {:>11} {:>11} {:>8} {:>9}",
        "scenario", "size", "mean_us", "p99_us", "alloc/t", "rss_mib"
    );
    for (name, n, warmup, ticks) in cases {
        let mut reports: Vec<serde_json::Value> = Vec::new();
        for _ in 0..runs.max(1) {
            let output = std::process::Command::new(&exe)
                .args(["bench", name, "--conveyors", &n.max(1).to_string()])
                .args([
                    "--ticks",
                    &ticks.to_string(),
                    "--warmup",
                    &warmup.to_string(),
                ])
                .output()
                .map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(format!(
                    "{name}: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            reports.push(serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?);
        }
        let median = |f: &dyn Fn(&serde_json::Value) -> f64| {
            let mut v: Vec<f64> = reports.iter().map(f).collect();
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let mean = median(&|r| r["tick"]["mean_us"].as_f64().unwrap_or(f64::NAN));
        let p99 = median(&|r| r["tick"]["p99_us"].as_f64().unwrap_or(f64::NAN));
        let alloc = median(&|r| r["allocations_per_tick"].as_f64().unwrap_or(f64::NAN));
        let rss = median(&|r| r["peak_rss_kib"].as_f64().unwrap_or(f64::NAN)) / 1024.0;
        println!("{name:<11} {n:>7} {mean:>11.2} {p99:>11.2} {alloc:>8.2} {rss:>9.1}");
        results.push(serde_json::json!({
            "scenario": name, "size": n, "median_mean_us": mean, "median_p99_us": p99,
            "median_allocations_per_tick": alloc, "runs": reports,
        }));
    }
    let doc =
        serde_json::json!({ "warmup_ticks": warmup, "measured_ticks": ticks, "results": results });
    let json = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    std::fs::write(out, json).map_err(|e| format!("{out}: {e}"))?;
    println!("\nfull report: {out}");
    Ok(())
}
