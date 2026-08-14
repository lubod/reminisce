use reminisce::system_utils::*;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn test_system_utils_calculations() {
    let cpu_count = get_cpu_count();
    assert!(cpu_count > 0);

    let _load = get_load_average().await;
    let _gpu = get_gpu_load().await;

    // Batch size adjustments
    assert_eq!(adjust_batch_size(3.5), 0);
    assert_eq!(adjust_batch_size(2.5), 1);
    assert_eq!(adjust_batch_size(1.5), 2);
    assert_eq!(adjust_batch_size(0.5), 3);

    // Concurrency limits under normal load
    let normal_limits = calculate_worker_concurrency(0.5, 10, 4);
    assert!(!normal_limits.is_overloaded());
    assert!(normal_limits.verification >= 2);
    assert!(normal_limits.embedding >= 1);
    assert!(normal_limits.face_detection >= 1);
    assert!(normal_limits.description >= 1);
    assert!(!normal_limits.gpu_overloaded);

    // Concurrency limits under high GPU load
    let high_gpu_limits = calculate_worker_concurrency(0.5, 95, 4);
    assert!(high_gpu_limits.gpu_overloaded);

    // Concurrency limits under extreme system load
    let overloaded_limits = calculate_worker_concurrency(10.0, 0, 4);
    assert!(overloaded_limits.is_overloaded());
    assert_eq!(overloaded_limits.verification, 0);

    // Parallel batch size calculations
    assert_eq!(calculate_parallel_batch_size(5, 10.0, 4), 0);
    assert!(calculate_parallel_batch_size(5, 0.5, 4) >= 3);
    assert!(calculate_parallel_batch_size(5, 3.5, 4) >= 3);
    assert!(calculate_parallel_batch_size(5, 4.5, 4) >= 3);
}

#[tokio::test]
async fn test_run_worker_loop_lifecycle() {
    let token = CancellationToken::new();
    let token_clone = token.clone();

    let mut tick_count = 0;
    let worker_handle = tokio::spawn(async move {
        run_worker_loop(
            "test_worker",
            Duration::from_millis(10),
            Duration::from_millis(50),
            token_clone,
            move || {
                tick_count += 1;
                async move {
                    if tick_count == 1 {
                        Ok(true) // did work
                    } else if tick_count == 2 {
                        Ok(false) // idle
                    } else {
                        Err("simulated error".to_string())
                    }
                }
            },
        ).await;
    });

    tokio::time::sleep(Duration::from_millis(80)).await;
    token.cancel();

    let res = tokio::time::timeout(Duration::from_secs(2), worker_handle).await;
    assert!(res.is_ok(), "worker loop exited cleanly on cancellation");
}
