//! Cut this process off from the network before it touches any user input.
//! `check`, `report` and `verify` have no network code; this makes that a property of the process,
//! not just of the source. Must run before any thread exists, so it is the first thing `main` does.

use std::io::IsTerminal;

/// One layer of the sandbox: its name, and whether it is in place (else why not).
type Layer = (&'static str, Result<(), String>);

/// Puts in place every layer this OS has; true if at least one is. One note on stderr names them. It is always printed
/// when a layer is missing; with every layer in place only on a terminal, so that `--json 2>&1` stays plain JSON.
pub fn enter() -> bool {
    let layers = install();
    if layers.is_empty() || layers.iter().any(|l| l.1.is_err()) || std::io::stderr().is_terminal() {
        eprintln!("{}", note(&layers));
    }
    layers.iter().any(|l| l.1.is_ok())
}

/// The note names the layers in place and the missing ones with the reason. It says "cut" only if a layer is in place.
fn note(layers: &[Layer]) -> String {
    let on: Vec<&str> = layers.iter().filter(|l| l.1.is_ok()).map(|l| l.0).collect();
    let off: Vec<String> = layers.iter().filter_map(|(n, r)| r.as_ref().err().map(|e| format!("no {n}: {e}"))).collect();
    if on.is_empty() {
        let why = if off.is_empty() { "no sandbox on this OS".to_string() } else { off.join("; ") };
        return format!("note: network NOT cut for this command ({why}); it has no network code, but nothing stops it at run time");
    }
    let missing = if off.is_empty() { String::new() } else { format!(" ({})", off.join("; ")) };
    format!("note: network cut for this command: {}{missing}", on.join(" + "))
}

#[cfg(target_os = "linux")]
fn install() -> Vec<Layer> {
    // SAFETY: unshare takes only flags; no memory is passed.
    let r = unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) };
    vec![("empty network namespace", if r == 0 { Ok(()) } else { Err(std::io::Error::last_os_error().to_string()) })]
}

/// Windows and the rest: nothing light cuts one running process off (AppContainer is a different way to start one).
#[cfg(not(target_os = "linux"))]
fn install() -> Vec<Layer> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_names_the_layers_in_place_and_says_not_cut_without_one() {
        let (ns, filter, mac) = ("empty network namespace", "seccomp socket filter", "macOS sandbox (no-network)");
        let e = |why: &str| Err(why.to_string());
        let eperm = "Operation not permitted (os error 1)";
        let free = "it has no network code, but nothing stops it at run time";
        let cases = [
            (vec![(ns, Ok(())), (filter, Ok(()))], "note: network cut for this command: empty network namespace + seccomp socket filter".into()),
            (
                vec![(ns, e(eperm)), (filter, Ok(()))],
                format!("note: network cut for this command: seccomp socket filter (no empty network namespace: {eperm})"),
            ),
            (
                vec![(ns, Ok(())), (filter, e("not built for this CPU architecture"))],
                "note: network cut for this command: empty network namespace (no seccomp socket filter: not built for this CPU architecture)"
                    .into(),
            ),
            (
                vec![(ns, e(eperm)), (filter, e("seccomp: Invalid argument (os error 22)"))],
                format!(
                    "note: network NOT cut for this command (no empty network namespace: {eperm}; no seccomp socket filter: seccomp: Invalid argument (os error 22)); {free}"
                ),
            ),
            (vec![(mac, Ok(()))], "note: network cut for this command: macOS sandbox (no-network)".into()),
            (vec![(mac, e("bad profile"))], format!("note: network NOT cut for this command (no macOS sandbox (no-network): bad profile); {free}")),
            (vec![], format!("note: network NOT cut for this command (no sandbox on this OS); {free}")),
        ];
        for (layers, want) in cases {
            assert_eq!(note(&layers), want);
        }
    }
}
