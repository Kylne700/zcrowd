use std::io::Write;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_zcrowd");
const T: u64 = 1_790_000_000;

fn line(kind: &str, zat: u64, i: u64, extra: &str) -> String {
    format!(
        r#"{{"h":{},"t":{},"kind":"{kind}","zat":{zat},"pool":"sapling","fee_zat":null,"n_tin":1,"n_tout":0,"txid":"{i:064x}","confidence":"high"{extra}}}"#,
        3_500_000 + i,
        T - 100 * i
    )
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

#[test]
fn cli_check_finds_send_max_round_trip_and_reads_amount_from_stdin() {
    let d = data_file("cli-a.ndjson", &[line("shield", 100_015_000, 1, "")]);
    let (ok, out, err) = run(&["check", "--unshield", "--data", d.to_str().unwrap()], Some("1.0\n"));
    assert!(ok, "{err}");
    assert!(out.contains("crowd: 1 ") && out.contains("verdict: RED"), "{out}");
    assert!(!err.contains("shell history"), "stdin input must not warn");
    // value as argument: works, warns on stderr, and the amount is not echoed into the JSON output
    let (ok, out, err) = run(&["check", "--unshield", "1.0", "--data", d.to_str().unwrap(), "--json"], None);
    assert!(ok && err.contains("shell history"));
    assert!(out.contains("\"k\":1") && !out.contains("100000000") && !out.contains("1.0000"));
}

#[test]
fn cli_check_no_match_odd_amount_and_bad_input() {
    let d = data_file("cli-b.ndjson", &[line("shield", 200_000_000, 1, "")]);
    let (ok, out, _) = run(&["check", "--unshield", "-", "--data", d.to_str().unwrap()], Some("1.23456789\n"));
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
    let (_, out, _) = run(&["check", "--shield", "-", "--data", f], Some("1\n"));
    assert!(out.contains("verdict: GREEN"), "{out}");
    let (ok, out, _) = run(&["check", "--unshield", "-", "--to", "-", "--data", f], Some(&format!("1\n{a}\n")));
    assert!(ok && out.contains("verdict: RED") && out.contains("address: RED") && out.contains("shielded on"), "{out}");
    // the address never appears in the data file or anywhere else we wrote
    assert!(!std::fs::read_to_string(&d).unwrap().contains("t1Zzz"));
    let (_, out, _) = run(&["check", "--unshield", "-", "--to", "-", "--data", f], Some("1\nt1Zzz...bad\n"));
    assert!(out.is_empty());
}

// Spec 41 C3: the process cuts its own network. A listener on loopback is reachable from this process
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
    let d = data_file("cli-report.ndjson", &[line("shield", 100_015_000, 1, ""), line("unshield", 100_000_000, 2, "")]);
    let out = d.with_extension("html");
    let (ok, _, err) = run(&["report", "--data", d.to_str().unwrap(), "--out", out.to_str().unwrap()], None);
    assert!(ok, "{err}");
    let html = std::fs::read_to_string(&out).unwrap();
    assert!(html.contains("Window: blocks 3499000-3500100"), "{html}");
    for bad in ["src=", "href=", "url(", "http://", "https://", "@import", "<script"] {
        assert!(!html.contains(bad), "report contains {bad}");
    }
}
