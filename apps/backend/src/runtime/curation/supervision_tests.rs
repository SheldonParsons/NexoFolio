//! Reproduce the work/heartbeat lock dependency with no model calls.
use super::{CurationError, jobs::Jobs, supervision::supervise};
use nexofolio_common::ProjectId;
use std::{future::pending, time::Duration};

#[tokio::test]
async fn saving_can_commit_while_heartbeat_waits_for_its_lock() {
    let (job, _) = Jobs::default().start(ProjectId::new(), 1);
    let lock = tokio::sync::Mutex::new(());
    let (started, wait) = tokio::sync::oneshot::channel();
    let (saved, committed) = tokio::sync::oneshot::channel();
    let run = async {
        let transaction = lock.lock().await;
        wait.await.unwrap();
        drop(transaction);
        committed.await.unwrap();
        Ok(())
    };
    let heartbeat = async {
        started.send(()).unwrap();
        let _transaction = lock.lock().await;
        saved.send(()).unwrap();
        pending::<Result<(), CurationError>>().await
    };
    tokio::time::timeout(
        Duration::from_secs(2),
        supervise(&job, run, heartbeat, Duration::from_secs(900)),
    )
    .await
    .expect("heartbeat must not suspend the lock holder")
    .unwrap();
}

#[tokio::test]
async fn cancellation_remains_available_during_heartbeat_wait() {
    let (job, _) = Jobs::default().start(ProjectId::new(), 1);
    let lock = tokio::sync::Mutex::new(());
    let run = async {
        let _transaction = lock.lock().await;
        pending::<Result<(), CurationError>>().await
    };
    let heartbeat = async {
        job.cancel();
        let _transaction = lock.lock().await;
        pending::<Result<(), CurationError>>().await
    };
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        supervise(&job, run, heartbeat, Duration::from_secs(900)),
    )
    .await
    .expect("supervision must still respond");
    assert!(matches!(result, Err(CurationError::Failed(_))));
    assert!(
        lock.try_lock().is_ok(),
        "cancelled work must release its transaction"
    );
}

#[tokio::test(start_paused = true)]
async fn continued_progress_runs_past_both_old_two_and_six_hour_limits() {
    let (job, _) = Jobs::default().start(ProjectId::new(), 1);
    let started = tokio::time::Instant::now();
    let run = async {
        for _ in 0..48 {
            tokio::time::sleep(Duration::from_secs(600)).await;
            job.update(|p| p.completed += 1);
        }
        Ok(())
    };
    tokio::time::timeout(
        Duration::from_secs(9 * 3600),
        supervise(&job, run, pending(), Duration::from_secs(900)),
    )
    .await
    .expect("virtual run must finish")
    .expect("elapsed duration must not stop advancing work");
    assert_eq!(job.snapshot().completed, 48);
    assert!(started.elapsed() >= Duration::from_secs(8 * 3600));
}

#[test]
fn heartbeat_does_not_masquerade_as_processing_progress() {
    let (job, _) = Jobs::default().start(ProjectId::new(), 63);
    job.prepare_start(63);
    let before = job.snapshot();
    job.heartbeat();
    let after = job.snapshot();
    assert!(after.heartbeat_at.is_some());
    assert_eq!(after.updated_at, before.updated_at);
    assert_eq!(after.activity.len(), before.activity.len());
    assert_eq!(after.preparation.unwrap().endpoints_read, 0);
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn postgres_commit_is_polled_while_heartbeat_waits_on_project_lock() {
    let url = std::env::var("TEST_DATABASE_URL").unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let project = uuid::Uuid::new_v4().to_string();
    let (job, _) = Jobs::default().start(ProjectId::new(), 1);
    let (started, wait) = tokio::sync::oneshot::channel();
    let (saved, committed) = tokio::sync::oneshot::channel();
    let mut transaction = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2))")
        .bind(0x6b6e0001_i32)
        .bind(&project)
        .execute(&mut *transaction)
        .await
        .unwrap();
    let run = async {
        wait.await.unwrap();
        // Hold the transaction until the heartbeat has entered its lock wait.
        tokio::time::sleep(Duration::from_millis(30)).await;
        transaction.commit().await.unwrap();
        committed.await.unwrap();
        Ok(())
    };
    let heartbeat = async {
        let mut transaction = pool.begin().await.unwrap();
        started.send(()).unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2))")
            .bind(0x6b6e0001_i32)
            .bind(&project)
            .execute(&mut *transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        saved.send(()).unwrap();
        pending::<Result<(), CurationError>>().await
    };
    tokio::time::timeout(
        Duration::from_secs(5),
        supervise(&job, run, heartbeat, Duration::from_secs(900)),
    )
    .await
    .expect("database save and heartbeat deadlocked")
    .unwrap();
    pool.close().await;
}

#[tokio::test]
async fn live_heartbeats_do_not_disable_the_no_progress_timeout() {
    let (job, _) = Jobs::default().start(ProjectId::new(), 1);
    let heartbeat = async {
        loop {
            job.heartbeat();
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        #[allow(unreachable_code)]
        Ok(())
    };
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        supervise(&job, pending(), heartbeat, Duration::from_millis(20)),
    )
    .await
    .unwrap();
    assert!(
        matches!(result,Err(CurationError::Failed(message)) if message.contains("没有处理进展"))
    );
}
