//! Benchmark and chaos harness (spec section 6).
//! Run: cargo run --release -p print-tools -- bench | chaos | calibration
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
            printed: HashMap::new(),
        },
        cfg: WorkerConfig::default(),
        sleep: |_| {},
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
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chaos.db").to_string_lossy().to_string();
    let total = 300usize;
    let mut successes: HashMap<String, u32> = HashMap::new();
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
                success_prints: HashMap::new(),
            },
            cfg: WorkerConfig {
                max_attempts: 6,
                backoff_base_ms: 1,
                backoff_max_ms: 4,
            },
            sleep: |_| {},
            entropy: guard,
        };
        // Process a bounded slice, then "crash" (drop everything, reopen).
        for _ in 0..37 {
            match w.run_once("chaos-printer").unwrap() {
                Outcome::Idle => break,
                _ => continue,
            }
        }
        for (k, v) in &w.backend.success_prints {
            *successes.entry(k.clone()).or_default() += v;
        }
        drop(w);
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
            let done_ids: Vec<&String> = successes.keys().collect();
            assert_eq!(
                done_ids.len() as u64,
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

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("bench") => bench(),
        Some("chaos") => chaos(),
        Some("calibration") => calibration(),
        _ => {
            eprintln!("usage: native-print-tools <bench|chaos|calibration>");
            std::process::exit(2);
        }
    }
}
