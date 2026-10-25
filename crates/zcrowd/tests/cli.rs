use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const BIN: &str = env!("CARGO_BIN_EXE_zcrowd");
const T: u64 = 1_790_000_000;

/// An event at time `t`, for data whose newest block is minutes old (`line_in` puts its events around `T`).
fn line_at(kind: &str, pool: &str, zat: u64, i: u64, t: u64, extra: &str) -> String {
    let n_tin = u8::from(kind != "move"); // a move has no transparent input
    format!(
        r#"{{"h":{},"t":{t},"kind":"{kind}","zat":{zat},"pool":"{pool}","fee_zat":null,"n_tin":{n_tin},"n_tout":0,"txid":"{i:064x}","confidence":"high"{extra}}}"#,
        3_500_000 + i
    )
}

fn line_in(kind: &str, pool: &str, zat: u64, i: u64, extra: &str) -> String {
    line_at(kind, pool, zat, i, T - 100 * i, extra)
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

/// A meta whose newest block is from `last_t` and whose window covers `span` seconds.
fn meta_at(last_t: u64, span: u64) -> String {
    format!(
        r#"{{"version":{},"first_h":3499000,"first_t":{},"last_h":3500100,"last_t":{last_t}}}"#,
        zcrowd_core::model::FORMAT_VERSION,
        last_t - span
    )
}

fn wall() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
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
        feed(&mut child, s);
    }
    let o = child.wait_with_output().unwrap();
    (o.status.success(), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

/// Types `s` into the child and closes its stdin. A child that refuses its arguments exits without reading it, so on a busy machine the
/// pipe can be broken before we write: that is the child's answer, not a failure of the test (NOTES S9d F7). Any other error is one.
fn feed(child: &mut Child, s: &str) {
    if let Err(e) = child.stdin.take().unwrap().write_all(s.as_bytes()) {
        assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe, "writing to the child's stdin: {e}");
    }
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

/// The race behind the random failures of the option-limit test, made certain: the child is already gone when its input is written.
/// (`try_wait` polls without closing our end of its stdin, so the pipe is broken by then.)
#[test]
fn input_for_a_child_that_already_refused_its_arguments_is_not_a_test_failure() {
    let mut child = Command::new(BIN)
        .args(["check", "--no-such-flag"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let t0 = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        assert!(t0.elapsed().as_secs() < 30, "the child did not exit");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    feed(&mut child, "1.0\n");
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
    // without --pool the worse of Sapling and Ironwood counts (a shield cannot go into Orchard): nothing entered Ironwood, so the whole note would be unique there
    let (_, out, _) = run(&["check", "--shield", "-", "--data", f], Some("1\n"));
    contract(&out);
    assert!(out.contains("verdict: RED") && out.contains("pool ironwood: k=0 weighted=0.0 -> RED") && !out.contains("orchard"), "{out}");
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
    // a malformed address is refused without being printed back, and `check` writes nothing anywhere
    let before = std::fs::read(&d).unwrap();
    let (ok, out, err) = run(&["check", "--unshield", "-", "--to", "-", "--data", f], Some("1\nt1Zzz...bad\n"));
    assert!(!ok && out.is_empty() && err.contains("not a Zcash address") && !err.contains("t1Zzz"), "{out}{err}");
    assert_eq!(std::fs::read(&d).unwrap(), before);
}

/// A mainnet Unified Address with these receivers, as a string.
fn unified_address(rev: zcash_address::unified::Revision, receivers: Vec<zcash_address::unified::Receiver>) -> String {
    use zcash_address::unified::{Address, Encoding, Uitem};
    use zcash_address::{ToAddress, ZcashAddress};
    use zcash_protocol::consensus::NetworkType;
    let ua = Address::try_from_items(rev, receivers.into_iter().map(Uitem::Data).collect()).unwrap();
    ZcashAddress::from_unified(NetworkType::Main, ua).to_string()
}

// `--to` takes a Unified Address (ZIP 316) too: its transparent receiver is checked, the note says so, and neither the
// Unified Address nor the transparent address inside it is printed back. One without a transparent receiver is no unshield.
#[test]
fn a_unified_address_is_checked_through_its_transparent_receiver_and_never_printed_back() {
    use zcash_address::ToAddress;
    use zcash_address::unified::{Receiver, Revision};
    use zcash_protocol::consensus::NetworkType;
    let a = zcrowd_core::decode::p2pkh([7u8; 20]);
    let mut v: Vec<String> = (1..=12).map(|i| line("shield", 100_000_000, i, "")).collect();
    v.push(line("shield", 500_000_000, 20, &format!(r#","src":["{a}"]"#)));
    let d = data_file("cli-ua.ndjson", &v);
    let f = d.to_str().unwrap();
    let args = ["check", "--unshield", "-", "--to", "-", "--pool", "sapling", "--data", f];
    let with_both = unified_address(Revision::R0, vec![Receiver::Sapling([1; 43]), Receiver::P2pkh([7; 20])]);
    let t_only = unified_address(Revision::R2, vec![Receiver::P2pkh([7; 20])]);
    for (ua, prefers_shielded) in [(&with_both, true), (&t_only, false)] {
        let (ok, out, err) = run(&args, Some(&format!("1\n{ua}\n")));
        contract(&out);
        assert!(ok, "{err}");
        // the shield source is found through the receiver: RED, as for the t1 address itself
        assert!(out.contains("verdict: RED") && out.contains("address: RED") && out.contains("shielded on"), "{out}");
        assert!(out.contains("note: --to is a Unified Address: its transparent receiver is checked"), "{out}");
        assert_eq!(out.contains("Wallets prefer its shielded receiver (ZIP 316)"), prefers_shielded, "{out}");
        assert!(!out.contains(ua.as_str()) && !out.contains(&a) && !err.contains(ua.as_str()), "{out}{err}");
        let (_, js, _) = run(&[&args[..], &["--json"]].concat(), Some(&format!("1\n{ua}\n")));
        assert!(js.contains(r#""address_verdict":"RED""#) && js.contains("note: --to is a Unified Address"), "{js}");
        assert!(!js.contains(ua.as_str()) && !js.contains(&a), "{js}");
    }
    // a plain address carries no such note
    let (_, out, _) = run(&args, Some(&format!("1\n{a}\n")));
    assert!(out.contains("address: RED") && !out.contains("Unified Address"), "{out}");
    // no transparent receiver: nothing to check, and the way out is said in words
    for ua in [
        unified_address(Revision::R0, vec![Receiver::Sapling([1; 43])]),
        unified_address(Revision::R0, vec![Receiver::Orchard([2; 43]), Receiver::Sapling([1; 43])]),
    ] {
        let (ok, out, err) = run(&args, Some(&format!("1\n{ua}\n")));
        assert!(!ok && out.is_empty(), "{out}");
        assert!(err.contains("this Unified Address has no transparent receiver: sending to it does not leave the shielded pools"), "{err}");
        assert!(!err.contains(ua.as_str()), "{err}");
    }
    // Bech32m may be written in capitals (QR codes): a Unified Address in capitals is checked like the lowercase one, mixed
    // case is refused, and neither the capitals nor the lowercase form is printed back
    for ua in [&with_both, &t_only] {
        let upper = ua.to_uppercase();
        let (ok, out, err) = run(&args, Some(&format!("1\n{upper}\n")));
        assert!(ok && out.contains("address: RED") && out.contains("note: --to is a Unified Address"), "{out}{err}");
        let (_, js, _) = run(&[&args[..], &["--json"]].concat(), Some(&format!("1\n{upper}\n")));
        assert!(js.contains(r#""address_verdict":"RED""#), "{js}");
        let mixed = format!("{}{}", ua[..1].to_uppercase(), &ua[1..]);
        let (ok_m, out_m, err_m) = run(&args, Some(&format!("1\n{mixed}\n")));
        assert!(!ok_m && out_m.is_empty() && err_m.contains("not a Zcash address"), "{out_m}{err_m}");
        for text in [&out, &err, &js, &out_m, &err_m] {
            assert!(![upper.as_str(), ua.as_str(), mixed.as_str()].iter().any(|t| text.contains(t)), "{text}");
        }
    }
    // a TEX address may be written in capitals (ZIP 320, Bech32m); a t1 address may not
    let tex = zcash_address::ZcashAddress::from_tex(NetworkType::Main, [7u8; 20]).to_string();
    for t in [tex.clone(), tex.to_uppercase()] {
        let (ok, out, err) = run(&args, Some(&format!("1\n{t}\n")));
        assert!(ok && out.contains("address: RED") && !out.contains(&t), "{out}{err}");
    }
    let (ok, out, err) = run(&args, Some(&format!("1\n{}\n", a.to_uppercase())));
    assert!(!ok && out.is_empty() && err.contains("not a Zcash address"), "{out}{err}");
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
    // --shield: the amount that arrives in the pool, counted against entries into that pool (not Orchard: see below)
    let (ok, out, _) = run(&["check", "--shield", "-", "--pool", "ironwood", "--data", f], Some("1.00015\n"));
    assert!(ok);
    contract(&out);
    assert!(out.contains("verdict: GREEN") && out.contains("crowd: k=12 "), "{out}");
    let (ok, _, err) = run(&["check", "--unshield", "-", "--pool", "bitcoin", "--data", f], Some("1.0\n"));
    assert!(!ok && err.contains("--pool"), "{err}");
}

// NOTES S9d F5: from NU6.3 on no value may enter Orchard (ZIP 258), so a shield cannot go into it. Orchard's empty crowd
// used to make every `--shield` under the default `--pool any` RED. Now `any` is the worse of Sapling and Ironwood, and
// `--pool orchard` is refused before the amount is read.
#[test]
fn shield_mode_weighs_only_the_pools_a_shield_can_go_into() {
    // 1.5 ZEC went into Sapling 3 times and into Ironwood 20 times; nothing went into Orchard
    let mut v: Vec<String> = (1..=3).map(|i| line_in("shield", "sapling", 150_000_000, i, "")).collect();
    v.extend((4..=23).map(|i| line_in("shield", "ironwood", 150_000_000, i, "")));
    let d = data_file("cli-shield-pools.ndjson", &v);
    let f = d.to_str().unwrap();
    let (ok, out, err) = run(&["check", "--shield", "-", "--data", f], Some("1.5\n"));
    assert!(ok, "{err}");
    contract(&out);
    for l in [
        "check: shield (amount arriving in the pool); pool any (worst of sapling, ironwood);",
        "pool sapling: k=3 weighted=3.0 -> AMBER",
        "pool ironwood: k=20 weighted=20.0 -> GREEN",
        "crowd: k=3 weighted=3.0 (pool sapling, the worst of the two:",
        "verdict: AMBER - small crowd",
        "other pools: 20 entries",
    ] {
        assert!(out.contains(l), "{l}\n{out}");
    }
    assert!(!out.contains("orchard"), "{out}");
    let (ok, js, _) = run(&["check", "--shield", "-", "--json", "--data", f], Some("1.5\n"));
    assert!(ok);
    let j: serde_json::Value = serde_json::from_str(&js).unwrap();
    let pools: Vec<_> = j["pools"].as_array().unwrap().iter().map(|p| p["pool"].as_str().unwrap()).collect();
    assert_eq!((pools, j["verdict"].as_str(), j["pool"].as_str()), (vec!["sapling", "ironwood"], Some("AMBER"), Some("sapling")), "{j}");
    // a pool that can take the shield is still scored alone
    let (ok, out, _) = run(&["check", "--shield", "-", "--pool", "ironwood", "--data", f], Some("1.5\n"));
    assert!(ok && out.contains("verdict: GREEN") && out.contains("crowd: k=20 weighted=20.0 "), "{out}");
    // Orchard: refused with the reason before the amount is read, and the amount is not echoed
    let (ok, out, err) = run(&["check", "--shield", "-", "--pool", "orchard", "--data", f], Some("1.5\n"));
    assert!(!ok && out.is_empty() && err.contains("ZIP 258") && err.contains("--pool"), "{out}{err}");
    assert!(!err.contains("1.5"), "{err}");
    // unshield still weighs all three, Orchard included
    let (ok, js, _) = run(&["check", "--unshield", "-", "--json", "--data", f], Some("1.5\n"));
    assert!(ok);
    let j: serde_json::Value = serde_json::from_str(&js).unwrap();
    assert_eq!(j["pools"].as_array().unwrap().len(), 3, "{j}");
}

/// `n` more shields of `zat` into `pool`, numbered on from `*i`.
fn shields(v: &mut Vec<String>, i: &mut u64, pool: &str, zat: u64, n: u64) {
    for _ in 0..n {
        *i += 1;
        v.push(line_in("shield", pool, zat, *i, ""));
    }
}

/// The result of one `check --json` at `amount` (mode is `--unshield` or `--shield`).
fn check_json(f: &str, mode: &str, amount: &str, pool: &str) -> serde_json::Value {
    let (ok, out, err) = run(&["check", mode, "-", "--pool", pool, "--json", "--data", f], Some(&format!("{amount}\n")));
    assert!(ok, "{err}");
    serde_json::from_str(&out).unwrap()
}

/// Every suggestion of the check at `amount` must be what the check itself says when asked at the suggested amount with the
/// same settings (its k, weighted and verdict). Returns how many suggestions were compared.
fn every_suggestion_is_what_its_own_check_says(f: &str, mode: &str, amount: &str, pool: &str) -> usize {
    let j = check_json(f, mode, amount, pool);
    let suggestions = j["suggestions"].as_array().unwrap();
    for s in suggestions {
        let zat = s["zat"].as_u64().unwrap();
        let at = format!("{}.{:08}", zat / 100_000_000, zat % 100_000_000);
        let own = check_json(f, mode, &at, pool);
        for key in ["k", "weighted", "verdict"] {
            assert_eq!(
                s[key], own[key],
                "{mode} {amount} --pool {pool}: the suggestion {at} says {key}={} but checking {at} says {}\n{j}\n{own}",
                s[key], own[key]
            );
        }
    }
    suggestions.len()
}

// NOTES S9d F2: under `--pool any` the suggestions were scored in the one pool that decided the verdict, so a "GREEN"
// suggestion could be RED the moment the user checked that amount (and checking it suggested the first amount back, a loop).
// Fixture B: 2.0 ZEC went into Sapling once and Ironwood 20 times; 2.1 ZEC into Sapling 15 times and Ironwood once; 1.95 ZEC
// into both 12 times.
#[test]
fn under_any_a_suggestion_is_scored_the_way_the_check_scores_that_amount() {
    let (mut v, mut i) = (Vec::new(), 0);
    for (zat, sapling, ironwood) in [(200_000_000, 1, 20), (210_000_000, 15, 1), (195_000_000, 12, 12)] {
        shields(&mut v, &mut i, "sapling", zat, sapling);
        shields(&mut v, &mut i, "ironwood", zat, ironwood);
    }
    let d = data_file("cli-fixture-b.ndjson", &v);
    let f = d.to_str().unwrap();
    let suggested = |amount: &str, pool: &str| -> Vec<(u64, u64, String)> {
        let j = check_json(f, "--unshield", amount, pool);
        j["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["zat"].as_u64().unwrap(), s["k"].as_u64().unwrap(), s["verdict"].to_string()))
            .collect()
    };
    // 2.0: Sapling k=1 is the worst pool. 2.1 would be GREEN in Sapling but is RED in Ironwood (k=1): not a way out.
    assert_eq!(suggested("2.0", "any"), vec![(195_000_000, 12, "\"GREEN\"".to_string())]);
    // 2.1: Ironwood k=1 is the worst pool. 2.0 would be GREEN in Ironwood but is RED in Sapling.
    assert_eq!(suggested("2.1", "any"), vec![(195_000_000, 12, "\"GREEN\"".to_string())]);
    let (_, out, _) = run(&["check", "--unshield", "-", "--data", f], Some("2.0\n"));
    assert!(out.contains("  1.95000000 ZEC -> crowd 12 (weighted 12.0, GREEN)") && !out.contains("2.10000000 ZEC"), "{out}");
    // one pool alone is judged alone, as before: a Sapling user does gain from 2.1
    assert_eq!(suggested("2.0", "sapling").iter().map(|s| s.0).collect::<Vec<_>>(), vec![210_000_000, 195_000_000]);
    assert_eq!(suggested("2.1", "ironwood").iter().map(|s| s.0).collect::<Vec<_>>(), vec![200_000_000, 195_000_000]);
    // the rule behind it, on this fixture and on the sample: whatever is suggested is what checking that amount reports
    let mut compared = 0;
    for pool in ["any", "sapling", "orchard", "ironwood"] {
        for amount in ["2.0", "2.1", "1.97"] {
            compared += every_suggestion_is_what_its_own_check_says(f, "--unshield", amount, pool);
        }
    }
    assert!(compared >= 10, "only {compared} suggestions were compared on the fixture");
    let sample = format!("{}/../../fixtures/flows-sample.ndjson", env!("CARGO_MANIFEST_DIR"));
    let on_sample: usize = [("--unshield", "1.23456789"), ("--unshield", "2.5"), ("--shield", "2.5")]
        .iter()
        .map(|(mode, amount)| every_suggestion_is_what_its_own_check_says(&sample, mode, amount, "any"))
        .sum();
    assert!(on_sample > 0, "the sample gave no suggestion to compare");
}

// NOTES S9d F6: nothing has been able to enter Orchard since NU6.3 (ZIP 258), so an Orchard crowd of zero says nothing about an
// Orchard note (shielded before the window); a plain NO MATCH did not tell the user that.
#[test]
fn an_empty_orchard_crowd_says_why_and_how_to_reach_further_back() {
    let note = "note: orchard has no entries at all in this window, and none can arrive since NU6.3 (ZIP 258).";
    // Sapling and Ironwood are busy; Orchard has one shield, 20 days old
    let mut v: Vec<String> = (1..=12).map(|i| line_in("shield", "sapling", 100_000_000, i, "")).collect();
    v.extend((13..=15).map(|i| line_in("shield", "ironwood", 100_000_000, i, "")));
    let old_orchard =
        line_in("shield", "orchard", 100_000_000, 16, "").replace(&format!("\"t\":{}", T - 1600), &format!("\"t\":{}", T - 20 * 86_400));
    assert!(old_orchard.contains(&format!("\"t\":{}", T - 20 * 86_400)), "{old_orchard}");
    let with_old = data_file("cli-orchard-old.ndjson", &[v.clone(), vec![old_orchard]].concat());
    let without = data_file("cli-orchard-none.ndjson", &v);
    let go = |f: &std::path::Path, args: &[&str]| -> (String, serde_json::Value) {
        let base = ["check", "--unshield", "-", "--data", f.to_str().unwrap()];
        let (ok, out, err) = run(&[&base[..], args].concat(), Some("1\n"));
        assert!(ok, "{err}");
        contract(&out);
        let (ok, js, err) = run(&[&base[..], args, &["--json"]].concat(), Some("1\n"));
        assert!(ok, "{err}");
        (out, serde_json::from_str(&js).unwrap())
    };
    let said = |out: &str, j: &serde_json::Value| -> (usize, bool) {
        (
            out.lines().filter(|l| l.starts_with(note)).count(),
            j["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().starts_with(note)),
        )
    };
    // no Orchard entry at all: once under the default, with the way out, and the same line in the JSON notes
    let (out, j) = go(&without, &[]);
    assert_eq!(said(&out, &j), (1, true), "{out}");
    assert!(out.contains("sync more days (sync --days N) and check with --lookback-days N to reach back to your shield"), "{out}");
    // the Orchard user's own answer carries it too, and so does the one for an Orchard-only question
    let (out, j) = go(&without, &["--pool", "orchard"]);
    assert_eq!(said(&out, &j), (1, true), "{out}");
    assert!(out.contains("verdict: NO MATCH"), "{out}");
    // other pools are not Orchard: no line; a shield cannot go into Orchard: no line
    let (out, j) = go(&without, &["--pool", "sapling"]);
    assert_eq!(said(&out, &j), (0, false), "{out}");
    let (ok, out, err) = run(&["check", "--shield", "-", "--data", without.to_str().unwrap()], Some("1\n"));
    assert!(ok && !out.contains("orchard"), "{out}{err}");
    // an Orchard entry inside the window: Orchard does have a crowd to look at, no line; shorter than its age: the line is back
    let (out, j) = go(&with_old, &[]);
    assert_eq!(said(&out, &j), (0, false), "{out}");
    let (out, j) = go(&with_old, &["--lookback-days", "7"]);
    assert_eq!(said(&out, &j), (1, true), "{out}");
}

// NOTES S9e, F8 h: a multi-pool shield or move is Low confidence and no entry of any pool, so the crowd never counts it. When some
// lie in the lookback and amount windows of a check, a NO MATCH or a small crowd can be wrong, and the check says how many.
#[test]
fn low_confidence_shields_in_the_window_are_told_and_never_counted() {
    let key = "note: low-confidence shields or moves of about this amount: ";
    let low = |l: String| l.replace(r#""confidence":"high""#, r#""confidence":"low""#);
    let aged = |l: String, i: u64, days: u64| l.replace(&format!("\"t\":{}", T - 100 * i), &format!("\"t\":{}", T - days * 86_400));
    let busy: Vec<String> = (1..=3).map(|i| line_in("shield", "sapling", 100_000_000, i, "")).collect();
    let mut v = busy.clone();
    v.push(low(line_in("shield", "mixed", 100_000_000, 4, ""))); // in both windows
    v.push(low(line_in("move", "mixed", 100_020_000, 5, ""))); // in the window of an unshield only (its fee allowance)
    v.push(low(line_in("unshield", "mixed", 100_000_000, 6, ""))); // an unshield is never an entry
    v.push(aged(low(line_in("shield", "mixed", 100_000_000, 7, "")), 7, 20)); // 20 days old: inside a lookback of 30, outside 7
    v.push(low(line_in("shield", "mixed", 200_000_000, 8, ""))); // another amount
    v.push(aged(low(line_in("shield", "mixed", 100_000_000, 9, "")), 9, 40)); // before every lookback of this test
    let with = data_file("cli-low-window.ndjson", &v);
    // the same data without the shields and moves that could matter: nothing to say
    let none = data_file("cli-low-none.ndjson", &[busy, vec![v[5].clone(), v[7].clone(), v[8].clone()]].concat());
    // what a check says about them (text and JSON must agree, the text puts it after the verdict, the JSON last), and the crowd
    let said = |f: &std::path::Path, mode: &str, extra: &[&str]| -> (Option<u64>, u64) {
        let args = [&["check", mode, "-", "--data", f.to_str().unwrap()][..], extra].concat();
        let (ok, out, err) = run(&args, Some("1\n"));
        assert!(ok, "{err}");
        let (ok, json, err) = run(&[&args[..], &["--json"]].concat(), Some("1\n"));
        assert!(ok, "{err}");
        let j: serde_json::Value = serde_json::from_str(&json).unwrap();
        let last = j["notes"].as_array().unwrap().last().and_then(|n| n.as_str()).unwrap_or("").to_string();
        let lines: Vec<&str> = out.lines().collect();
        let told = lines.iter().position(|l| l.starts_with(key));
        assert_eq!(told.is_some(), last.starts_with(key), "{out}\n{last}");
        let k = j["k"].as_u64().unwrap();
        let Some(at) = told else { return (None, k) };
        assert_eq!(lines[at], last);
        assert!(at > lines.iter().position(|l| l.starts_with("verdict:")).unwrap(), "after the verdict: {out}");
        assert_eq!(lines.iter().filter(|l| l.starts_with(key)).count(), 1, "{out}");
        (Some(lines[at][key.len()..].split('.').next().unwrap().parse().unwrap()), k)
    };
    // the crowd is the three High-confidence shields into Sapling whatever the note says
    let sapling = ["--pool", "sapling"];
    assert_eq!(said(&with, "--unshield", &sapling), (Some(3), 3));
    assert_eq!(said(&with, "--shield", &sapling), (Some(2), 3));
    assert_eq!(said(&with, "--unshield", &["--pool", "sapling", "--lookback-days", "7"]), (Some(2), 3));
    assert_eq!(said(&with, "--shield", &["--pool", "sapling", "--lookback-days", "7"]), (Some(1), 3));
    assert_eq!(said(&with, "--unshield", &["--pool", "ironwood"]), (Some(3), 0), "whatever the pool: they could be in any of them");
    assert_eq!(
        said(&with, "--unshield", &["--pool", "sapling", "--tolerance-zat", "100000000"]),
        (Some(4), 3),
        "a wider window finds the other amount"
    );
    assert_eq!(said(&none, "--unshield", &sapling), (None, 3));
    assert_eq!(said(&none, "--shield", &sapling), (None, 3));
}

// The process cuts its own network. A listener on loopback is reachable from this process but not from the probe once it
// is in the sandbox. On Linux the seccomp layer answers first (the socket itself is refused: "Operation not permitted"; the
// empty namespace alone would say "Network is unreachable"); on macOS the no-network profile. Windows has no sandbox.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
#[cfg(target_os = "linux")] // other OSes have no sandbox (src/sandbox.rs)
fn sandbox_blocks_connections() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let (_, plain, _) = run(&["sandbox-probe", &addr], None);
    assert_eq!(plain.trim(), "connected");
    let (_, boxed, err) = run(&["sandbox-probe", &addr, "--sandbox"], None);
    assert!(boxed.starts_with("blocked"), "{boxed} {err}");
}

/// `check`, `report` and `verify` each stop at a data file that is a FIFO nobody writes to, so what is left to look at is the
/// process as it is by then: `held(pid)` must become true before that, or the data had its chance before the cut.
#[cfg(target_os = "linux")]
fn each_is_cut_off_before_its_data(dir: &str, what: &str, held: impl Fn(u32) -> bool) {
    let d = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(dir);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let fifo = d.join("flows.ndjson");
    assert!(Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    std::fs::write(d.join("flows.meta.json"), meta("")).unwrap(); // a good meta, so that the events are what blocks
    let (f, html) = (fifo.to_str().unwrap(), d.join("r.html"));
    let id = "0".repeat(64);
    let cases: [(&str, Vec<&str>); 3] = [
        ("check", vec!["check", "--unshield", "1", "--data", f]),
        ("report", vec!["report", "--data", f, "--out", html.to_str().unwrap()]),
        ("verify", vec!["verify", &id, "--data", f]),
    ];
    for (name, args) in cases {
        let mut c = Command::new(BIN).args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
        let start = std::time::Instant::now();
        let mut ok = false;
        while start.elapsed() < std::time::Duration::from_secs(10) {
            if c.try_wait().unwrap().is_some() {
                break; // gone before it got to the data: whatever it says is in stderr
            }
            if held(c.id()) {
                ok = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = c.kill();
        let err = String::from_utf8_lossy(&c.wait_with_output().unwrap().stderr).into_owned();
        assert!(ok, "`zcrowd {name}` did not {what} before reading its data: {err}");
    }
}

// `sandbox_blocks_connections` shows that entering the sandbox cuts the network; this shows that every command that must be
// offline enters its namespace layer: without `sandbox::enter()` in main, the process stays in this test's network
// namespace. It needs user namespaces (refused on a default Ubuntu 24.04; the seccomp test below holds there).
#[cfg(target_os = "linux")]
#[test]
fn check_report_and_verify_each_leave_the_network_before_they_read_anything() {
    let own = std::fs::read_link("/proc/self/ns/net").unwrap();
    each_is_cut_off_before_its_data("sandbox-fifo", "enter a network namespace of its own", |pid| {
        std::fs::read_link(format!("/proc/{pid}/ns/net")).is_ok_and(|ns| ns != own)
    });
}

// The seccomp layer, also where the namespace works (as here): each command has one seccomp filter more than this test by
// the time it waits for its data. Without it in `sandbox::enter()`, or if it were only a fallback, the count stays.
#[cfg(all(
    target_os = "linux",
    target_pointer_width = "64",
    any(target_arch = "x86_64", all(target_arch = "aarch64", target_endian = "little"))
))]
#[test]
fn check_report_and_verify_each_install_the_seccomp_filter_before_they_read_anything() {
    let filters = |pid: &str| -> Option<u32> {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        status.lines().find_map(|l| l.strip_prefix("Seccomp_filters:"))?.trim().parse().ok()
    };
    let own = filters("self").expect("Seccomp_filters in /proc/self/status (Linux 5.9 or later)");
    each_is_cut_off_before_its_data("seccomp-fifo", "install the seccomp filter", |pid| filters(&pid.to_string()) == Some(own + 1));
}

const FIXTURE_TX: &str = "954c19b2c73e667170c28b8d3a94fd96582cba736a5ed7275a10fc0c3e64e149";

// `verify` reads the transaction id privately with `-`, as `check` reads the amount; an id in the arguments stays in shell
// history, which says which transaction was looked at.
#[test]
fn verify_reads_the_id_from_stdin_notes_one_in_the_arguments_and_advises_spot_checks() {
    let fixtures = format!("{}/../../fixtures", env!("CARGO_MANIFEST_DIR"));
    let sample = std::fs::read_to_string(format!("{fixtures}/flows-sample.ndjson")).unwrap();
    let event = sample.lines().find(|l| l.contains(FIXTURE_TX)).unwrap().to_string();
    let d = data_file("cli-verify.ndjson", std::slice::from_ref(&event));
    let (f, raw) = (d.to_str().unwrap(), format!("{fixtures}/raw"));
    let verify = |id: &str, stdin: Option<&str>| run(&["verify", id, "--data", f, "--raw-dir", &raw], stdin);

    let (ok, out, err) = verify("-", Some(&format!("{FIXTURE_TX}\n")));
    assert!(ok, "{out}{err}");
    assert!(out.contains("MATCH: recomputed") && !err.contains("shell history"), "{out}{err}");
    assert!(out.contains("Spot-check random events, not your own matches, and open the explorer over Tor"), "{out}");
    assert!(!out.contains("compare on any explorer"), "{out}");
    // blanks and capitals around a typed id are forgiven
    let (ok, out, _) = verify("-", Some(&format!("  {}  \n", FIXTURE_TX.to_uppercase())));
    assert!(ok && out.contains("MATCH"), "{out}");
    // an id in the arguments works and says where it stays
    let (ok, out, err) = verify(FIXTURE_TX, None);
    assert!(ok && out.contains("MATCH"), "{out}{err}");
    assert!(err.contains("a transaction id given as an argument is visible in shell history"), "{err}");
    // neither a word nor an id that is not in the data is printed back
    for (typed, want) in [("zz-not-an-id", "not a transaction id"), ("0".repeat(64).as_str(), "is not an event")] {
        let (ok, out, err) = verify("-", Some(&format!("{typed}\n")));
        assert!(!ok && out.is_empty() && err.contains(want) && !err.contains(typed), "{out}{err}");
    }
    // data of another zcrowd version is refused here too, not decoded against a different format
    let old = data_file_meta("cli-verify-old.ndjson", &[event], Some(r#"{"first_h":1,"first_t":1,"last_h":2,"last_t":2}"#));
    let (ok, _, err) = run(&["verify", "-", "--data", old.to_str().unwrap(), "--raw-dir", &raw], Some(&format!("{FIXTURE_TX}\n")));
    assert!(!ok && err.contains(OTHER_VERSION), "{err}");
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

/// NOTES S9d F6: an unshield out of Orchard is NO MATCH by construction when the data holds no entry into Orchard (none can arrive
/// since NU6.3, ZIP 258). The report counts them apart instead of letting them pass for unshields nobody could link.
#[test]
fn report_counts_the_no_match_unshields_out_of_an_orchard_nothing_can_enter() {
    let report = |name: &str, lines: &[String]| {
        let d = data_file(name, lines);
        let out = d.with_extension("html");
        let (ok, _, err) = run(&["report", "--data", d.to_str().unwrap(), "--out", out.to_str().unwrap()], None);
        assert!(ok, "{err}");
        std::fs::read_to_string(&out).unwrap()
    };
    // nothing entered any pool: three NO MATCH, two of them out of Orchard
    let mut v = vec![
        line_in("unshield", "orchard", 100_000_000, 1, ""),
        line_in("unshield", "orchard", 200_000_000, 2, ""),
        line("unshield", 300_000_000, 3, ""),
    ];
    let html = report("cli-report-orchard-empty.ndjson", &v);
    assert!(html.contains("<tr><td>NO MATCH</td><td>3</td>"), "{html}");
    assert!(html.contains("<p>2 of the NO MATCH unshields left Orchard. This data holds no shield or move into Orchard"), "{html}");
    // a move into Orchard anywhere in the data: its NO MATCH is a real one, so no such sentence
    v.push(line_in("move", "orchard", 400_000_000, 4, ""));
    let html = report("cli-report-orchard-entry.ndjson", &v);
    assert!(html.contains("<tr><td>NO MATCH</td><td>3</td>") && !html.contains("left Orchard"), "{html}");
    // no unshield out of Orchard: nothing to say either
    let html = report("cli-report-orchard-none.ndjson", &[line("unshield", 300_000_000, 3, "")]);
    assert!(!html.contains("left Orchard"), "{html}");
}

/// NOTES S9d F8: serde's position is inside the one line it was given, so a damaged line 4 of the file used to be reported as "line 1".
/// The blank line is there to show that the number is the file's line, not the count of events.
#[test]
fn a_damaged_event_line_is_reported_with_its_line_number_in_the_file() {
    let good = line("shield", 100_000_000, 1, "");
    let d = data_file("cli-damaged-line.ndjson", &[good.clone(), String::new(), good, r#"{"h":3500002,"t":"#.into()]);
    let f = d.to_str().unwrap();
    let html = d.with_extension("html");
    for args in [vec!["check", "--unshield", "-", "--data", f], vec!["report", "--data", f, "--out", html.to_str().unwrap()]] {
        let (ok, _, err) = run(&args, Some("1.0\n"));
        assert!(!ok && err.contains("line 4") && !err.contains("at line 1"), "{} -> {err}", args[0]);
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

#[test]
fn sync_reports_its_progress_on_stderr_with_heights_and_counts_only() {
    let d = scratch("cli-progress");
    let (ok, out, err) = offline_sync(&d.join("flows.ndjson"));
    assert!(ok, "{err}");
    // 2 days = 2304 blocks + 144 margin + the end block: two chunks of blocks, then the 3 transactions
    for want in [
        "sync: fetching blocks 3520182..3522630",
        "sync: blocks 2000 of 2449 (81%)",
        "sync: blocks 2449 of 2449 (100%)",
        "sync: fetching up to 3 transactions",
        "sync: fetched 3 of up to 3 transactions",
    ] {
        assert!(err.contains(want), "missing {want:?}:\n{err}");
    }
    assert!(!out.contains("sync:"), "progress belongs on stderr, stdout keeps its lines: {out}");
    assert!(!err.contains("still fetching"), "a sync that is not slow adds no status line:\n{err}");
    let long_hex = |l: &str| l.split(|c: char| !c.is_ascii_hexdigit()).any(|w| w.len() >= 20);
    assert!(!err.lines().filter(|l| l.starts_with("sync:")).any(long_hex), "a progress line carries a transaction id:\n{err}");
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

/// `check --unshield` on one shield of 1.00015 ZEC and the given meta, amount 1.0 (found: RED) or `amount`; text or JSON.
fn check_with_meta(name: &str, meta: &str, json: bool) -> String {
    let d = data_file_meta(name, &[line("shield", 100_015_000, 1, "")], Some(meta));
    let mut args = vec!["check", "--unshield", "-", "--pool", "sapling", "--data", d.to_str().unwrap()];
    args.extend(json.then_some("--json"));
    let (ok, out, err) = run(&args, Some("1.0\n"));
    assert!(ok, "{err}");
    out
}

// The amount is typed privately; nothing the command prints may bring it back (scrollback, CI logs).
#[test]
fn the_amount_typed_is_never_printed_back() {
    // no shield near this amount, so no match line can carry it either
    let d = data_file("cli-noecho.ndjson", &[line("shield", 200_000_000, 1, "")]);
    let f = d.to_str().unwrap();
    let (amt, zat) = ("1.24975318", "124975318");
    let clean = |out: &str, err: &str, what: &str| {
        for s in [out, err] {
            assert!(!s.contains(amt) && !s.contains(zat) && !s.contains("1.2497"), "{what}: {out} {err}");
        }
    };
    for (what, args) in [
        ("unshield", vec!["check", "--unshield", "-", "--data", f]),
        ("unshield json", vec!["check", "--unshield", "-", "--json", "--data", f]),
        ("shield", vec!["check", "--shield", "-", "--data", f]),
    ] {
        let (ok, out, err) = run(&args, Some(&format!("{amt}\n")));
        assert!(ok && out.contains("NO MATCH") | out.contains("RED"), "{what}: {out} {err}");
        clean(&out, &err, what);
    }
    // given as an argument it is warned about, but not repeated
    let (ok, out, err) = run(&["check", "--unshield", amt, "--data", f], None);
    assert!(ok && err.contains("shell history"), "{err}");
    clean(&out, &err, "argument");
    // typed wrong: the error says what is wrong, not what was typed
    for bad in ["1.2497531x", "1.249753189", "-1.24975318", "124975318.24975318", "99999999999999999999.24975318"] {
        let (ok, out, err) = run(&["check", "--unshield", "-", "--data", f], Some(&format!("{bad}\n")));
        assert!(!ok && !err.is_empty(), "{bad}");
        assert!(!out.contains("1.2497") && !err.contains("1.2497") && !err.contains("24975318"), "{bad}: {out} {err}");
    }
}

// NOTES S9d F1: under `--pool any` GREEN outranks NO MATCH, so a pool with no entry never becomes the verdict and its caveat
// never showed (a user whose own shield landed after the last block saw GREEN and nothing else). The line now follows every pool
// shown. Orchard is left out: nothing can enter it from NU6.3 on (ZIP 258), so later blocks cannot change its crowd.
#[test]
fn under_any_the_missing_shields_line_follows_every_pool_shown_except_orchard() {
    let line_after = "shields after 2026-09-21 14:13 UTC (last synced block) are not counted";
    // 12 shields of 1.2345 ZEC into Sapling, none into Ironwood or Orchard: Sapling GREEN, the others NO MATCH
    let sapling: Vec<String> = (1..=12).map(|i| line("shield", 123_450_000, i, "")).collect();
    let d = data_file("cli-any-end1.ndjson", &sapling);
    let f = d.to_str().unwrap();
    let (ok, out, err) = run(&["check", "--unshield", "-", "--data", f], Some("1.2345\n"));
    assert!(ok, "{err}");
    assert!(out.contains("verdict: GREEN") && out.contains("pool ironwood: k=0 weighted=0.0 -> NO MATCH"), "{out}");
    assert!(out.contains(&format!("\n{line_after}\n")), "{out}");
    let (_, js, _) = run(&["check", "--unshield", "-", "--json", "--data", f], Some("1.2345\n"));
    let j: serde_json::Value = serde_json::from_str(&js).unwrap();
    assert!(j["notes"].as_array().unwrap().iter().any(|n| n == line_after), "{j}");
    // a pool chosen on its own is judged on its own: its GREEN says nothing about the others
    let (_, out, _) = run(&["check", "--unshield", "-", "--pool", "sapling", "--data", f], Some("1.2345\n"));
    assert!(out.contains("verdict: GREEN") && !out.contains("(last synced block)"), "{out}");
    // only Orchard has no entry: no line, under `any` and when chosen
    let both: Vec<String> = sapling.iter().cloned().chain((13..=24).map(|i| line_in("shield", "ironwood", 123_450_000, i, ""))).collect();
    let d = data_file("cli-any-end2.ndjson", &both);
    let f = d.to_str().unwrap();
    let (ok, out, _) = run(&["check", "--unshield", "-", "--data", f], Some("1.2345\n"));
    assert!(ok && out.contains("verdict: GREEN") && out.contains("pool orchard: k=0 weighted=0.0 -> NO MATCH"), "{out}");
    assert!(!out.contains("(last synced block)"), "{out}");
    let (ok, out, _) = run(&["check", "--unshield", "-", "--pool", "orchard", "--data", f], Some("1.2345\n"));
    assert!(ok && out.contains("verdict: NO MATCH") && !out.contains("(last synced block)"), "{out}");
}

#[test]
fn no_match_and_red_say_that_shields_after_the_last_block_are_not_counted() {
    let line_after = "\nshields after 2026-09-21 14:13 UTC (last synced block) are not counted\n";
    let f = |name: &str, lines: &[String]| data_file(name, lines);
    let check = |d: &std::path::Path, mode: &str, amt: &str, extra: &[&str]| {
        let mut args = vec!["check", mode, "-", "--pool", "sapling", "--data", d.to_str().unwrap()];
        args.extend(extra);
        let (ok, out, err) = run(&args, Some(&format!("{amt}\n")));
        assert!(ok, "{err}");
        out
    };
    // NO MATCH and RED: always, even though this data is as fresh as it can be for the verdict itself
    let none = f("cli-end1.ndjson", &[line("shield", 200_000_000, 1, "")]);
    let out = check(&none, "--unshield", "1.0", &[]);
    assert!(out.contains("verdict: NO MATCH") && out.contains(line_after), "{out}");
    let one = f("cli-end2.ndjson", &[line("shield", 100_015_000, 1, "")]);
    let out = check(&one, "--unshield", "1.0", &[]);
    assert!(out.contains("verdict: RED") && out.contains(line_after), "{out}");
    let out = check(&none, "--shield", "1.0", &[]);
    assert!(out.contains("verdict: RED") && out.contains(line_after), "{out}");
    // AMBER and GREEN: not said
    let two: Vec<String> = (1..=2).map(|i| line("shield", 100_000_000, i, "")).collect();
    let out = check(&f("cli-end3.ndjson", &two), "--unshield", "1.0", &[]);
    assert!(out.contains("verdict: AMBER") && !out.contains("(last synced block)"), "{out}");
    let twelve: Vec<String> = (1..=12).map(|i| line("shield", 100_000_000, i, "")).collect();
    let out = check(&f("cli-end4.ndjson", &twelve), "--unshield", "1.0", &[]);
    assert!(out.contains("verdict: GREEN") && !out.contains("(last synced block)"), "{out}");
    // the overall verdict counts: a GREEN amount with a RED address is RED, and says it
    let a = "t1XQZdZMnzXBcL8yx2PR27dSNrqctgwLgux";
    let mut v = twelve.clone();
    v.push(line("shield", 500_000_000, 20, &format!(r#","src":["{a}"]"#)));
    let d = f("cli-end5.ndjson", &v);
    let (ok, out, _) =
        run(&["check", "--unshield", "-", "--to", "-", "--pool", "sapling", "--data", d.to_str().unwrap()], Some(&format!("1\n{a}\n")));
    assert!(ok && out.contains("verdict: RED (worst of amount GREEN and address RED)") && out.contains(line_after), "{out}");
    // --json carries it among the notes, and only there
    let j: serde_json::Value = serde_json::from_str(&check(&none, "--unshield", "1.0", &["--json"])).unwrap();
    let notes: Vec<&str> = j["notes"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
    assert!(notes.contains(&line_after.trim()), "{j}");
}

// NOTES S9g, N1 of the third review: the address finding had no such line. An address found nowhere ("clear", NO MATCH) or only as
// the destination of earlier unshields (AMBER) turns RED if a shield from it comes after the newest block, however fresh the data
// is and in every pool, Orchard too: ZIP 258 closes Orchard to new value, not the address check. A shield source (RED) is final.
#[test]
fn an_address_that_is_not_a_shield_source_says_that_shields_after_the_last_block_are_not_counted() {
    let last_t = wall() - 600; // newest block 10 minutes old and a month of data: no stale or short-window warning to blame
    let (clear, used) = ("t1XQZdZMnzXBcL8yx2PR27dSNrqctgwLgux", "t1Sr1dKZJGxoMcxsUcK6a5RZofaJi8UWHxF");
    let line_after = format!("shields after {} (last synced block) are not counted", zcrowd_core::model::fmt_utc(last_t));
    // 12 shields of 1.2345 ZEC into Sapling and 12 into Ironwood, none into Orchard: GREEN in both, NO MATCH in Orchard
    let crowd: Vec<String> =
        (1..=24).map(|i| line_at("shield", if i <= 12 { "sapling" } else { "ironwood" }, 123_450_000, i, last_t - 100 * i, "")).collect();
    let file = |name: &str, extra: Option<String>| {
        let lines: Vec<String> = crowd.iter().cloned().chain(extra).collect();
        data_file_meta(name, &lines, Some(&meta_at(last_t, 31 * 86_400)))
    };
    let d_clear = file("cli-addr-end1.ndjson", None);
    let d_used =
        file("cli-addr-end2.ndjson", Some(line_at("unshield", "sapling", 50_000_000, 30, last_t - 3000, &format!(r#","dst":["{used}"]"#))));
    let d_src =
        file("cli-addr-end3.ndjson", Some(line_at("shield", "sapling", 500_000_000, 31, last_t - 3100, &format!(r#","src":["{clear}"]"#))));
    let go = |d: &std::path::Path, pool: &str, who: &str, json: bool| {
        let mut args = vec!["check", "--unshield", "-", "--to", "-", "--pool", pool, "--data", d.to_str().unwrap()];
        args.extend(json.then_some("--json"));
        let (ok, out, err) = run(&args, Some(&format!("1.2345\n{who}\n")));
        assert!(ok, "{err}");
        out
    };
    let rank = |v: &str| ["NO MATCH", "GREEN", "AMBER", "RED"].iter().position(|x| *x == v).unwrap();
    for (pool, amount) in [("any", "GREEN"), ("sapling", "GREEN"), ("orchard", "NO MATCH"), ("ironwood", "GREEN")] {
        for (d, who, address) in [(&d_clear, clear, "NO MATCH"), (&d_used, used, "AMBER")] {
            let overall = if rank(address) > rank(amount) { address } else { amount };
            let want = format!("verdict: {overall} (worst of amount {amount} and address {address})");
            let out = go(d, pool, who, false);
            contract(&out);
            let at = |l: &str| out.lines().position(|x| x == l || x.starts_with(l));
            assert!(at(&want).is_some() && at(&want) < at(&line_after), "--pool {pool}, address {address}: {out}");
            assert_eq!(out.lines().filter(|l| *l == line_after).count(), 1, "--pool {pool}, address {address}: {out}");
            assert!(!out.contains("warning:") && !out.contains(who), "{out}");
            // --json: once among the notes, and no warning among them either
            let j: serde_json::Value = serde_json::from_str(&go(d, pool, who, true)).unwrap();
            let notes: Vec<&str> = j["notes"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
            assert_eq!(notes.iter().filter(|n| **n == line_after).count(), 1, "--pool {pool}, address {address}: {j}");
            assert!(!notes.iter().any(|n| n.starts_with("warning:")), "{j}");
        }
    }
    // it is the address that adds the line: the same data and amount without --to are GREEN in Sapling and say nothing
    let (ok, out, _) = run(&["check", "--unshield", "-", "--pool", "sapling", "--data", d_clear.to_str().unwrap()], Some("1.2345\n"));
    assert!(ok && out.contains("verdict: GREEN") && !out.contains("(last synced block)"), "{out}");
    // a shield source is RED and final: no shield later can undo it, and under Orchard nothing else rests on missing data
    let out = go(&d_src, "orchard", clear, false);
    assert!(out.contains("verdict: RED (worst of amount NO MATCH and address RED)") && !out.contains("(last synced block)"), "{out}");
    // --to belongs to --unshield
    let (ok, out, err) =
        run(&["check", "--shield", "-", "--to", "-", "--data", d_clear.to_str().unwrap()], Some(&format!("1.2345\n{clear}\n")));
    assert!(!ok && out.is_empty() && err.contains("--to applies to --unshield") && !err.contains(clear), "{out}{err}");
}

#[test]
fn json_carries_the_data_window_its_age_the_footer_and_the_notes() {
    let t = wall() - 3 * 3600;
    // one hour short of a 30-day lookback, and a sync that skipped something that matters
    let m = meta_at(t, 30 * 86_400 - 3600).replace('}', r#","skipped":{"conservation_fails":2}}"#);
    let j: serde_json::Value = serde_json::from_str(&check_with_meta("cli-js1.ndjson", &m, true)).unwrap();
    assert_eq!(
        j["data"],
        serde_json::json!({"first_h": 3499000, "last_h": 3500100, "first_t": t - 30 * 86_400 + 3600, "last_t": t, "events": 1})
    );
    let age = j["stale_secs"].as_u64().unwrap();
    assert!((3 * 3600..3 * 3600 + 120).contains(&age), "{j}");
    assert_eq!(j["short_window"], true);
    let footer = j["not_covered"].as_str().unwrap();
    assert!(footer.starts_with("Covers amount + timing within one pool") && footer.contains("Not covered: wallet/fee fingerprints"), "{j}");
    assert_eq!(j["pools"].as_array().unwrap().len(), 3);
    let notes: Vec<&str> = j["notes"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
    for want in [
        "warning: skipped conservation_fails 2:",
        "warning: the newest block in this data is 3.0 h old",
        "warning: the data covers 29.96 days but the lookback is 30 days",
        "shields after ",
    ] {
        assert!(notes.iter().any(|n| n.starts_with(want) || n.contains(want)), "{want}: {notes:?}");
    }
    // a window as long as the lookback, a block a minute old: nothing to say about age or length
    let j: serde_json::Value = serde_json::from_str(&check_with_meta("cli-js2.ndjson", &meta_at(wall() - 60, 30 * 86_400), true)).unwrap();
    assert_eq!((&j["short_window"], j["stale_secs"].as_u64().map(|s| s < 3600)), (&serde_json::json!(false), Some(true)), "{j}");
    assert!(!j["notes"].to_string().contains("warning"), "{j}");
}

// The 6 hours of slack are gone: a window even a second shorter than the lookback is said to be short.
#[test]
fn a_window_shorter_than_the_lookback_is_a_short_window_without_slack() {
    let t = wall() - 60;
    let day = 30 * 86_400;
    let text = |span| check_with_meta("cli-short.ndjson", &meta_at(t, span), false);
    assert!(!text(day).contains("the data covers"), "exactly the lookback is enough");
    assert!(text(day - 1).contains("warning: the data covers 30.00 days but the lookback is 30 days"));
    assert!(text(day - 3600).contains("warning: the data covers 29.96 days but the lookback is 30 days"));
    // the committed sample (31.65 days: a 30-day sync plus its margin of 5%) is long enough for a 31-day lookback, not for 32
    let sample = format!("{}/../../fixtures/flows-sample.ndjson", env!("CARGO_MANIFEST_DIR"));
    let go = |days: &str| run(&["check", "--unshield", "-", "--lookback-days", days, "--data", &sample], Some("1.0\n")).1;
    for days in ["30", "31"] {
        assert!(!go(days).contains("the data covers"), "{}", go(days));
    }
    assert!(go("32").contains("warning: the data covers 31.65 days but the lookback is 32 days"), "{}", go("32"));
}

#[test]
fn a_clock_far_off_the_chain_is_reported() {
    let now = wall();
    let m = |last_t| meta_at(last_t, 31 * 86_400);
    let out = check_with_meta("cli-clock1.ndjson", &m(now + 4 * 3600), false);
    assert!(
        out.contains("warning: this computer's clock (") && out.contains(") is more than 3 h behind the newest block in this data ("),
        "{out}"
    );
    let out = check_with_meta("cli-clock2.ndjson", &m(now - 400 * 86_400), false);
    assert!(out.contains(") is more than a year after the newest block in this data ("), "{out}");
    // within the limits: a clock 1 h behind, data a week old (stale, but the clock is fine)
    for (name, last_t) in
        [("cli-clock3.ndjson", now + 3600), ("cli-clock4.ndjson", now - 7 * 86_400), ("cli-clock5.ndjson", now - 300 * 86_400)]
    {
        let out = check_with_meta(name, &m(last_t), false);
        assert!(!out.contains("this computer's clock"), "{out}");
    }
    let j: serde_json::Value = serde_json::from_str(&check_with_meta("cli-clock6.ndjson", &m(now + 4 * 3600), true)).unwrap();
    assert!(j["notes"].to_string().contains("more than 3 h behind"), "{j}");
    assert_eq!(j["stale_secs"], 0);
}

#[test]
fn numeric_options_have_limits_and_huge_values_are_refused_not_crashed() {
    let d = data_file("cli-limits.ndjson", &[line("shield", 100_015_000, 1, "")]);
    let f = d.to_str().unwrap();
    let max = u64::MAX.to_string();
    for (flag, v, range) in [
        ("--lookback-days", "0", "1..=3650"),
        ("--lookback-days", "3651", "1..=3650"),
        ("--lookback-days", max.as_str(), "1..=3650"),
        ("--tolerance-zat", "100000001", "0..=100000000"),
        ("--tolerance-zat", max.as_str(), "0..=100000000"),
        ("--fee-allowance-zat", "100000001", "0..=100000000"),
        ("--fee-allowance-zat", max.as_str(), "0..=100000000"),
    ] {
        let (ok, out, err) = run(&["check", "--unshield", "-", flag, v, "--data", f], Some("1.0\n"));
        assert!(!ok && out.is_empty() && err.contains(&format!("not in {range}")) && !err.contains("panicked"), "{flag} {v}: {err}");
    }
    for days in ["0", "3651", max.as_str()] {
        let (ok, _, err) = run(&["sync", "--days", days, "--out", "/nonexistent/flows.ndjson"], None);
        assert!(!ok && err.contains("not in 1..=3650") && !err.contains("panicked"), "--days {days}: {err}");
    }
    // the limits themselves are accepted, and the widest window still gives a verdict
    let args = [
        "check",
        "--unshield",
        "-",
        "--lookback-days",
        "3650",
        "--tolerance-zat",
        "100000000",
        "--fee-allowance-zat",
        "100000000",
        "--data",
        f,
    ];
    let (ok, out, err) = run(&args, Some("1.0\n"));
    assert!(ok, "{err}");
    contract(&out);
    // and the help says where the limits are
    for (cmd, want) in [("check", ["1 to 3650", "0 to 100000000"]), ("sync", ["1 to 3650", "1 to 3650"])] {
        let (_, help, _) = run(&[cmd, "--help"], None);
        assert!(want.iter().all(|w| help.contains(w)), "{cmd}: {help}");
    }
}
