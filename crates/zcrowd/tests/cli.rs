use std::io::Write;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_zcrowd");
const T: u64 = 1_790_000_000;

fn line_in(kind: &str, pool: &str, zat: u64, i: u64, extra: &str) -> String {
    let n_tin = u8::from(kind != "move"); // a move has no transparent input
    format!(
        r#"{{"h":{},"t":{},"kind":"{kind}","zat":{zat},"pool":"{pool}","fee_zat":null,"n_tin":{n_tin},"n_tout":0,"txid":"{i:064x}","confidence":"high"{extra}}}"#,
        3_500_000 + i,
        T - 100 * i
    )
}

fn line(kind: &str, zat: u64, i: u64, extra: &str) -> String {
    line_in(kind, "sapling", zat, i, extra)
}

/// The meta file of a month of data ending at `T`, in the current format. `extra` is more JSON fields, e.g. `,"skipped":{}`.
fn meta(extra: &str) -> String {
    format!(
        r#"{{"version":{},"first_h":3499000,"first_t":{},"last_h":3500100,"last_t":{T}{extra}}}"#,
        zcrowd_core::model::FORMAT_VERSION,
        T - 31 * 86_400
    )
}

fn data_file_meta(name: &str, lines: &[String], meta: Option<&str>) -> std::path::PathBuf {
    let p = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&p, lines.join("\n") + "\n").unwrap();
    let mp = p.with_extension("meta.json");
    match meta {
        Some(m) => std::fs::write(mp, m).unwrap(),
        None => drop(std::fs::remove_file(mp)),
    }
    p
}

fn data_file(name: &str, lines: &[String]) -> std::path::PathBuf {
    data_file_meta(name, lines, Some(&meta("")))
}

fn run(args: &[&str], stdin: Option<&str>) -> (bool, String, String) {
    run_env(args, stdin, &[])
}

