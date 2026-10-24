//! The data file and its meta: the format version is checked, and a save that stops half way never leaves a window that
//! claims more than the events hold.
use anyhow::bail;
use std::collections::BTreeMap;
use std::path::PathBuf;
use zcrowd_core::model::{Confidence, Coverage, Event, FORMAT_VERSION, Kind, Meta, Pool, load, read_meta, save, save_with};

fn dir(name: &str) -> PathBuf {
    let d: PathBuf = [env!("CARGO_TARGET_TMPDIR"), "meta", name].iter().collect();
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn event(h: u64) -> Event {
    Event {
        h,
        t: 1_790_000_000 + h,
        kind: Kind::Shield,
        zat: 1_000_000 + h,
        pool: Pool::Ironwood,
        fee_zat: None,
        n_tin: 1,
        n_tout: 0,
        txid: format!("{h:064x}"),
        confidence: Confidence::High,
        coinbase: false,
        src: vec![],
        dst: vec![],
    }
}

/// Events at heights `a..=b` and the meta that describes them.
fn window(a: u64, b: u64) -> (Vec<Event>, Meta) {
    let cov = Coverage { first_h: a, first_t: 1_790_000_000 + a, last_h: b, last_t: 1_790_000_000 + b };
    (vec![event(a), event(b)], Meta { version: FORMAT_VERSION, cov, skipped: Some(BTreeMap::from([("no_net_crossing".into(), 3)])) })
}

const OTHER_VERSION: &str = "data from another zcrowd version: run `zcrowd sync` again";

#[test]
fn save_then_load_round_trips_events_and_meta() {
    let p = dir("round-trip").join("flows.ndjson");
    let (ev, meta) = window(100, 200);
    save(&p, &ev, &meta).unwrap();
    let (got, got_meta) = load(&p).unwrap();
    assert_eq!((got, got_meta), (ev, meta));
    // no temp file is left behind
    let mut names: Vec<_> =
        std::fs::read_dir(p.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    assert_eq!(names, ["flows.meta.json", "flows.ndjson"]);
}

#[test]
fn a_meta_without_this_version_is_refused() {
    let p = dir("version").join("flows.ndjson");
    let (ev, meta) = window(100, 200);
    save(&p, &ev, &meta).unwrap();
    let mp = Coverage::path_for(&p);
    // written before the Ironwood pool: the four window fields and no version
    std::fs::write(&mp, r#"{"first_h":100,"first_t":1790000100,"last_h":200,"last_t":1790000200}"#).unwrap();
    assert_eq!(read_meta(&p).unwrap_err().to_string(), OTHER_VERSION);
    assert_eq!(load(&p).unwrap_err().to_string(), OTHER_VERSION);
    for v in [0, 1, FORMAT_VERSION + 1] {
        let json = format!(r#"{{"version":{v},"first_h":100,"first_t":1790000100,"last_h":200,"last_t":1790000200}}"#);
        std::fs::write(&mp, json).unwrap();
        assert_eq!(read_meta(&p).unwrap_err().to_string(), OTHER_VERSION, "version {v}");
    }
    // not a meta at all
    std::fs::write(&mp, "{").unwrap();
    assert_eq!(read_meta(&p).unwrap_err().to_string(), OTHER_VERSION);
    // no meta at all: another message, same advice
    std::fs::remove_file(&mp).unwrap();
    let e = read_meta(&p).unwrap_err().to_string();
    assert!(e.contains("no coverage file") && e.ends_with("run `zcrowd sync` again"), "{e}");
}

#[test]
fn a_save_that_stops_between_the_two_renames_leaves_no_window_to_trust() {
    let p = dir("crash").join("flows.ndjson");
    let (old_ev, old_meta) = window(100, 200);
    save(&p, &old_ev, &old_meta).unwrap();
    // the next sync covers other blocks; the process dies after the events are in place and before the meta is
    let (new_ev, new_meta) = window(300, 400);
    let err = save_with(&p, &new_ev, &new_meta, || bail!("injected failure")).unwrap_err();
    assert_eq!(err.to_string(), "injected failure");
    // new events, and no meta at all: neither the old window (it would claim blocks 100..200) nor the new one (events
    // were written, the window was not confirmed) is offered
    assert_eq!(zcrowd_core::model::read_ndjson(&p).unwrap(), new_ev);
    let e = load(&p).unwrap_err().to_string();
    assert!(e.contains("no coverage file") && e.ends_with("run `zcrowd sync` again"), "{e}");
    // running sync again finishes the job: the window is exactly the new events' range
    save(&p, &new_ev, &new_meta).unwrap();
    let (ev, meta) = load(&p).unwrap();
    assert_eq!((ev.first().unwrap().h, ev.last().unwrap().h), (meta.cov.first_h, meta.cov.last_h));
    assert_eq!(meta, new_meta);
}

#[test]
fn a_save_that_fails_before_the_events_are_replaced_keeps_the_old_pair() {
    let d = dir("early");
    let p = d.join("flows.ndjson");
    let (old_ev, old_meta) = window(100, 200);
    save(&p, &old_ev, &old_meta).unwrap();
    // the temp file cannot be created (a directory sits on its name): the save fails at its first write
    std::fs::create_dir(p.with_extension("tmp")).unwrap();
    let (new_ev, new_meta) = window(300, 400);
    assert!(save(&p, &new_ev, &new_meta).is_err());
    assert_eq!(load(&p).unwrap(), (old_ev, old_meta));
}
