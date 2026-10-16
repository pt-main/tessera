use std::hint::black_box;
use std::time::{Duration, Instant};

use tessera::generating::to_json;
use tessera::{
    errors::format_errors_traced,
    generating::to_tessera,
    parsing::{end_block, parse_into, start_block},
    structures::Config,
};

// ---------------------------------------------------------------------------
// Timing stats
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TimingStats {
    pub iters: usize,
    pub total: Duration,
    pub avg: Duration,
    pub min: Duration,
    pub max: Duration,
    pub median: Duration,
    pub p90: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub stddev: Duration,
    pub errors_last: usize,
    pub items_last: usize,
}

pub fn bench_parse(code: &str, iters: usize) -> TimingStats {
    assert!(iters > 0, "iters must be > 0");

    // Warmup — not measured.
    {
        let mut cfg = Config::default();
        let _ = parse_into(&mut cfg, code);
    }

    let mut samples: Vec<Duration> = Vec::with_capacity(iters);
    let mut errors_last = 0usize;
    let mut items_last = 0usize;

    for _ in 0..iters {
        let mut cfg = Config::default();
        let start = Instant::now();
        let errors = parse_into(&mut cfg, black_box(code));
        let elapsed = start.elapsed();

        black_box(&cfg);
        black_box(&errors);

        samples.push(elapsed);
        errors_last = errors.len();
        items_last = count_items(&cfg);
    }

    // Print errors once, outside the timing loop.
    if errors_last > 0 {
        let mut cfg = Config::default();
        let errors = parse_into(&mut cfg, code);
        for err in &errors {
            println!("  [err] {}", err);
        }
    }

    let (total, avg, min, max, median, p90, p95, p99, stddev) = summarize(&samples);
    TimingStats {
        iters,
        total,
        avg,
        min,
        max,
        median,
        p90,
        p95,
        p99,
        stddev,
        errors_last,
        items_last,
    }
}

fn count_items(cfg: &Config) -> usize {
    let n = cfg.ints.len()
        + cfg.strs.len()
        + cfg.floats.len()
        + cfg.bytes.len()
        + cfg.bools.len()
        + cfg.inners.len();
    for child in cfg.inners.values() {
        n + count_items(child);
    }
    n
}

fn summarize(samples: &[Duration]) -> (Duration, Duration, Duration, Duration, Duration, Duration, Duration, Duration, Duration) {
    assert!(!samples.is_empty());
    let total: Duration = samples.iter().copied().sum();
    let avg = total / samples.len() as u32;

    let mut sorted: Vec<Duration> = samples.to_vec();
    sorted.sort_unstable();

    let min = sorted.first().copied().unwrap();
    let max = sorted.last().copied().unwrap();
    let median = percentile(&sorted, 50.0);
    let p90 = percentile(&sorted, 90.0);
    let p95 = percentile(&sorted, 95.0);
    let p99 = percentile(&sorted, 99.0);

    let mean_secs = avg.as_secs_f64();
    let var = samples
        .iter()
        .map(|d| {
            let diff = d.as_secs_f64() - mean_secs;
            diff * diff
        })
        .sum::<f64>()
        / samples.len() as f64;
    let stddev = Duration::from_secs_f64(var.sqrt());

    (total, avg, min, max, median, p90, p95, p99, stddev)
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let n = sorted.len();
    let idx = ((n as f64 - 1.0) * p / 100.0).round() as usize;
    sorted[idx.min(n - 1)]
}

// ---------------------------------------------------------------------------
// Input stats & formatting
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct InputStats {
    pub bytes: usize,
    pub lines: usize,
    pub non_empty_lines: usize,
    pub words: usize,
}

impl InputStats {
    pub fn of(code: &str) -> Self {
        Self {
            bytes: code.len(),
            lines: code.lines().count(),
            non_empty_lines: code.lines().filter(|l| !l.trim().is_empty()).count(),
            words: code.split_whitespace().count(),
        }
    }
}

fn human_size(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.2} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.2} KiB", b / KIB)
    } else {
        format!("{} B", bytes)
    }
}

fn throughput_mibps(bytes: usize, d: Duration) -> f64 {
    let secs = d.as_secs_f64();
    if secs <= 0.0 {
        return f64::INFINITY;
    }
    (bytes as f64) / (1024.0 * 1024.0) / secs
}

