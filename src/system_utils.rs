use log::{error, info};
use std::future::Future;
use std::time::Duration;
use sysinfo::{System, SystemExt};
use tokio::time::sleep;

pub async fn get_load_average() -> f64 {
    let sys = System::new_all();
    sys.load_average().one
}

pub async fn get_gpu_load() -> u32 {
    match tokio::fs::read_to_string("/sys/class/drm/card0/device/gpu_busy_percent").await {
        Ok(s) => s.trim().parse().unwrap_or(0),
        Err(_) => 0,
    }
}

pub fn get_cpu_count() -> usize {
    let sys = System::new_all();
    sys.cpus().len()
}

pub fn adjust_batch_size(load_average: f64) -> i64 {
    if load_average > 3.0 {
        0
    } else if load_average > 2.0 {
        1
    } else if load_average > 1.0 {
        2
    } else {
        3
    }
}

/// Weighted concurrency limits for all worker types.
/// Priority order: verification > embedding > face_detection > description
#[derive(Debug, Clone, Copy)]
pub struct WorkerConcurrencyLimits {
    pub verification: usize,
    pub embedding: usize,
    pub face_detection: usize,
    pub description: usize,
    pub gpu_overloaded: bool,
}

impl WorkerConcurrencyLimits {
    pub fn is_overloaded(&self) -> bool {
        self.verification == 0
    }
}

/// Calculate weighted concurrency limits based on system load, GPU load and CPU count.
pub fn calculate_worker_concurrency(load_average: f64, gpu_load: u32, cpu_count: usize) -> WorkerConcurrencyLimits {
    let normalized_load = load_average / (cpu_count as f64).max(1.0);
    let gpu_overloaded = gpu_load > 90;

    if normalized_load > 1.2 {
        info!("System overloaded ({:.0}% normalized), pausing all workers", normalized_load * 100.0);
        return WorkerConcurrencyLimits {
            verification: 0,
            embedding: 0,
            face_detection: 0,
            description: 0,
            gpu_overloaded,
        };
    }

    let base = ((cpu_count as f64) * 0.7).max(1.0);

    let load_multiplier = if normalized_load > 0.9 {
        0.5
    } else if normalized_load > 0.7 {
        0.75
    } else if normalized_load > 0.5 {
        0.9
    } else {
        1.0
    };

    let ai_multiplier = if gpu_load > 80 {
        0.3
    } else if gpu_load > 50 {
        0.6
    } else {
        1.0
    };

    let available = (base * load_multiplier).max(1.0);

    WorkerConcurrencyLimits {
        verification: ((available * 1.5).ceil() as usize).min(16).max(2),
        embedding: ((available * ai_multiplier).ceil() as usize).min(10).max(1),
        face_detection: ((available * 0.75 * ai_multiplier).ceil() as usize).min(8).max(1),
        description: ((available * 0.25 * ai_multiplier).ceil().max(1.0) as usize).min(4).max(1),
        gpu_overloaded,
    }
}

/// Calculate optimal batch size for parallel processing.
pub fn calculate_parallel_batch_size(concurrency: usize, load_average: f64, cpu_count: usize) -> i64 {
    let normalized_load = load_average / (cpu_count as f64).max(1.0);

    if normalized_load > 1.5 {
        return 0;
    }

    let multiplier = if normalized_load > 1.0 {
        2
    } else if normalized_load > 0.7 {
        3
    } else {
        5
    };

    ((concurrency * multiplier) as i64).max(3).min(50)
}

/// Generic helper for adaptive worker loops with exponential backoff.
///
/// Returns `Ok(true)` if work was done (resets to `min_interval`),
/// `Ok(false)` if idle (doubles interval up to `max_interval`),
/// or `Err` which also triggers backoff.
pub async fn run_worker_loop<F, Fut>(
    name: &str,
    min_interval: Duration,
    max_interval: Duration,
    mut task: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool, String>>,
{
    let mut current_interval = min_interval;

    loop {
        match task().await {
            Ok(did_work) => {
                if did_work {
                    current_interval = min_interval;
                } else {
                    current_interval = (current_interval * 2).min(max_interval);
                }
            }
            Err(e) => {
                error!("Worker '{}' failed: {}", name, e);
                current_interval = (current_interval * 2).min(max_interval);
            }
        }

        sleep(current_interval).await;
    }
}
