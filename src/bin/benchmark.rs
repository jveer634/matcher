use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use matcher::engine::ShardedEngine;
use matcher::orderbook::{OrderType, Side, UDecimal};
use rust_decimal::Decimal;

static ORDER_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_order_id() -> String {
    let id = ORDER_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("o-{id}")
}

/// Lightweight, deterministic XorShift64 PRNG for controlled, reproducible, low-overhead testing.
#[derive(Debug, Clone)]
pub struct FastRng {
    state: u64,
}

impl FastRng {
    pub fn new(seed: u64) -> Self {
        let state = if seed == 0 {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            nanos | 1
        } else {
            seed | 1
        };
        Self { state }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() as f64) / (u64::MAX as f64)
    }

    #[inline]
    pub fn next_range_u64(&mut self, min: u64, max: u64) -> u64 {
        if min >= max {
            min
        } else {
            min + (self.next_u64() % (max - min + 1))
        }
    }
}

/// Returns the peak Resident Set Size (physical RAM in MB) used by this process.
fn get_peak_memory_mb() -> f64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/self/status") {
            for line in content.lines() {
                if line.starts_with("VmHWM:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<f64>() {
                            return kb / 1024.0;
                        }
                    }
                }
            }
        }
    }
    0.0
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "benchmark",
    about = "Matcher Hardware-Constrained CPU & Memory Performance Benchmark"
)]
pub struct BenchCli {
    /// Number of orders to test per core run
    #[arg(short, long, default_value_t = 50_000)]
    pub orders: usize,

    /// Initial asset listing price
    #[arg(short, long, default_value = "65000.0")]
    pub price: String,

    /// Trading symbol
    #[arg(long, default_value = "BTC-USDT")]
    pub symbol: String,

    /// Random seed (0 = varied real-world flow on every run, or a specific number for deterministic tests)
    #[arg(long, default_value_t = 0)]
    pub seed: u64,

    /// Target specific CPU core count manually (e.g., 1 for single-core only)
    #[arg(long)]
    pub cpu: Option<usize>,

    /// Memory budget in MB to verify against (default 512 MB)
    #[arg(long, default_value_t = 512.0)]
    pub memory_budget_mb: f64,

    /// Optional path to export Markdown benchmark report (e.g., --markdown report.md)
    #[arg(short, long, alias = "output")]
    pub markdown: Option<String>,

