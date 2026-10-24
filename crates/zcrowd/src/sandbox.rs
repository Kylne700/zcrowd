//! Cut this process off from the network before it touches any user input.
//! `check`, `report` and `verify` have no network code; this makes that a property of the process,
//! not just of the source. Must run before any thread exists, so it is the first thing `main` does.

/// Returns true if the process is now in an empty network namespace.
pub fn enter() -> bool {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: unshare takes only flags; no memory is passed.
        let r = unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) };
        if r == 0 {
            return true;
        }
        eprintln!(
            "note: could not enter a network-less sandbox ({}); this command has no network code anyway",
            std::io::Error::last_os_error()
        );
        false
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("note: no network sandbox on this OS; this command has no network code anyway");
        false
    }
}
