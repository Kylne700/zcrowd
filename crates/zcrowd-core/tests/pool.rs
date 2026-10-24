//! The connection pool that raw transactions are fetched over: connections are fakes numbered in the order they are made.
use anyhow::anyhow;
use futures::FutureExt;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Duration;
use zcrowd_core::source::{Connect, Pool, call_pooled};

const FOREVER: Duration = Duration::from_secs(3600);

/// A pool of `size` connections, 0 first. `made` counts the connections made, first one included; one in `fail` fails.
async fn pool_where(size: usize, max_age: Duration, fail: fn(usize) -> bool) -> (Pool<usize>, Arc<AtomicUsize>) {
    let made = Arc::new(AtomicUsize::new(1));
    let attempts = made.clone();
    let connect: Connect<usize> = Box::new(move || {
        let n = attempts.fetch_add(1, Relaxed);
        async move { if fail(n) { Err(anyhow!("no route")) } else { Ok(n) } }.boxed()
    });
    (Pool::start(size, max_age, 0, connect).await, made)
}

async fn pool(size: usize, max_age: Duration) -> (Pool<usize>, Arc<AtomicUsize>) {
    pool_where(size, max_age, |_| false).await
}

fn count(conns: impl Iterator<Item = usize>) -> BTreeMap<usize, usize> {
    let mut m = BTreeMap::new();
    for c in conns {
        *m.entry(c).or_insert(0) += 1;
    }
    m
}

#[tokio::test]
async fn requests_in_flight_together_are_spread_over_the_connections() {
    let (p, made) = pool(4, FOREVER).await;
    assert_eq!(made.load(Relaxed), 4);
    let mut held = Vec::new();
    for _ in 0..8 {
        held.push(p.get().await.unwrap());
    }
    // two on each; one connection for all of them is how a sync ran before
    assert_eq!(count(held.iter().map(|l| l.conn)), BTreeMap::from([(0, 2), (1, 2), (2, 2), (3, 2)]));
    assert_eq!(made.load(Relaxed), 4, "nothing to replace");
}

#[tokio::test]
async fn a_connection_that_is_still_busy_gets_no_new_request_while_others_are_free() {
    let (p, _) = pool(4, FOREVER).await;
    let slow = p.get().await.unwrap(); // its request goes on
    let used: Vec<usize> = {
        let mut v = Vec::new();
        for _ in 0..9 {
            v.push(p.get().await.unwrap().conn); // each request ends at once
        }
        v
    };
    assert_eq!(count(used.into_iter()), BTreeMap::from([(1, 3), (2, 3), (3, 3)]), "slow one was {}", slow.conn);
}

#[tokio::test]
async fn a_connection_given_up_on_is_not_used_again_and_is_replaced() {
    let (p, made) = pool(2, FOREVER).await;
    let bad = p.get().await.unwrap();
    assert_eq!(bad.conn, 0);
    bad.give_up();
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(p.get().await.unwrap());
    }
    assert_eq!(count(held.iter().map(|l| l.conn)), BTreeMap::from([(1, 2), (2, 2)]));
    assert_eq!(made.load(Relaxed), 3, "one new connection, made once");
    assert_eq!(bad.conn, 0, "the request that was running keeps its connection");
}

#[tokio::test]
async fn a_connection_that_is_too_old_is_replaced_at_its_next_use_one_at_a_time() {
    let (p, made) = pool(4, Duration::from_millis(50)).await;
    tokio::time::sleep(Duration::from_millis(300)).await; // all four are old now
    for n in 1..=3 {
        drop(p.get().await.unwrap());
        assert_eq!(made.load(Relaxed), 4 + n, "one connection per use");
    }
    // the pool stays the size it was: the replaced ones are gone
    let mut held = Vec::new();
    for _ in 0..8 {
        held.push(p.get().await.unwrap());
    }
    assert_eq!(count(held.iter().map(|l| l.conn)).len(), 4);
}

#[tokio::test]
async fn connections_that_are_not_old_are_kept() {
    let (p, made) = pool(4, FOREVER).await;
    for _ in 0..20 {
        drop(p.get().await.unwrap());
    }
    assert_eq!(made.load(Relaxed), 4);
}

#[tokio::test]
async fn the_connections_of_a_pool_do_not_all_grow_old_at_once() {
    // ages at the start are 0, 1, 2 and 3 s of 4: only the last is old after 1.1 s
    let (p, made) = pool(4, Duration::from_secs(4)).await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    drop(p.get().await.unwrap());
    assert_eq!(made.load(Relaxed), 5, "the oldest is replaced");
    for _ in 0..10 {
        drop(p.get().await.unwrap());
    }
    assert_eq!(made.load(Relaxed), 5, "the others are not");
}

#[tokio::test]
async fn a_connection_that_cannot_be_made_is_not_tried_at_every_request() {
    let (p, made) = pool_where(3, FOREVER, |_| true).await;
    assert_eq!(made.load(Relaxed), 3, "the two extra ones were tried at the start");
    for _ in 0..10 {
        assert_eq!(p.get().await.unwrap().conn, 0, "the pool works with the one it has");
    }
    assert_eq!(made.load(Relaxed), 4, "one more try, then none for a while");
}

#[tokio::test]
async fn a_pool_with_no_connection_left_tries_again_and_recovers() {
    let (p, made) = pool_where(1, FOREVER, |n| n == 1).await;
    p.get().await.unwrap().give_up();
    assert!(p.get().await.is_err(), "the new connection failed and there is none");
    assert_eq!(p.get().await.unwrap().conn, 2, "no waiting for it when there is nothing else");
    assert_eq!(made.load(Relaxed), 3);
}

#[tokio::test]
async fn a_request_that_failed_because_of_its_connection_is_retried_on_another() {
    let (p, made) = pool_where(1, FOREVER, |_| false).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let got = call_pooled(&p, "test", move |c| {
        seen.fetch_add(1, Relaxed);
        async move { if c == 0 { Err(tonic::Status::unavailable("connection reset")) } else { Ok(c) } }
    })
    .await
    .unwrap();
    // connection 0 is tried once and then dropped; five tries on it would end the sync
    assert_eq!((got, calls.load(Relaxed), made.load(Relaxed)), (1, 2, 2));
}

#[tokio::test]
async fn an_error_of_the_server_does_not_cost_a_connection() {
    let (p, made) = pool(2, FOREVER).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    // lightwalletd's own "unknown" error, once: the request is tried again and no connection is given up for it
    let got = call_pooled(&p, "test", move |_| {
        let first = seen.fetch_add(1, Relaxed) == 0;
        async move { if first { Err(tonic::Status::unknown("no such transaction")) } else { Ok(7) } }
    })
    .await
    .unwrap();
    drop(p.get().await.unwrap()); // a replacement would be made here
    assert_eq!((got, calls.load(Relaxed), made.load(Relaxed)), (7, 2, 2));
}