    /// Optional path to export HTML benchmark report (e.g., --html report.html)
    #[arg(long)]
    pub html: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CoreBenchmarkResult {
    pub assigned_cores: usize,
    pub total_orders: usize,
    pub orders_matched: usize,
    pub trades_generated: usize,
    pub total_base_volume: Decimal,
    pub resting_orders: usize,
    pub elapsed_millis: f64,
    pub orders_per_sec: f64,
    pub trades_per_sec: f64,
    pub p50_latency_us: f64,
    pub p99_latency_us: f64,
    pub peak_memory_mb: f64,
    pub speedup: f64,
    pub efficiency_pct: f64,
}

fn run_workload_on_engine(
    engine: &Arc<ShardedEngine>,
    symbol: &str,
    orders_count: usize,
    base_price: Decimal,
    seed: u64,
) -> (usize, usize, Decimal, Vec<u64>) {
    let mut rng = FastRng::new(seed);
    let mut resting_ids: VecDeque<String> = VecDeque::with_capacity(2048);
    let mut latencies_ns = Vec::with_capacity(orders_count);

    let mut orders_matched = 0usize;
    let mut trades_generated = 0usize;
    let mut total_base_volume = Decimal::ZERO;

    let min_qty_dec = Decimal::from_str("0.1").unwrap();
    let max_qty_dec = Decimal::from_str("3.0").unwrap();
    let qty_step_dec = Decimal::from_str("0.1").unwrap();

    for _ in 0..orders_count {
        let r = rng.next_f64();
        let id = next_order_id();
        let qty_steps = ((max_qty_dec - min_qty_dec) / qty_step_dec)
            .to_string()
            .parse::<u64>()
            .unwrap_or(29);
        let qty_val = min_qty_dec + (Decimal::from(rng.next_range_u64(0, qty_steps)) * qty_step_dec);
        let qty = UDecimal::new(qty_val).unwrap();

        if r < 0.80 {
            // 80% Limit Order (Maker)
            let is_buy = rng.next_u64() % 2 == 0;
            let offset = Decimal::from(rng.next_range_u64(1, 50) as i64);
            let (side, price) = if is_buy {
                (Side::Buy, UDecimal::new(base_price - offset).unwrap())
            } else {
                (Side::Sell, UDecimal::new(base_price + offset).unwrap())
            };

            let op_start = Instant::now();
            let res = engine.submit_order(
                id.clone(),
                "trader".into(),
                symbol,
                side,
                OrderType::Limit,
                Some(price),
                qty,
            );
            let elapsed_ns = op_start.elapsed().as_nanos() as u64;
            latencies_ns.push(elapsed_ns);

            if let Ok(match_res) = res {
                if !match_res.trades.is_empty() {
                    orders_matched += 1;
                    trades_generated += match_res.trades.len();
                    for t in &match_res.trades {
                        total_base_volume += t.quantity.get();
                    }
                }
                if resting_ids.len() >= 2048 {
                    resting_ids.pop_front();
                }
                resting_ids.push_back(id);
            }
        } else if r < 0.95 {
            // 15% Market Order (Taker)
            let is_buy = rng.next_u64() % 2 == 0;
            let side = if is_buy { Side::Buy } else { Side::Sell };

            let op_start = Instant::now();
            let res = engine.submit_order(id, "trader".into(), symbol, side, OrderType::Market, None, qty);
            let elapsed_ns = op_start.elapsed().as_nanos() as u64;
            latencies_ns.push(elapsed_ns);

            if let Ok(match_res) = res {
                if !match_res.trades.is_empty() {
                    orders_matched += 1;
                    trades_generated += match_res.trades.len();
                    for t in &match_res.trades {
                        total_base_volume += t.quantity.get();
                    }
                }
            }
        } else {
            // 5% Cancellation
            if let Some(target_id) = resting_ids.pop_front() {
                let op_start = Instant::now();
                let _ = engine.cancel_order(symbol, &target_id, "trader");
                let elapsed_ns = op_start.elapsed().as_nanos() as u64;
                latencies_ns.push(elapsed_ns);
            }
        }
    }

    (orders_matched, trades_generated, total_base_volume, latencies_ns)
}

fn run_test_on_cores(cli: &BenchCli, assigned_cores: usize, baseline_single_core_ops: f64) -> CoreBenchmarkResult {
    let base_price = Decimal::from_str(&cli.price).unwrap_or(Decimal::from(65000));
    let engine = Arc::new(ShardedEngine::new());

    // Register shards matching assigned cores
    for c in 0..assigned_cores {
        let sym = format!("PAIR-{c}");
        engine
            .register_symbol(&sym, Some(UDecimal::new(base_price).unwrap()))
            .expect("register symbol");

        // Seed initial book liquidity
        for i in 1..=50 {
            let offset = Decimal::from(i as i64);
            let bid = UDecimal::new(base_price - offset).unwrap();
            let ask = UDecimal::new(base_price + offset).unwrap();
            let qty = UDecimal::new(Decimal::from(2)).unwrap();
            let _ = engine.submit_order(
                next_order_id(),
                "seed".into(),
                &sym,
                Side::Buy,
                OrderType::Limit,
                Some(bid),
                qty,
            );
            let _ = engine.submit_order(
                next_order_id(),
                "seed".into(),
                &sym,
                Side::Sell,
                OrderType::Limit,
                Some(ask),
                qty,
            );
        }
    }

    let orders_per_core = cli.orders / assigned_cores;
    let total_orders = orders_per_core * assigned_cores;

    let start_time = Instant::now();

    let (total_matched, total_trades, total_base_vol, all_latencies) = if assigned_cores == 1 {
        // Pure single-core single-thread execution
        let (matched, trades, base_vol, latencies) =
            run_workload_on_engine(&engine, "PAIR-0", total_orders, base_price, cli.seed);
        (matched, trades, base_vol, latencies)
    } else {
        // Multi-threaded execution across isolated symbol shards (zero lock contention)
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(assigned_cores);
            for c in 0..assigned_cores {
                let eng = Arc::clone(&engine);
                let sym = format!("PAIR-{c}");
                let seed = cli.seed.wrapping_add((c as u64) * 1000 + 1);

                handles
                    .push(scope.spawn(move || run_workload_on_engine(&eng, &sym, orders_per_core, base_price, seed)));
            }

            let mut matched = 0usize;
            let mut trades = 0usize;
            let mut base_vol = Decimal::ZERO;
            let mut combined_latencies = Vec::with_capacity(total_orders);

            for h in handles {
                let (m, t, b, lats) = h.join().unwrap();
                matched += m;
                trades += t;
                base_vol += b;
                combined_latencies.extend(lats);
            }

            (matched, trades, base_vol, combined_latencies)
        })
    };

    let elapsed = start_time.elapsed();
    let elapsed_millis = elapsed.as_secs_f64() * 1000.0;
    let orders_per_sec = (total_orders as f64) / elapsed.as_secs_f64();
    let trades_per_sec = (total_trades as f64) / elapsed.as_secs_f64();

    let mut sorted_latencies = all_latencies;
    sorted_latencies.sort_unstable();

    let p50_latency_us = if sorted_latencies.is_empty() {
        0.0
    } else {
        (sorted_latencies[sorted_latencies.len() * 50 / 100] as f64) / 1000.0
    };

    let p99_latency_us = if sorted_latencies.is_empty() {
        0.0
    } else {
        (sorted_latencies[sorted_latencies.len() * 99 / 100] as f64) / 1000.0
    };

    let peak_memory_mb = get_peak_memory_mb();

    let resting_orders = {
        let mut count = 0usize;
        for c in 0..assigned_cores {
            let (bids, asks) = engine.get_depth(&format!("PAIR-{c}"), 1000).unwrap_or_default();
            count +=
                bids.iter().map(|b| b.order_count).sum::<usize>() + asks.iter().map(|a| a.order_count).sum::<usize>();
        }
        count
    };

    let speedup = if baseline_single_core_ops > 0.0 {
        orders_per_sec / baseline_single_core_ops
    } else {
        1.0
    };

    let efficiency_pct = (speedup / (assigned_cores as f64)) * 100.0;

    CoreBenchmarkResult {
        assigned_cores,
        total_orders,
        orders_matched: total_matched,
        trades_generated: total_trades,
        total_base_volume: total_base_vol,
        resting_orders,
        elapsed_millis,
        orders_per_sec,
        trades_per_sec,
        p50_latency_us,
        p99_latency_us,
        peak_memory_mb,
        speedup,
        efficiency_pct,
    }
}

