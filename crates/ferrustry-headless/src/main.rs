//! Headless-запуск Ferrustry: сценарии, бенчмарки и проверка сохранений.

mod alloc;

use std::process::ExitCode;
use std::time::Instant;

use ferrustry_content::ContentRegistry;
use ferrustry_format::{read_save, write_save};
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
  ferrustry-headless bench <b1|b2> [--ticks N] [--warmup N] [--conveyors N] [--out FILE]
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
        "bench" => {
            let name = args.positional().ok_or("bench requires a scenario name")?;
            let ticks = args.number("--ticks", 3000)?;
            let warmup = args.number("--warmup", 300)?;
            let conveyors = args.number("--conveyors", 10_000)?;
            let out = args.value("--out")?;
            args.finish()?;
            let report = bench(&reg, &name, ticks, warmup, conveyors)?;
            let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            println!("{json}");
            if let Some(path) = out {
                std::fs::write(&path, json).map_err(|e| format!("{path}: {e}"))?;
            }
            Ok(())
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
    warmup_ticks: u64,
    measured_ticks: u64,
    buildings: usize,
    conveyor_segments: usize,
    items_on_conveyors: u64,
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
) -> Result<Report, String> {
    let setup = Instant::now();
    let mut w = match name {
        "b1" => scenario::bench_empty(reg),
        "b2" => scenario::bench_conveyor_loops(
            reg,
            u32::try_from(conveyors).map_err(|e| e.to_string())?,
        ),
        other => return Err(format!("unknown benchmark `{other}` (available: b1, b2)")),
    }
    .map_err(|e| e.to_string())?;
    let setup_ms = setup.elapsed().as_secs_f64() * 1e3;

    for _ in 0..warmup {
        w.step(reg);
    }
    let mut samples: Vec<f64> = Vec::with_capacity(usize::try_from(ticks).unwrap_or(0));
    let allocs_before = alloc::count();
    for _ in 0..ticks {
        let t = Instant::now();
        w.step(reg);
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
        warmup_ticks: warmup,
        measured_ticks: ticks,
        buildings: w.buildings().len(),
        conveyor_segments: w.segments().count(),
        items_on_conveyors: w.items_on_conveyors().iter().sum(),
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

/// Пиковый RSS (Linux). На других ОС пока не измеряется.
fn peak_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

fn cpu_model() -> Option<String> {
    let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let line = info.lines().find(|l| l.starts_with("model name"))?;
    Some(line.split(':').nth(1)?.trim().to_owned())
}
