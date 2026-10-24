//! When Tor is selected and fails, sync stops loudly and never tries a direct connection.
use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_zcrowd");
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn scratch(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ok(c: &mut Command) -> bool {
    c.output().is_ok_and(|o| o.status.success())
}

fn offline_sync(extra: &[&str]) -> Output {
    Command::new(BIN)
        .args(["sync", "--days", "0", "--candidates-only"])
        .args(extra)
        .env("ZCROWD_OFFLINE", "1")
        .env("ZCROWD_FIXTURES", format!("{ROOT}/fixtures"))
        .output()
        .unwrap()
}

// Tor is the default, but there is no network (empty network namespace): sync must fail with a message that names
// the direct-connection flag, and must not look up the server or connect to it directly. The shim logs every name lookup and
// connect; the server name is unresolvable on purpose, so any direct attempt would show up as a lookup.
#[test]
fn tor_failure_is_loud_and_never_goes_direct() {
    if !ok(Command::new("unshare").args(["-rn", "true"])) {
        eprintln!("SKIPPED: cannot create a network namespace here");
        return;
    }
    let d = scratch("tor-fail");
    let shim = d.join("leakcheck.so");
    if !ok(Command::new("cc").args(["-shared", "-fPIC", "-o"]).arg(&shim).arg(format!("{ROOT}/tests/leakcheck/leakcheck.c")).arg("-ldl")) {
        eprintln!("SKIPPED: no C compiler for the leak-check shim");
        return;
    }
    // Returns (stderr, shim log). `extra` are sync flags; the out dir is per run so "wrote nothing" is checkable.
    let run = |tag: &str, extra: &[&str]| {
        let log = d.join(format!("{tag}.log"));
        let o = Command::new("unshare")
            .args(["-rn", BIN, "sync", "--server", "https://zcrowd-canary.invalid:443"])
            .args(extra)
            .arg("--out")
            .arg(d.join(tag).join("flows.ndjson"))
            .env("LD_PRELOAD", &shim)
            .env("LEAKCHECK_LOG", &log)
            .env("ZCROWD_TOR_BOOTSTRAP_SECS", "5") // the netns has no network: do not wait for the 120 s default
            .env("ARTI_FS_DISABLE_PERMISSION_CHECKS", "yes") // uid 65534 owns "/" inside the namespace
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(1), "{tag}: {}", String::from_utf8_lossy(&o.stderr));
        assert!(!d.join(tag).exists(), "{tag}: a failed sync must write no output");
        (String::from_utf8_lossy(&o.stderr).into_owned(), std::fs::read_to_string(&log).unwrap_or_default())
    };

    // Control: --no-tor is the one way to a direct connection, and the shim must see that attempt.
    let (err, net) = run("direct", &["--no-tor"]);
    assert!(err.contains("warning: --no-tor"), "{err}");
    assert!(net.contains("getaddrinfo zcrowd-canary.invalid"), "the shim missed a direct connection attempt:\n{net}");

    // Default (Tor): fails loudly, names the flag, never looks up the server, and did try Tor relays.
    let tor_dir = d.join("tor-state");
    let (err, net) = run("tor", &["--tor-dir", tor_dir.to_str().unwrap()]);
    assert!(err.contains("did not bootstrap") && err.contains("--no-tor"), "{err}");
    assert!(net.lines().any(|l| l.starts_with("connect ")), "the shim saw no connect at all (Tor was not even tried?):\n{net}");
    assert!(!net.contains("canary"), "the server name was looked up: a direct attempt\n{net}");
    assert!(!net.lines().any(|l| l.starts_with("getaddrinfo ")), "Tor mode resolved a name locally:\n{net}");
}

#[test]
fn tor_flags_conflict_and_offline_mode_never_starts_tor() {
    for bad in [&["--tor", "--no-tor"][..], &["--no-tor", "--tor-dir", "x"][..]] {
        let o = Command::new(BIN).arg("sync").args(bad).output().unwrap();
        assert_eq!(o.status.code(), Some(2), "{bad:?}");
    }
    let d = scratch("tor-offline");
    let o = offline_sync(&["--tor", "--tor-dir", d.join("tor").to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!d.join("tor").exists(), "fixtures mode must not create Tor state");
}

// --end-height pins the range for repeatable runs but can never reach into the reorg margin.
#[test]
fn end_height_pins_the_range_and_respects_the_reorg_margin() {
    let o = offline_sync(&["--end-height", "3522000"]);
    assert!(String::from_utf8_lossy(&o.stdout).contains("range=3521856..3522000"), "{o:?}");
    let o = offline_sync(&["--end-height", "9999999"]);
    assert!(String::from_utf8_lossy(&o.stdout).contains("..3522630 "), "{o:?}");
    assert!(String::from_utf8_lossy(&o.stderr).contains("reorg margin"), "{o:?}");
}

// A plain-text URL is refused before Tor starts: no bootstrap, no Tor state, and no "use --no-tor" advice.
#[test]
fn plain_text_url_is_refused_before_tor_starts() {
    let d = scratch("tor-plain");
    let o = Command::new(BIN)
        .args(["sync", "--server", "http://example.org", "--tor-dir"])
        .arg(d.join("tor-state"))
        .arg("--out")
        .arg(d.join("out/flows.ndjson"))
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(1), "{err}");
    assert!(err.contains("refusing a plain-text connection") && !err.contains("--no-tor"), "{err}");
    assert!(!d.join("tor-state").exists() && !d.join("out").exists(), "nothing may be created");
}
