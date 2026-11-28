//! Benchmark and chaos harness (spec section 6).
//! Run: cargo run --release -p print-tools -- bench | chaos | calibration | license-bench
//! Every mode prints measured numbers and exits non-zero on any failed
//! invariant. Nothing here simulates a *result*: the invariants checked
//! are the real queue/worker code paths used by the app.

use print_core::worker::Outcome;
use print_core::{BackendError, Job, JobState, PrintBackend, Queue, Worker, WorkerConfig};
use std::collections::HashMap;
use std::time::Instant;

fn job(id: usize, printer: &str, prio: i64) -> Job {
    Job {
        id: format!("job-{id}"),
        idempotency_key: format!("bench-{id}"),
        printer: printer.into(),
        report: Some("bench.report".into()),
        pages: 1,
        bytes: 2048,
        priority: prio,
        state: JobState::Queued,
        attempts: 0,
        error_code: None,
    }
}

struct OkBackend {
    printed: HashMap<String, u32>,
}
impl PrintBackend for OkBackend {
    fn print(&mut self, job: &Job) -> Result<(), BackendError> {
        *self.printed.entry(job.id.clone()).or_default() += 1;
        Ok(())
    }
}

fn bench() {
    // Sustained: 1,000 jobs through enqueue + claim + worker + done.
    // The bench models a bulk-import terminal, so it raises the default
    // 500-active-jobs backpressure limit for its queue explicitly.
    let mut q = Queue::in_memory().unwrap();
    q.set_max_active_per_printer(5_000);
    let t0 = Instant::now();
    for i in 0..1000 {
        q.enqueue(&job(i, "bench-printer", 0)).unwrap();
    }
    let enqueue_ms = t0.elapsed().as_millis();
    let mut w = Worker {
        queue: &mut q,
        backend: OkBackend {
            printed: HashMap::with_capacity(1000),
        },
        cfg: WorkerConfig::default(),
        entropy: 0,
    };
    let t1 = Instant::now();
    let mut done = 0;
    while let Outcome::Done(_) = w.run_once("bench-printer").unwrap() {
        done += 1;
    }
    let process_ms = t1.elapsed().as_millis().max(1);
    println!("bench.sustained jobs={done} enqueue_ms={enqueue_ms} process_ms={process_ms}");
    println!(
        "bench.sustained rate_jobs_per_sec={}",
        (done as u128 * 1000) / process_ms
    );
    assert_eq!(done, 1000);
    assert_eq!(w.backend.printed.len(), 1000);
    assert!(
        w.backend.printed.values().all(|&n| n == 1),
        "duplicate print detected"
    );
    drop(w);

    // Burst: 200 jobs enqueued "in one minute" — here, as fast as possible,
    // which is strictly harder on the queue than the spec burst.
    let mut q2 = Queue::in_memory().unwrap();
    let t2 = Instant::now();
    for i in 0..200 {
        q2.enqueue(&job(10_000 + i, "burst", 10)).unwrap();
    }
    println!(
        "bench.burst jobs=200 enqueue_ms={}",
        t2.elapsed().as_millis()
    );
    println!("bench.result=PASS no_loss=1000/1000 no_duplicates=true");
}

/// Deterministic pseudo-random source (LCG) so chaos runs are reproducible.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

struct FlakyBackend {
    rng: Lcg,
    success_prints: HashMap<String, u32>,
}
impl PrintBackend for FlakyBackend {
    fn print(&mut self, job: &Job) -> Result<(), BackendError> {
        // ~30% transient failures (offline/busy class).
        if self.rng.next() % 10 < 3 {
            return Err(BackendError::retryable(
                "CHAOS_TRANSIENT",
                "injected transient failure",
            ));
        }
        *self.success_prints.entry(job.id.clone()).or_default() += 1;
        Ok(())
    }
}