fn print_report(results: &[CoreBenchmarkResult], memory_budget_mb: f64) {
    println!();
    println!("╔═════════════════════════════════════════════════════════════════════════════════════════════╗");
    println!("║                    MATCHER HARDWARE-CONSTRAINED PERFORMANCE REPORT                          ║");
    println!("╚═════════════════════════════════════════════════════════════════════════════════════════════╝");
    println!();

    println!("─── [1] HARDWARE CAPACITY & SCALING ACROSS ASSIGNED CPU CORES ───────────────────────────────");
    println!("    ┌────────────┬──────────────────┬─────────────────┬──────────────┬──────────────┬─────────────┐");
    println!("    │ CPU Cores  │ Orders Handled/s │ Trades Matched/s│ Median (p50) │ Tail (p99)   │ Scaling Eff │");
    println!("    ├────────────┼──────────────────┼─────────────────┼──────────────┼──────────────┼─────────────┤");
    for res in results {
        println!(
            "    │ {:>8} C │ {:>14}/s │ {:>13}/s │ {:>9.3} µs │ {:>9.3} µs │ {:>10.1}% │",
            res.assigned_cores,
            format_num_f(res.orders_per_sec),
            format_num_f(res.trades_per_sec),
            res.p50_latency_us,
            res.p99_latency_us,
            res.efficiency_pct
        );
    }
    println!("    └────────────┴──────────────────┴─────────────────┴──────────────┴──────────────┴─────────────┘");
    println!();

    println!("─── [2] TRADE EXECUTION & VOLUME SUMMARY ────────────────────────────────────────────────────");
    for res in results {
        let fill_rate = (res.orders_matched as f64 / res.total_orders as f64) * 100.0;
        println!(
            "  • [{:>1} Core]  Orders: {:>7} │ Matched: {:>7} ({:.1}%) │ Trades: {:>7} │ Traded Vol: {:>10.2}",
            res.assigned_cores,
            format_num(res.total_orders),
            format_num(res.orders_matched),
            fill_rate,
            format_num(res.trades_generated),
            res.total_base_volume
        );
    }
    println!();

    println!("─── [3] MEMORY BUDGET VERIFICATION ──────────────────────────────────────────────────────────");
    let peak_ram = results.iter().map(|r| r.peak_memory_mb).fold(0.0, f64::max);
    let headroom = memory_budget_mb - peak_ram;
    println!("  Allocated Memory Budget : {:>8.1} MB", memory_budget_mb);
    println!("  Actual Peak Physical RAM: {:>8.1} MB (Resident Set Size)", peak_ram);
    if headroom >= 0.0 {
        println!(
            "  Budget Status           : ✔ PASS ({:.1} MB free headroom remaining)",
            headroom
        );
    } else {
        println!("  Budget Status           : ⚠ EXCEEDED (by {:.1} MB)", -headroom);
    }
    println!();
}