fn units_per_sec(units: usize, d: Duration) -> f64 {
    let secs = d.as_secs_f64();
    if secs <= 0.0 {
        return f64::INFINITY;
    }
    units as f64 / secs
}

// ---------------------------------------------------------------------------
// Pretty printing
// ---------------------------------------------------------------------------

fn print_input(stats: InputStats) {
    println!("=== input ===");
    println!("  bytes           : {:>10}  ({})", stats.bytes, human_size(stats.bytes));
    println!("  lines           : {:>10}", stats.lines);
    println!("  non-empty lines : {:>10}", stats.non_empty_lines);
    println!("  whitespace words: {:>10}", stats.words);
}

fn print_timings(s: &TimingStats) {
    println!("\n=== timing (iters={}) ===", s.iters);
    println!("  total  : {:>12?}  ({:>6} ms)", s.total, s.total.as_millis());
    println!("  avg    : {:>12?}  ({:>6} ms)", s.avg, s.avg.as_millis());
    println!("  median : {:>12?}  ({:>6} ms)", s.median, s.median.as_millis());
    println!("  p90    : {:>12?}  ({:>6} ms)", s.p90, s.p90.as_millis());
    println!("  p95    : {:>12?}  ({:>6} ms)", s.p95, s.p95.as_millis());
    println!("  p99    : {:>12?}  ({:>6} ms)", s.p99, s.p99.as_millis());
    println!("  min    : {:>12?}  ({:>6} ms)", s.min, s.min.as_millis());
    println!("  max    : {:>12?}  ({:>6} ms)", s.max, s.max.as_millis());
    println!("  stddev : {:>12?}  ({:>6} ms)", s.stddev, s.stddev.as_millis());
    println!("  errors : {:>12}", s.errors_last);
    println!("  items  : {:>12}", s.items_last);
}

fn print_throughput(stats: InputStats, s: &TimingStats) {
    println!("\n=== throughput (based on median) ===");
    println!("  {:>10.2} MiB/s", throughput_mibps(stats.bytes, s.median));
    println!("  {:>10.0} lines/s", units_per_sec(stats.lines, s.median));
    println!("  {:>10.0} non-empty lines/s", units_per_sec(stats.non_empty_lines, s.median));
    println!("  {:>10.0} words/s", units_per_sec(stats.words, s.median));
    println!("  {:>10.0} items/s", units_per_sec(s.items_last, s.median));

    if s.errors_last > 0 && stats.lines > 0 {
        println!(
            "\n  error rate: {:.4}% ({}/{})",
            s.errors_last as f64 / stats.lines as f64 * 100.0,
            s.errors_last,
            stats.lines
        );
    }
}

// ---------------------------------------------------------------------------
// Synthetic input generator
// ---------------------------------------------------------------------------

fn build_synthetic_input(n: usize) -> String {
    use std::fmt::Write;
    let mut code = String::with_capacity(n * 16);
    for i in 0..n {
        let _ = writeln!(
            code,
            "
{start_block} {i}
    # comm
    key{i} i 0
    key{i}_1 i 1
    {start_block} in_sec
        key s test string
    {end_block}
{end_block}
",
            start_block = start_block,
            end_block = end_block,
            i = i,
        );
    }
    code
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let iters: usize = std::env::var("BENCH_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let n: usize = std::env::var("BENCH_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);

    println!("tessera parse benchmark");
    println!("  iters = {}", iters);
    println!("  blocks per input = {}", n);

    let code = build_synthetic_input(n);
    let stats = InputStats::of(&code);
    print_input(stats);

    let s = bench_parse(&code, iters);
    print_timings(&s);
    print_throughput(stats, &s);

    // -----------------------------------------------------------------------
    // Demo: error reporting on a small snippet
    // -----------------------------------------------------------------------
    println!("\n=== error reporting demo ===");
    let code = "
< test
    key i 0
    key f 1.0
    key s test
    < inner
        key l 1
        key b 0123
        this is error
    >
>
>
";
    let mut cfg = Config::default();
    let errors = parse_into(&mut cfg, code);
    println!("{}", format_errors_traced(&errors, code));
    println!("{}", to_tessera(&cfg));
    println!("{}", to_json(&cfg));
}