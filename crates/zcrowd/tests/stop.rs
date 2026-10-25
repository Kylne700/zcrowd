//! A sync that is told to stop (Ctrl-C, or the terminate signal of the system) leaves the data it had whole, and the next
//! sync goes on from the transactions already fetched.
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_zcrowd");
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn scratch(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// An offline sync (the fixtures are the server) of the two days it has three transactions for, one at a time.
fn sync(out: &Path, delay_ms: u64) -> Command {
    let mut c = Command::new(BIN);
    c.args(["sync", "--days", "2", "--concurrency", "1", "--out"])
        .arg(out)
        .env("ZCROWD_OFFLINE", "1")
        .env("ZCROWD_FIXTURES", format!("{ROOT}/fixtures"))
        .env("ZCROWD_FIXTURES_DELAY_MS", delay_ms.to_string());
    c
}

/// Every file under `dir` with its bytes, the transaction cache and the lock left out.
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut m = BTreeMap::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                todo.push(p);
            } else {
                m.insert(p.strip_prefix(dir).unwrap().to_path_buf(), std::fs::read(&p).unwrap());
            }
        }
    }
    m
}

fn saved(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    files(dir).into_iter().filter(|(p, _)| !p.starts_with("tx") && p.extension().is_none_or(|x| x != "lock")).collect()
}

fn cached(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("tx")).map_or(0, |d| d.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "hex")).count())
}

/// The numbers of the `events=` line, up to the counts of fetched and cached transactions, which differ between runs.
fn events_line(stdout: &[u8]) -> String {
    let s = String::from_utf8_lossy(stdout);
    let l = s.lines().find(|l| l.starts_with("events=")).unwrap_or_else(|| panic!("no events line in {s}"));
    l.split(" fetched_txs=").next().unwrap().to_string()
}

fn stop_a_sync_half_way(name: &str, signal: i32, code: i32, said: &str) {
    let dir = scratch(name);
    let out = dir.join("flows.ndjson");

    // data from an earlier sync, then a cache that is empty again
    let first = sync(&out, 0).output().unwrap();
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    let before = saved(&dir);
    assert!(!before.is_empty());
    let expected = events_line(&first.stdout);
    std::fs::remove_dir_all(dir.join("tx")).unwrap();

    // a slow one, stopped while its second transaction is on the way
    let mut child = sync(&out, 1500).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
    let (tx, lines) = mpsc::channel();
    let err = child.stderr.take().unwrap();
    std::thread::spawn(move || BufReader::new(err).lines().map_while(Result::ok).for_each(|l| drop(tx.send(l))));
    let t = Instant::now();
    while cached(&dir) == 0 {
        assert!(t.elapsed() < Duration::from_secs(30), "no transaction was cached in 30 s");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(unsafe { libc::kill(child.id() as i32, signal) }, 0);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if t.elapsed() > Duration::from_secs(40) {
            child.kill().unwrap();
            panic!("the sync did not stop within 40 s of the signal");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stderr: Vec<String> = lines.iter().collect();
    let stderr = stderr.join("\n");

    assert_eq!(status.code(), Some(code), "exit status {status:?}; stderr: {stderr}");
    assert!(stderr.contains(&format!("sync: stopped by {said}.")) && stderr.contains("run the same command again to go on"), "{stderr}");
    assert!(stderr.contains("is as it was"), "{stderr}");
    assert_eq!(saved(&dir), before, "the data of the earlier sync is as it was");
    assert!(files(&dir).keys().all(|p| p.extension().is_none_or(|x| x != "tmp")), "a temporary file is left: {:?}", files(&dir).keys());
    let kept = cached(&dir);
    assert!((1..3).contains(&kept), "{kept} transactions cached");

    // the lock is free and the next sync fetches only what is missing
    let rerun = sync(&out, 0).output().unwrap();
    assert!(rerun.status.success(), "{}", String::from_utf8_lossy(&rerun.stderr));
    let line = String::from_utf8_lossy(&rerun.stdout).lines().find(|l| l.starts_with("events=")).unwrap().to_string();
    assert_eq!(events_line(&rerun.stdout), expected);
    assert!(line.contains(&format!("fetched_txs={} cached_txs={kept}", 3 - kept)), "{line}");
    assert_eq!(cached(&dir), 3);
}

#[test]
fn ctrl_c_stops_a_sync_without_touching_the_saved_data() {
    stop_a_sync_half_way("stop-int", libc::SIGINT, 130, "SIGINT");
}

#[test]
fn the_terminate_signal_stops_a_sync_without_touching_the_saved_data() {
    stop_a_sync_half_way("stop-term", libc::SIGTERM, 143, "SIGTERM");
}