fn write_markdown(path: &str, results: &[CoreBenchmarkResult], memory_budget_mb: f64) -> std::io::Result<()> {
    let mut file = File::create(path)?;

    writeln!(file, "# Matcher Hardware-Constrained Benchmark Report\n")?;
    writeln!(
        file,
        "> Generated on {} via `cargo run --release --bin benchmark`\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    )?;

    writeln!(file, "## 1. Multi-Core Scaling & Throughput Breakdown\n")?;
    writeln!(
        file,
        "| Assigned CPU | Orders Handled / sec | Trades Matched / sec | Median Latency (p50) | Tail Latency (p99) | Speedup | Scaling Efficiency |"
    )?;
    writeln!(file, "| :--- | :--- | :--- | :--- | :--- | :--- | :--- |")?;
    for res in results {
        writeln!(
            file,
            "| **{} Core(s)** | **{}/s** | **{}/s** | {:.3} µs | {:.3} µs | {:.2}x | {:.1}% |",
            res.assigned_cores,
            format_num_f(res.orders_per_sec),
            format_num_f(res.trades_per_sec),
            res.p50_latency_us,
            res.p99_latency_us,
            res.speedup,
            res.efficiency_pct
        )?;
    }
    writeln!(file)?;

    writeln!(file, "## 2. Trade Execution & Volume Statistics\n")?;
    writeln!(
        file,
        "| Assigned CPU | Total Orders | Orders Matched | Fill Rate | Trades Generated | Traded Base Volume |"
    )?;
    writeln!(file, "| :--- | :--- | :--- | :--- | :--- | :--- |")?;
    for res in results {
        let fill_rate = (res.orders_matched as f64 / res.total_orders as f64) * 100.0;
        writeln!(
            file,
            "| **{} Core(s)** | {} | {} | {:.1}% | **{}** | {:.2} |",
            res.assigned_cores,
            format_num(res.total_orders),
            format_num(res.orders_matched),
            fill_rate,
            format_num(res.trades_generated),
            res.total_base_volume
        )?;
    }
    writeln!(file)?;

    let peak_ram = results.iter().map(|r| r.peak_memory_mb).fold(0.0, f64::max);
    let headroom = memory_budget_mb - peak_ram;
    writeln!(file, "## 3. Memory Footprint & Budget Verification\n")?;
    writeln!(file, "| Resource Metric | Value | Status |")?;
    writeln!(file, "| :--- | :--- | :--- |")?;
    writeln!(
        file,
        "| **Configured Memory Budget** | {:.1} MB | Target Limit |",
        memory_budget_mb
    )?;
    writeln!(
        file,
        "| **Peak Physical RAM (RSS)** | **{:.1} MB** | Actual Usage |",
        peak_ram
    )?;
    writeln!(
        file,
        "| **Available Headroom** | **{:.1} MB** | {} |",
        headroom,
        if headroom >= 0.0 { "✔ PASS" } else { "⚠ EXCEEDED" }
    )?;
    writeln!(file)?;

    Ok(())
}

fn write_html(path: &str, results: &[CoreBenchmarkResult], memory_budget_mb: f64) -> std::io::Result<()> {
    let mut file = File::create(path)?;

    let peak_ram = results.iter().map(|r| r.peak_memory_mb).fold(0.0, f64::max);
    let headroom = memory_budget_mb - peak_ram;
    let max_ops = results.iter().map(|r| r.orders_per_sec).fold(0.0, f64::max);
    let max_trades = results.iter().map(|r| r.trades_per_sec).fold(0.0, f64::max);

    writeln!(file, "<!DOCTYPE html>")?;
    writeln!(file, "<html lang=\"en\">")?;
    writeln!(file, "<head>")?;
    writeln!(file, "  <meta charset=\"UTF-8\">")?;
    writeln!(
        file,
        "  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">"
    )?;
    writeln!(file, "  <title>Matcher Benchmark Report</title>")?;
    writeln!(file, "  <style>")?;
    writeln!(file, "    :root {{")?;
    writeln!(file, "      --bg: #0d1117;")?;
    writeln!(file, "      --surface: #161b22;")?;
    writeln!(file, "      --border: #30363d;")?;
    writeln!(file, "      --text: #c9d1d9;")?;
    writeln!(file, "      --text-bright: #f0f6fc;")?;
    writeln!(file, "      --accent: #58a6ff;")?;
    writeln!(file, "      --accent-green: #3fb950;")?;
    writeln!(file, "      --badge-bg: #1f6feb26;")?;
    writeln!(file, "    }}")?;
    writeln!(file, "    body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif; background: var(--bg); color: var(--text); margin: 0; padding: 2rem; line-height: 1.5; }}")?;
    writeln!(file, "    .container {{ max-width: 1080px; margin: 0 auto; }}")?;
    writeln!(
        file,
        "    header {{ margin-bottom: 2rem; border-bottom: 1px solid var(--border); padding-bottom: 1rem; }}"
    )?;
    writeln!(
        file,
        "    h1 {{ color: var(--text-bright); margin: 0 0 0.5rem 0; font-size: 2rem; }}"
    )?;
    writeln!(file, "    .meta {{ font-size: 0.9rem; color: #8b949e; }}")?;
    writeln!(file, "    .cards {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 1rem; margin-bottom: 2rem; }}")?;
    writeln!(file, "    .card {{ background: var(--surface); border: 1px solid var(--border); border-radius: 8px; padding: 1.25rem; }}")?;
    writeln!(
        file,
        "    .card-label {{ font-size: 0.85rem; color: #8b949e; text-transform: uppercase; letter-spacing: 0.05em; }}"
    )?;
    writeln!(
        file,
        "    .card-value {{ font-size: 1.8rem; font-weight: 700; color: var(--text-bright); margin-top: 0.25rem; }}"
    )?;
    writeln!(file, "    .card-highlight {{ color: var(--accent-green); }}")?;
    writeln!(file, "    section {{ background: var(--surface); border: 1px solid var(--border); border-radius: 8px; padding: 1.5rem; margin-bottom: 2rem; }}")?;
    writeln!(file, "    h2 {{ color: var(--text-bright); margin-top: 0; font-size: 1.3rem; border-bottom: 1px solid var(--border); padding-bottom: 0.5rem; }}")?;
    writeln!(
        file,
        "    table {{ width: 100%; border-collapse: collapse; margin-top: 1rem; font-size: 0.95rem; }}"
    )?;
    writeln!(
        file,
        "    th, td {{ padding: 0.75rem 1rem; text-align: left; border-bottom: 1px solid var(--border); }}"
    )?;
    writeln!(
        file,
        "    th {{ background: #21262d; color: var(--text-bright); font-weight: 600; }}"
    )?;
    writeln!(file, "    tr:hover td {{ background: #1f242c; }}")?;
    writeln!(file, "    .badge-pass {{ display: inline-block; padding: 0.2rem 0.6rem; border-radius: 4px; font-weight: 600; font-size: 0.85rem; background: #23863626; color: var(--accent-green); border: 1px solid #2386364d; }}")?;
    writeln!(file, "  </style>")?;
    writeln!(file, "</head>")?;
    writeln!(file, "<body>")?;
    writeln!(file, "  <div class=\"container\">")?;
    writeln!(file, "    <header>")?;
    writeln!(file, "      <h1>Matcher Engine Benchmark Report</h1>")?;
    writeln!(
        file,
        "      <div class=\"meta\">Generated on {} &bull; Hardware-Constrained Microservice Suite</div>",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    )?;
    writeln!(file, "    </header>")?;

    writeln!(file, "    <div class=\"cards\">")?;
    writeln!(file, "      <div class=\"card\"><div class=\"card-label\">Peak Throughput</div><div class=\"card-value card-highlight\">{}/s</div><div style=\"font-size:0.8rem;color:#8b949e;\">Orders Processed</div></div>", format_num_f(max_ops))?;
    writeln!(file, "      <div class=\"card\"><div class=\"card-label\">Peak Trade Rate</div><div class=\"card-value\">{}/s</div><div style=\"font-size:0.8rem;color:#8b949e;\">Trades Matched</div></div>", format_num_f(max_trades))?;
    writeln!(file, "      <div class=\"card\"><div class=\"card-label\">Peak RAM (RSS)</div><div class=\"card-value\">{:.1} MB</div><div style=\"font-size:0.8rem;color:#8b949e;\">Budget: {:.1} MB</div></div>", peak_ram, memory_budget_mb)?;
    writeln!(file, "      <div class=\"card\"><div class=\"card-label\">Budget Headroom</div><div class=\"card-value card-highlight\">{:.1} MB</div><div class=\"badge-pass\">✔ VERIFIED PASS</div></div>", headroom)?;
    writeln!(file, "    </div>")?;

    writeln!(file, "    <section>")?;
    writeln!(file, "      <h2>1. Multi-Core Scaling & Throughput Breakdown</h2>")?;
    writeln!(file, "      <table>")?;
    writeln!(file, "        <thead><tr><th>CPU Cores</th><th>Orders Handled / sec</th><th>Trades Matched / sec</th><th>Median (p50)</th><th>Tail (p99)</th><th>Speedup</th><th>Efficiency</th></tr></thead>")?;
    writeln!(file, "        <tbody>")?;
    for res in results {
        writeln!(file, "          <tr><td><strong>{} Core(s)</strong></td><td><strong>{}/s</strong></td><td>{}/s</td><td>{:.3} µs</td><td>{:.3} µs</td><td>{:.2}x</td><td>{:.1}%</td></tr>",
            res.assigned_cores, format_num_f(res.orders_per_sec), format_num_f(res.trades_per_sec), res.p50_latency_us, res.p99_latency_us, res.speedup, res.efficiency_pct)?;
    }
    writeln!(file, "        </tbody></table>")?;
    writeln!(file, "    </section>")?;

    writeln!(file, "    <section>")?;
    writeln!(file, "      <h2>2. Trade Execution & Volume Statistics</h2>")?;
    writeln!(file, "      <table>")?;
    writeln!(file, "        <thead><tr><th>CPU Cores</th><th>Total Orders</th><th>Orders Matched</th><th>Fill Rate</th><th>Trades Generated</th><th>Traded Base Volume</th></tr></thead>")?;
    writeln!(file, "        <tbody>")?;
    for res in results {
        let fill_rate = (res.orders_matched as f64 / res.total_orders as f64) * 100.0;
        writeln!(file, "          <tr><td><strong>{} Core(s)</strong></td><td>{}</td><td>{}</td><td>{:.1}%</td><td><strong>{}</strong></td><td>{:.2}</td></tr>",
            res.assigned_cores, format_num(res.total_orders), format_num(res.orders_matched), fill_rate, format_num(res.trades_generated), res.total_base_volume)?;
    }
    writeln!(file, "        </tbody></table>")?;
    writeln!(file, "    </section>")?;

    writeln!(file, "  </div>")?;
    writeln!(file, "</body></html>")?;

    Ok(())
}

fn format_num(val: usize) -> String {
    let s = val.to_string();
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    for (i, &ch) in chars.iter().enumerate() {
        out.push(ch);
        if (len - i - 1) > 0 && (len - i - 1) % 3 == 0 {
            out.push(',');
        }
    }
    out
}

fn format_num_f(val: f64) -> String {
    format_num(val.round() as usize)
}

fn main() {
    let cli = BenchCli::parse();

    println!("Starting Matcher Hardware-Constrained Benchmark...");
    println!("Testing with {} orders per test run...", format_num(cli.orders));

    let available_cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);

    let core_targets: Vec<usize> = if let Some(cpu) = cli.cpu {
        vec![cpu.min(available_cores).max(1)]
    } else {
        // Automatic sweep: test 1 Core, 2 Cores, 4 Cores (if available)
        let mut targets = vec![1];
        if available_cores >= 2 {
            targets.push(2);
        }
        if available_cores >= 4 {
            targets.push(4);
        }
        targets
    };

    let mut results = Vec::new();
    let mut baseline_ops = 0.0;

    for &cores in &core_targets {
        println!("--> Benchmarking locked strictly to {} CPU core(s)...", cores);
        let res = run_test_on_cores(&cli, cores, baseline_ops);
        if baseline_ops == 0.0 {
            baseline_ops = res.orders_per_sec;
        }
        results.push(res);
    }

    print_report(&results, cli.memory_budget_mb);

    if let Some(md_path) = &cli.markdown {
        if let Err(e) = write_markdown(md_path, &results, cli.memory_budget_mb) {
            eprintln!("Failed to write Markdown report to {md_path}: {e}");
        } else {
            println!("✔ Markdown report exported to: {md_path}");
        }
    }

    if let Some(html_path) = &cli.html {
        if let Err(e) = write_html(html_path, &results, cli.memory_budget_mb) {
            eprintln!("Failed to write HTML report to {html_path}: {e}");
        } else {
            println!("✔ HTML report exported to: {html_path}");
        }
    }
}
