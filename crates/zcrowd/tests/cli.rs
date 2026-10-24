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

fn data_file(name: &str, lines: &[String]) -> std::path::PathBuf {
    let p = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&p, lines.join("\n") + "\n").unwrap();
    std::fs::write(
        p.with_extension("meta.json"),
        format!(r#"{{"first_h":3499000,"first_t":{},"last_h":3500100,"last_t":{T}}}"#, T - 31 * 86_400),
    )
    .unwrap();
    p
}

fn run(args: &[&str], stdin: Option<&str>) -> (bool, String, String) {
    let mut c = Command::new(BIN);
    c.args(args).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
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
