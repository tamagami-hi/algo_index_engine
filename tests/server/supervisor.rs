use super::*;

fn never_signalled() -> impl std::future::Future<Output = String> {
    std::future::pending()
}

#[tokio::test]
async fn a_task_that_returns_an_error_is_reported_with_its_name() {
    let mut supervisor = Supervisor::new();
    supervisor.spawn("engine loop", async {
        anyhow::bail!("the feed gave up");
    });

    match wait_for_stop(&mut supervisor, never_signalled()).await {
        Stop::Task { name, ended } => {
            assert_eq!(name, "engine loop");
            assert!(ended.is_fault(), "an error is a fault");
            assert!(
                ended.detail().contains("the feed gave up"),
                "the cause must survive: {}",
                ended.detail()
            );
        }
        other => panic!("expected a task stop, got {other:?}"),
    }
}

#[tokio::test]
async fn a_task_that_panics_is_reported_rather_than_silently_dying() {
    let mut supervisor = Supervisor::new();
    supervisor.spawn("engine loop", async {
        panic!("strike arithmetic overflowed");
    });

    match wait_for_stop(&mut supervisor, never_signalled()).await {
        Stop::Task { name, ended } => {
            assert_eq!(name, "engine loop");
            assert!(ended.is_fault());
            assert!(
                matches!(ended, Ended::Panicked(ref payload) if payload.contains("overflowed")),
                "the panic payload must be reported: {ended:?}"
            );
        }
        other => panic!("expected a task stop, got {other:?}"),
    }
}

#[tokio::test]
async fn a_panicking_task_is_noticed_without_waiting_for_a_signal() {
    let mut supervisor = Supervisor::new();
    supervisor.spawn("engine loop", async { panic!("dead") });
    supervisor.spawn("http server", async {
        std::future::pending::<()>().await;
        Ok(())
    });

    let stop = wait_for_stop(&mut supervisor, never_signalled()).await;
    assert!(
        matches!(&stop, Stop::Task { name, .. } if name == "engine loop"),
        "the dead engine must be seen even though the server is still happily running: {stop:?}"
    );

    drop(supervisor);
}

#[tokio::test]
async fn a_task_that_returns_cleanly_still_stops_the_service() {
    let mut supervisor = Supervisor::new();
    supervisor.spawn("engine loop", async { Ok(()) });

    match wait_for_stop(&mut supervisor, never_signalled()).await {
        Stop::Task { name, ended } => {
            assert_eq!(name, "engine loop");
            assert_eq!(
                ended,
                Ended::Returned,
                "a clean return is not a fault, but a critical worker leaving is still a stop"
            );
        }
        other => panic!("expected a task stop, got {other:?}"),
    }
}

#[tokio::test]
async fn an_external_signal_wins_when_every_task_is_healthy() {
    let mut supervisor = Supervisor::new();
    supervisor.spawn("engine loop", async {
        std::future::pending::<()>().await;
        Ok(())
    });
    supervisor.spawn("http server", async {
        std::future::pending::<()>().await;
        Ok(())
    });

    let stop = wait_for_stop(&mut supervisor, std::future::ready("SIGTERM".to_owned())).await;
    assert_eq!(stop, Stop::Signal("SIGTERM".to_owned()));

    drop(supervisor);
}

#[tokio::test]
async fn an_empty_supervisor_reports_drained_rather_than_hanging() {
    let mut supervisor = Supervisor::new();
    assert_eq!(
        wait_for_stop(&mut supervisor, never_signalled()).await,
        Stop::Drained,
        "with nothing left to supervise there is nothing to wait for"
    );
}

#[tokio::test]
async fn draining_reports_every_remaining_task() {
    let mut supervisor = Supervisor::new();
    supervisor.spawn("first", async { Ok(()) });
    supervisor.spawn("second", async { anyhow::bail!("second failed") });

    let ended = supervisor
        .drain_within(std::time::Duration::from_secs(5))
        .await;
    assert_eq!(ended.len(), 2);

    let names: Vec<&str> = ended.iter().map(|(name, _)| name.as_str()).collect();
    assert!(
        names.contains(&"first") && names.contains(&"second"),
        "{names:?}"
    );
    assert!(
        ended
            .iter()
            .any(|(name, ended)| name == "second" && ended.is_fault()),
        "the failure must not be lost in the drain: {ended:?}"
    );
}

#[tokio::test]
async fn a_task_that_ignores_cancellation_is_aborted_instead_of_hanging_shutdown() {
    let mut supervisor = Supervisor::new();
    supervisor.spawn("stubborn task", async {
        std::future::pending::<()>().await;
        Ok(())
    });

    let ended = supervisor
        .drain_within(std::time::Duration::from_millis(50))
        .await;
    assert_eq!(ended.len(), 1, "shutdown must not wait forever");
    assert_eq!(ended[0].0, "stubborn task");
    assert!(
        matches!(ended[0].1, Ended::Failed(ref detail) if detail == "cancelled"),
        "an aborted task is cancelled, not panicked: {:?}",
        ended[0].1
    );
}