fn chaos() {
    let dir = std::env::temp_dir().join(format!("np-chaos-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chaos.db").to_string_lossy().to_string();
    let total = 300usize;
    let mut successes: HashMap<String, u32> = HashMap::with_capacity(total);
    let mut crashes = 0u32;
    let mut guard = 0;
    loop {
        guard += 1;
        assert!(guard < 50, "chaos loop did not converge");
        // (Re)open the durable queue and run recovery, exactly like an app
        // restart after a crash.
        let mut q = Queue::open(&path).unwrap();
        if guard == 1 {
            for i in 0..total {
                q.enqueue(&job(i, "chaos-printer", (i % 3) as i64)).unwrap();
            }
        }
        let (requeued, ambiguous) = q.recover_on_startup(false).unwrap();
        if guard > 1 {
            crashes += 1;
            println!("chaos.restart #{guard} recovered_requeued={requeued} ambiguous={ambiguous}");
        }
        let mut w = Worker {
            queue: &mut q,
            backend: FlakyBackend {
                rng: Lcg(0xC0FFEE + guard),
                success_prints: HashMap::with_capacity(37),
            },
            cfg: WorkerConfig {
                max_attempts: 6,
                // Zero backoff here on purpose: simulated restarts
                // are near-instant, so any real delay would exhaust
                // the restart guard before it elapsed. Durable
                // backoff itself is covered by print-core unit
                // tests; chaos covers crash recovery and no-loss.
                backoff_base_ms: 0,
                backoff_max_ms: 0,
            },
            entropy: guard,
        };
        // Process a bounded slice, then "crash" (drop everything, reopen).
        for _ in 0..37 {
            match w.run_once("chaos-printer").unwrap() {
                Outcome::Idle => break,
                _ => continue,
            }
        }
        let prints = std::mem::take(&mut w.backend.success_prints);
        drop(w);
        for (k, v) in prints {
            *successes.entry(k).or_default() += v;
        }
        let remaining = q.count_by_state(JobState::Queued).unwrap();
        let done = q.count_by_state(JobState::Done).unwrap();
        let dead = q.count_by_state(JobState::DeadLetter).unwrap();
        let failed = q.count_by_state(JobState::Failed).unwrap();
        if remaining == 0 && done + dead + failed == total as u64 {
            println!(
                "chaos.final done={done} dead_letter={dead} failed={failed} restarts={crashes}"
            );
            // Invariants: every done job was successfully printed exactly
            // once; nothing lost (all jobs accounted for).
            assert_eq!(
                successes.len() as u64,
                done,
                "successful prints must equal done jobs"
            );
            assert!(
                successes.values().all(|&n| n == 1),
                "a job was printed successfully twice"
            );
            assert_eq!(done + dead + failed, total as u64);
            println!("chaos.result=PASS no_loss={total}/{total} no_duplicate_success=true");
            break;
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

fn calibration() {
    let pdf = print_render::calibration::calibration_pdf();
    let info = print_render::parse_pdf_info(&pdf).unwrap();
    println!(
        "calibration.pdf bytes={} pages={} size={:.1}x{:.1}mm",
        pdf.len(),
        info.pages.len(),
        info.pages[0].size.width_mm,
        info.pages[0].size.height_mm
    );
    let bm = print_render::calibration::calibration_thermal_80mm();
    let stream = print_raw::EscPos::new()
        .raster(&bm)
        .feed_lines(4)
        .cut_partial();
    println!(
        "calibration.thermal dots={}x{} escpos_bytes={}",
        bm.width_dots,
        bm.height_dots,
        stream.bytes().len()
    );
    println!("calibration.result=PASS");
}

const LICENSE_BENCH_PUBKEY: [u8; 32] = [
    234, 74, 108, 99, 226, 156, 82, 10, 190, 245, 80, 123, 19, 46, 197, 249, 149, 71, 118, 174,
    190, 190, 123, 146, 66, 30, 234, 105, 20, 70, 210, 44,
];
const LICENSE_BENCH_KEY: &str = "ITBAA1.eyJjIjoiSXRiYWEgQmVuY2giLCJvIjoiaHR0cHM6Ly9lcnAuaXRiYWEudGVzdCIsImV4cCI6MTgwMDAwMDAwMCwidCI6InBybyJ9.8-tHpWQC_tlQ5-Qb7s8OBYiKWMtzHxkVdm__ANWT7jCF4iENB0B5l24oJZz-dVOCZnd5hm8tnhOnq4MZ55_eBw";
const LICENSE_BENCH_ORIGIN: &str = "https://erp.itbaa.test";

fn license_bench() {
    const OPS: usize = 1000;
    let t0 = Instant::now();
    for _ in 0..OPS {
        let payload = print_core::license::verify_license_with_pubkey(
            Some(std::hint::black_box(LICENSE_BENCH_KEY)),
            std::hint::black_box(&LICENSE_BENCH_PUBKEY),
            LICENSE_BENCH_ORIGIN,
            1_750_000_000,
            1_749_999_900,
        )
        .expect("benchmark license must verify");
        std::hint::black_box(payload);
    }
    let elapsed = t0.elapsed();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let us_per_op = elapsed.as_secs_f64() * 1_000_000.0 / OPS as f64;
    assert!(
        us_per_op < 1000.0,
        "license verification too slow: {us_per_op:.2} us/op (budget < 1000 us/op)"
    );
    println!("license_bench.result=PASS ops={OPS} total_ms={total_ms:.2} us_per_op={us_per_op:.2}");
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("bench") => bench(),
        Some("chaos") => chaos(),
        Some("calibration") => calibration(),
        Some("license-bench") => license_bench(),
        _ => {
            eprintln!("usage: native-print-tools <bench|chaos|calibration|license-bench>");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_bench_fixture_verifies() {
        let payload = print_core::license::verify_license_with_pubkey(
            Some(LICENSE_BENCH_KEY),
            &LICENSE_BENCH_PUBKEY,
            LICENSE_BENCH_ORIGIN,
            1_750_000_000,
            1_749_999_900,
        )
        .expect("fixture must be a valid Ed25519 license");
        assert_eq!(payload.c, "Itbaa Bench");
        assert_eq!(payload.t, "pro");
    }
}
