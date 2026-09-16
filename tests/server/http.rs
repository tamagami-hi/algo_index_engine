use super::*;

fn engine_with_frames() -> EngineState {
    EngineState::new()
}

#[tokio::test]
async fn the_browser_stream_coalesces_to_the_publish_interval() {
    let engine = engine_with_frames();
    let mut receiver = engine.subscribe();
    receiver.borrow_and_update();

    let shutdown = CancellationToken::new();
    let interval = Duration::from_millis(50);

    let driver = tokio::spawn({
        let engine = engine.clone();
        async move {
            for _ in 0..500 {
                engine.feed_subscribed(1);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    });

    let started = Instant::now();
    let mut publishes = 0;
    let mut last_publish = None;
    let mut sequences = Vec::new();

    while started.elapsed() < Duration::from_millis(500) {
        if !wait_to_publish(&mut receiver, last_publish, interval, &shutdown).await {
            break;
        }
        last_publish = Some(Instant::now());
        publishes += 1;
        sequences.push(receiver.borrow_and_update().sequence);
    }

    driver.abort();

    assert!(
        publishes <= 14,
        "500ms at a 50ms floor allows about ten publishes, got {publishes}"
    );
    assert!(
        publishes >= 5,
        "the stream must still be publishing, got {publishes}"
    );

    let mut ascending = sequences.clone();
    ascending.sort_unstable();
    assert_eq!(
        sequences, ascending,
        "each publish carries a newer state than the last: {sequences:?}"
    );
    assert!(
        sequences.windows(2).all(|pair| pair[1] > pair[0]),
        "no publish repeats a sequence already sent: {sequences:?}"
    );
    assert!(
        sequences.last().copied().unwrap_or(0) > publishes as u64,
        "intermediate states were skipped rather than queued: {sequences:?}"
    );
}

#[tokio::test]
async fn the_stream_waits_rather_than_republishing_an_unchanged_snapshot() {
    let engine = engine_with_frames();
    let mut receiver = engine.subscribe();
    receiver.borrow_and_update();

    let shutdown = CancellationToken::new();

    let idle = tokio::time::timeout(
        Duration::from_millis(200),
        wait_to_publish(&mut receiver, None, Duration::from_millis(50), &shutdown),
    )
    .await;

    assert!(
        idle.is_err(),
        "with no state change there is nothing to publish, so the stream must block"
    );
}

#[tokio::test]
async fn a_cancelled_server_ends_the_stream_instead_of_hanging_the_drain() {
    let engine = engine_with_frames();
    let mut receiver = engine.subscribe();
    receiver.borrow_and_update();

    let shutdown = CancellationToken::new();
    shutdown.cancel();

    let carry_on = wait_to_publish(&mut receiver, None, Duration::from_millis(50), &shutdown).await;
    assert!(
        !carry_on,
        "a cancelled token must end the stream so graceful shutdown can complete"
    );
}

#[tokio::test]
async fn the_publish_floor_is_honoured_even_when_changes_arrive_continuously() {
    let engine = engine_with_frames();
    let mut receiver = engine.subscribe();
    receiver.borrow_and_update();

    let shutdown = CancellationToken::new();
    let interval = Duration::from_millis(80);

    let driver = tokio::spawn({
        let engine = engine.clone();
        async move {
            loop {
                engine.feed_subscribed(2);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    });

    assert!(wait_to_publish(&mut receiver, None, interval, &shutdown).await);
    let first = Instant::now();
    assert!(wait_to_publish(&mut receiver, Some(first), interval, &shutdown).await);
    let gap = first.elapsed();

    driver.abort();

    assert!(
        gap >= interval,
        "consecutive publishes must be at least {interval:?} apart, was {gap:?}"
    );
}