fn run_env(args: &[&str], stdin: Option<&str>, env: &[(&str, &str)]) -> (bool, String, String) {
    let mut c = Command::new(BIN);
    c.args(args).envs(env.iter().copied()).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    if let Some(s) = stdin {
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
    }
    let o = child.wait_with_output().unwrap();
    (o.status.success(), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

/// The stable output contract: exactly one `verdict: <NO MATCH|RED|AMBER|GREEN>` line and one `crowd: k=<n> weighted=<w>` line.
fn contract(out: &str) {
    let v: Vec<_> = out.lines().filter_map(|l| l.strip_prefix("verdict: ")).collect();
    assert_eq!(v.len(), 1, "{out}");
    assert!(["NO MATCH", "RED", "AMBER", "GREEN"].iter().any(|w| v[0] == *w || v[0].starts_with(&format!("{w} "))), "{out}");
    let c: Vec<_> = out.lines().filter(|l| l.starts_with("crowd: ")).collect();
    assert_eq!(c.len(), 1, "{out}");
    let (k, rest) = c[0].strip_prefix("crowd: k=").and_then(|r| r.split_once(" weighted=")).unwrap_or_else(|| panic!("{out}"));
    k.parse::<u64>().unwrap();
    rest.split(' ').next().unwrap().parse::<f64>().unwrap();
}

#[test]
fn cli_check_finds_send_max_round_trip_and_reads_amount_from_stdin() {
    let d = data_file("cli-a.ndjson", &[line("shield", 100_015_000, 1, "")]);
    let (ok, out, err) = run(&["check", "--unshield", "--data", d.to_str().unwrap()], Some("1.0\n"));
    assert!(ok, "{err}");
    contract(&out);
    assert!(out.contains("crowd: k=1 weighted=1.0 ") && out.contains("verdict: RED"), "{out}");
    assert!(!err.contains("shell history"), "stdin input must not warn");
    // value as argument: works, warns on stderr, and the amount is not echoed into the JSON output
    let (ok, out, err) = run(&["check", "--unshield", "1.0", "--data", d.to_str().unwrap(), "--json"], None);
    assert!(ok && err.contains("shell history"));
    assert!(out.contains("\"k\":1") && out.contains("\"weighted\":1.0") && out.contains("\"verdict\":\"RED\""));
    assert!(!out.contains("100000000") && !out.contains("1.0000"));
}

#[test]
fn cli_check_no_match_odd_amount_and_bad_input() {
    let d = data_file("cli-b.ndjson", &[line("shield", 200_000_000, 1, "")]);
    let (ok, out, _) = run(&["check", "--unshield", "-", "--data", d.to_str().unwrap()], Some("1.23456789\n"));
    contract(&out);
    assert!(ok && out.contains("verdict: NO MATCH") && out.contains("easy to recognise"), "{out}");
    assert!(out.contains("Not checked: address reuse"));
    for bad in ["1e3", "-1", "0", "1.123456789"] {
        let (ok, _, err) = run(&["check", "--unshield", "-", "--data", d.to_str().unwrap()], Some(bad));
        assert!(!ok && !err.is_empty(), "{bad}");
    }
}

#[test]
fn cli_check_shield_mode_and_address_reuse() {
    let a = "t1XQZdZMnzXBcL8yx2PR27dSNrqctgwLgux";
    let src = format!(r#","src":["{a}"]"#);
    let mut v: Vec<String> = (1..=12).map(|i| line("shield", 100_000_000, i, "")).collect();
    v.push(line("shield", 500_000_000, 20, &src));
    let d = data_file("cli-c.ndjson", &v);
    let f = d.to_str().unwrap();
    let (_, out, _) = run(&["check", "--shield", "-", "--pool", "sapling", "--data", f], Some("1\n"));
    contract(&out);
    assert!(out.contains("verdict: GREEN"), "{out}");
    // without --pool the worst of the three pools counts: nothing entered Orchard or Ironwood, so the whole note would be unique there
    let (_, out, _) = run(&["check", "--shield", "-", "--data", f], Some("1\n"));
    contract(&out);
    assert!(out.contains("verdict: RED") && out.contains("pool orchard: k=0 weighted=0.0 -> RED"), "{out}");
    let (ok, out, _) = run(&["check", "--unshield", "-", "--to", "-", "--pool", "sapling", "--data", f], Some(&format!("1\n{a}\n")));
    contract(&out);
    assert!(ok && out.contains("verdict: RED") && out.contains("address: RED") && out.contains("shielded on"), "{out}");
    // --json carries the same overall verdict (not the amount's GREEN), plus the two parts; no typed value in it
    let (ok, js, _) =
        run(&["check", "--unshield", "-", "--to", "-", "--pool", "sapling", "--json", "--data", f], Some(&format!("1\n{a}\n")));
    assert!(ok && js.contains(r#""verdict":"RED""#), "{js}");
    assert!(js.contains(r#""amount_verdict":"GREEN""#) && js.contains(r#""address_verdict":"RED""#), "{js}");
    assert!(!js.contains(a), "{js}");
    let (_, js, _) = run(&["check", "--unshield", "-", "--pool", "sapling", "--json", "--data", f], Some("1\n"));
    assert!(js.contains(r#""verdict":"GREEN""#) && js.contains(r#""address_verdict":null"#), "{js}");
    // the address never appears in the data file or anywhere else we wrote
    assert!(!std::fs::read_to_string(&d).unwrap().contains("t1Zzz"));
    let (_, out, _) = run(&["check", "--unshield", "-", "--to", "-", "--data", f], Some("1\nt1Zzz...bad\n"));
    assert!(out.is_empty());
}

// A crowd is the entries into ONE pool; `--pool any` (the default) shows every pool and the worst verdict.
#[test]
fn cli_check_pool_option_scores_one_pool_and_any_shows_the_worst() {
    // 1.00015 ZEC entered Sapling once, Orchard 3 times, Ironwood 12 times (4 of them as moves from another pool)
    let mut v = vec![line_in("shield", "sapling", 100_015_000, 1, "")];
    v.extend((2..=4).map(|i| line_in("shield", "orchard", 100_015_000, i, "")));
    v.extend((5..=12).map(|i| line_in("shield", "ironwood", 100_015_000, i, "")));
    v.extend((13..=16).map(|i| line_in("move", "ironwood", 100_015_000, i, "")));
    // a low-confidence multi-pool event and an unshield are no entries into any pool
    v.push(line_in("shield", "mixed", 100_015_000, 17, "").replace(r#""confidence":"high""#, r#""confidence":"low""#));
    v.push(line_in("unshield", "ironwood", 100_015_000, 18, ""));
    let d = data_file("cli-pools.ndjson", &v);
    let f = d.to_str().unwrap();
    let json = |pool: &str| -> serde_json::Value {
        let (ok, out, err) = run(&["check", "--unshield", "-", "--pool", pool, "--json", "--data", f], Some("1.0\n"));
        assert!(ok, "{err}");
        serde_json::from_str(&out).unwrap()
    };
    for (pool, k, verdict, elsewhere) in [("sapling", 1, "RED", 15), ("orchard", 3, "AMBER", 13), ("ironwood", 12, "GREEN", 4)] {
        let j = json(pool);
        assert_eq!(
            (j["pool"].as_str(), j["k"].as_u64(), j["verdict"].as_str(), j["other_pools_k"].as_u64()),
            (Some(pool), Some(k), Some(verdict), Some(elsewhere)),
            "{j}"
        );
        assert_eq!(j["pools"].as_array().unwrap().len(), 3, "{j}");
    }
    let j = json("any");
    assert_eq!((j["pool"].as_str(), j["k"].as_u64(), j["verdict"].as_str()), (Some("sapling"), Some(1), Some("RED")), "{j}");
    // text: one line per pool, one verdict and one crowd line (the worst pool), the hint, and the other pools' entries
    let (ok, out, _) = run(&["check", "--unshield", "-", "--data", f], Some("1.0\n"));
    assert!(ok);
    contract(&out);
    for l in [
        "pool sapling: k=1 weighted=1.0 -> RED",
        "pool orchard: k=3 weighted=3.0 -> AMBER",
        "pool ironwood: k=12 weighted=12.0 -> GREEN",
        "verdict: RED",
        "funds in another pool? use --pool for a precise answer",
        "other pools: 15 entries",
        "linkable only through a visible pool move; not counted",
    ] {
        assert!(out.contains(l), "{l}\n{out}");
    }
    let (ok, out, _) = run(&["check", "--unshield", "-", "--pool", "ironwood", "--data", f], Some("1.0\n"));
    assert!(ok);
    contract(&out);
    assert!(
        out.contains("verdict: GREEN") && out.contains("crowd: k=12 weighted=12.0 ") && out.contains("other pools: 4 entries"),
        "{out}"
    );
    assert!(!out.contains("funds in another pool?") && !out.contains("pool sapling:"), "{out}");
    // --shield: the amount that arrives in the pool, counted against entries into that pool
    let (ok, out, _) = run(&["check", "--shield", "-", "--pool", "orchard", "--data", f], Some("1.00015\n"));
    assert!(ok);
    contract(&out);
    assert!(out.contains("verdict: AMBER") && out.contains("crowd: k=3 "), "{out}");
    let (ok, _, err) = run(&["check", "--unshield", "-", "--pool", "bitcoin", "--data", f], Some("1.0\n"));
    assert!(!ok && err.contains("--pool"), "{err}");
}

// The process cuts its own network. A listener on loopback is reachable from this process
// but not after the probe enters a fresh network namespace.
#[test]
fn sandbox_blocks_connections() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let (_, plain, _) = run(&["sandbox-probe", &addr], None);
    assert_eq!(plain.trim(), "connected");
    let (_, boxed, err) = run(&["sandbox-probe", &addr, "--sandbox"], None);
    assert!(boxed.starts_with("blocked"), "{boxed} {err}");
}

#[test]
fn report_is_self_contained_and_shows_the_data_window() {
    // an earlier shield of 1.00015 and a later unshield of 1.0: one candidate for an observer
    let d = data_file("cli-report.ndjson", &[line("shield", 100_015_000, 2, ""), line("unshield", 100_000_000, 1, "")]);
    let out = d.with_extension("html");
    let (ok, _, err) = run(&["report", "--data", d.to_str().unwrap(), "--out", out.to_str().unwrap()], None);
    assert!(ok, "{err}");
    let html = std::fs::read_to_string(&out).unwrap();
    assert!(html.contains("Window: blocks 3499000-3500100"), "{html}");
    assert!(html.contains("<tr><td>RED</td><td>1</td><td>100.0%</td></tr>"), "{html}");
    for bad in ["src=", "href=", "url(", "http://", "https://", "@import", "<script"] {
        assert!(!html.contains(bad), "report contains {bad}");
    }
}

const OTHER_VERSION: &str = "data from another zcrowd version: run `zcrowd sync` again";

// Data of another zcrowd (no version, or a different one) or cut off before its meta was written is never trusted.
#[test]
fn check_and_report_refuse_data_without_this_format_version() {
    let ev = [line("shield", 100_015_000, 1, "")];
    let unversioned = r#"{"first_h":3499000,"first_t":1,"last_h":3500100,"last_t":2}"#;
    let d = data_file_meta("cli-v1.ndjson", &ev, Some(unversioned));
    let f = d.to_str().unwrap();
    for args in [["check", "--unshield", "-", "--data", f], ["check", "--shield", "-", "--data", f]] {
        let (ok, out, err) = run(&args, Some("1.0\n"));
        assert!(!ok && out.is_empty() && err.contains(OTHER_VERSION), "{out} {err}");
    }
    let html = d.with_extension("html");
    let _ = std::fs::remove_file(&html);
    let (ok, _, err) = run(&["report", "--data", f, "--out", html.to_str().unwrap()], None);
    assert!(!ok && err.contains(OTHER_VERSION) && !html.exists(), "{err}");
    // events and no meta at all: an interrupted sync
    let d = data_file_meta("cli-v2.ndjson", &ev, None);
    let (ok, out, err) = run(&["check", "--unshield", "-", "--data", d.to_str().unwrap()], Some("1.0\n"));
    assert!(!ok && out.is_empty() && err.contains("no coverage file") && err.contains("run `zcrowd sync` again"), "{out} {err}");
}

#[test]
fn check_says_what_the_sync_skipped_and_whether_it_matters() {
    let ev = [line("shield", 100_015_000, 1, "")];
    let go = |name: &str, skipped: &str, json: bool| {
        let d = data_file_meta(name, &ev, Some(&meta(skipped)));
        let mut args = vec!["check", "--unshield", "-", "--pool", "sapling", "--data", d.to_str().unwrap()];
        args.extend(json.then_some("--json"));
        run(&args, Some("1.0\n"))
    };
    let (ok, out, _) = go("cli-s1.ndjson", r#","skipped":{"no_net_crossing":4,"no_transparent_part":8}"#, false);
    let benign = "\nskipped: no_net_crossing 4, no_transparent_part 8 (not shields or unshields: nothing crossed the turnstile)\n";
    assert!(ok && out.contains(benign), "{out}");
    let (_, out, _) = go("cli-s2.ndjson", r#","skipped":{"conservation_fails":2,"no_net_crossing":4}"#, false);
    let hole = "\nwarning: skipped conservation_fails 2: these could hide a shield or unshield and are missing from the crowd \
                (also skipped, harmless: no_net_crossing 4)\n";
    assert!(out.contains(hole), "{out}");
    let (_, out, _) = go("cli-s3.ndjson", r#","skipped":{}"#, false);
    assert!(out.contains("\nskipped: none\n"), "{out}");
    // a meta that recorded nothing says nothing
    let (_, out, _) = go("cli-s4.ndjson", "", false);
    assert!(!out.contains("skipped"), "{out}");
    let (ok, js, _) = go("cli-s1.ndjson", r#","skipped":{"no_net_crossing":4}"#, true);
    let j: serde_json::Value = serde_json::from_str(&js).unwrap();
    assert!(ok && j["skipped"]["no_net_crossing"] == 4, "{js}");
}

fn offline_sync(out: &std::path::Path) -> (bool, String, String) {
    let fixtures = format!("{}/../../fixtures", env!("CARGO_MANIFEST_DIR"));
    run_env(&["sync", "--days", "2", "--out", out.to_str().unwrap()], None, &[("ZCROWD_OFFLINE", "1"), ("ZCROWD_FIXTURES", &fixtures)])
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn sync_replaces_data_of_another_version_and_stores_the_format_version_and_skipped_counts() {
    let out = scratch("cli-sync-old").join("flows.ndjson");
    std::fs::write(&out, line("shield", 100_000_000, 1, "") + "\n").unwrap();
    std::fs::write(out.with_extension("meta.json"), r#"{"first_h":1,"first_t":1,"last_h":2,"last_t":2}"#).unwrap();
    let (ok, stdout, err) = offline_sync(&out);
    assert!(ok, "{stdout} {err}");
    assert!(err.contains("has no usable coverage file of this zcrowd version; replacing it"), "{err}");
    let (events, meta) = zcrowd_core::model::load(&out).unwrap();
    assert_eq!((events.len(), meta.version), (3, zcrowd_core::model::FORMAT_VERSION));
    assert!(meta.skipped.is_some());
}

#[test]
fn a_second_sync_on_the_same_output_is_refused_while_the_first_holds_the_lock() {
    let out = scratch("cli-sync-lock").join("flows.ndjson");
    let held = std::fs::File::create(out.with_extension("lock")).unwrap();
    held.lock().unwrap();
    let (ok, _, err) = offline_sync(&out);
    assert!(!ok && err.contains("another zcrowd sync is already writing"), "{err}");
    assert!(!out.exists(), "a refused sync writes nothing");
    drop(held);
    let (ok, stdout, err) = offline_sync(&out);
    assert!(ok && out.exists(), "{stdout} {err}");
}
