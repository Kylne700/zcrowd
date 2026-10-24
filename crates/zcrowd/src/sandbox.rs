//! Cut this process off from the network before it touches any user input.
//! `check`, `report` and `verify` have no network code; this makes that a property of the process,
//! not just of the source. Must run before any thread exists, so it is the first thing `main` does.

use std::io::IsTerminal;

/// One layer of the sandbox: its name, and whether it is in place (else why not).
type Layer = (&'static str, Result<(), String>);

/// Puts in place every layer this OS has; true if at least one is. One note on stderr names them. It is always printed
/// when a layer is missing; with every layer in place only on a terminal, so that `--json 2>&1` stays plain JSON.
pub fn enter() -> bool {
    #[cfg(target_os = "linux")]
    let layers = install(true);
    // Windows and the rest: nothing light cuts one running process off (AppContainer is a different way to start one).
    #[cfg(not(target_os = "linux"))]
    let layers: Vec<Layer> = Vec::new();
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

/// Linux: an empty network namespace (it needs a single thread, so it comes first), then the seccomp filter, which also
/// holds where user namespaces are refused (Ubuntu 24.04 by default). `netns: false` is for the test of the filter alone.
#[cfg(target_os = "linux")]
fn install(netns: bool) -> Vec<Layer> {
    let mut layers = Vec::new();
    if netns {
        // SAFETY: unshare takes only flags; no memory is passed.
        let r = unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) };
        layers.push(("empty network namespace", if r == 0 { Ok(()) } else { Err(std::io::Error::last_os_error().to_string()) }));
    }
    layers.push(("seccomp socket filter", seccomp()));
    layers
}

/// Refuses (EPERM) `socket` for every family, `socketpair`, and io_uring (it can open sockets past seccomp), on every thread
/// and for good: a filter cannot be removed and passes to children. `check`, `report` and `verify` use none of them.
#[cfg(all(
    target_os = "linux",
    target_pointer_width = "64",
    any(target_arch = "x86_64", all(target_arch = "aarch64", target_endian = "little"))
))]
fn seccomp() -> Result<(), String> {
    use libc::{BPF_ABS, BPF_JEQ, BPF_JMP, BPF_JSET, BPF_K, BPF_LD, BPF_RET, BPF_W, c_ulong, sock_filter};
    #[cfg(target_arch = "x86_64")]
    const ARCH: u32 = 0xc000_003e; // AUDIT_ARCH_X86_64
    #[cfg(target_arch = "aarch64")]
    const ARCH: u32 = 0xc000_00b7; // AUDIT_ARCH_AARCH64
    const REFUSE: u32 = libc::SECCOMP_RET_ERRNO | libc::EPERM as u32;
    let op = |code: u32, k: u32, jt: u8, jf: u8| sock_filter { code: code as u16, jt, jf, k };
    let load = |offset: usize| op(BPF_LD | BPF_W | BPF_ABS, offset as u32, 0, 0);
    let ret = |k: u32| op(BPF_RET | BPF_K, k, 0, 0);
    // If the test holds, fall through to the refusal; else jump over it.
    let refuse_if = |test: u32, k: u32| [op(BPF_JMP | test | BPF_K, k, 0, 1), ret(REFUSE)];
    // Another ABI (i386 through int 0x80 on x86_64) numbers its calls differently, so it is refused whole.
    let mut f = vec![load(std::mem::offset_of!(libc::seccomp_data, arch)), op(BPF_JMP | BPF_JEQ | BPF_K, ARCH, 1, 0), ret(REFUSE)];
    f.push(load(std::mem::offset_of!(libc::seccomp_data, nr)));
    #[cfg(target_arch = "x86_64")]
    f.extend(refuse_if(BPF_JSET, 0x4000_0000)); // x32 numbers: the same calls under other numbers
    for nr in [libc::SYS_socket, libc::SYS_socketpair, libc::SYS_io_uring_setup, libc::SYS_io_uring_enter, libc::SYS_io_uring_register] {
        f.extend(refuse_if(BPF_JEQ, nr as u32));
    }
    f.push(ret(libc::SECCOMP_RET_ALLOW));
    let prog = libc::sock_fprog { len: f.len() as u16, filter: f.as_mut_ptr() };
    // SAFETY: prctl takes integers only.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1 as c_ulong, 0 as c_ulong, 0 as c_ulong, 0 as c_ulong) } != 0 {
        return Err(format!("no_new_privs: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: prog points at f, which outlives the call; the kernel copies the program.
    let r = unsafe {
        libc::syscall(libc::SYS_seccomp, libc::SECCOMP_SET_MODE_FILTER as c_ulong, libc::SECCOMP_FILTER_FLAG_TSYNC, &raw const prog)
    };
    match r {
        0 => Ok(()),
        -1 => Err(format!("seccomp: {}", std::io::Error::last_os_error())),
        tid => Err(format!("seccomp: thread {tid} could not take the filter")),
    }
}

/// The filter needs this ABI's audit arch and call numbers; only those two are written down and tested.
#[cfg(all(
    target_os = "linux",
    not(all(target_pointer_width = "64", any(target_arch = "x86_64", all(target_arch = "aarch64", target_endian = "little"))))
))]
fn seccomp() -> Result<(), String> {
    Err("not built for this CPU architecture".into())
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

    /// These put the sandbox on a child process: on Linux only where the filter is built, and on macOS.
    #[cfg(any(
        target_os = "macos",
        all(
            target_os = "linux",
            target_pointer_width = "64",
            any(target_arch = "x86_64", all(target_arch = "aarch64", target_endian = "little"))
        )
    ))]
    mod child {
        use super::*;

        /// Runs the named test again in a child process with `ZCROWD_SANDBOX_CHILD` set, so that the sandbox lands on the child,
        /// never on the test harness. True in that child.
        fn in_child(name: &str) -> bool {
            if std::env::var_os("ZCROWD_SANDBOX_CHILD").is_some() {
                return true;
            }
            let test = format!("{}::{name}", module_path!().split_once("::").unwrap().1);
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([test.as_str(), "--exact", "--nocapture", "--test-threads=1"])
                .env("ZCROWD_SANDBOX_CHILD", "1")
                .output()
                .unwrap();
            let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            assert!(out.status.success() && log.contains("test result: ok. 1 passed"), "{test} in a child: {}\n{log}", out.status);
            false
        }

        /// A loopback port nothing listens on: without the sandbox a connection to it is refused.
        fn closed_port() -> std::net::SocketAddr {
            let addr = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
            assert_eq!(std::net::TcpStream::connect(addr).unwrap_err().kind(), std::io::ErrorKind::ConnectionRefused);
            addr
        }

        /// Every way to a new socket these tests know: each must be refused with EPERM.
        #[cfg(target_os = "linux")]
        fn every_socket_is_refused(closed: std::net::SocketAddr) {
            use libc::c_long;
            use std::io::ErrorKind::PermissionDenied;
            use std::os::unix::net::{UnixDatagram, UnixStream};
            assert_eq!(std::net::TcpStream::connect(closed).unwrap_err().kind(), PermissionDenied, "tcp connect");
            assert_eq!(std::net::UdpSocket::bind("127.0.0.1:0").unwrap_err().kind(), PermissionDenied, "udp bind");
            assert_eq!(UnixDatagram::unbound().unwrap_err().kind(), PermissionDenied, "unix socket");
            assert_eq!(UnixStream::pair().unwrap_err().kind(), PermissionDenied, "socketpair");
            let errno = |r: c_long| if r < 0 { Err(std::io::Error::last_os_error().raw_os_error()) } else { Ok(r) };
            let eperm = Err(Some(libc::EPERM));
            let mut params = [0u64; 15]; // struct io_uring_params: 120 bytes
            // SAFETY: params is writable and as large as the kernel's struct; the other calls get a bad fd and no memory.
            unsafe {
                assert_eq!(errno(libc::syscall(libc::SYS_io_uring_setup, 1i64, params.as_mut_ptr())), eperm, "io_uring_setup");
                assert_eq!(errno(libc::syscall(libc::SYS_io_uring_enter, -1i64, 0i64, 0i64, 0i64, 0i64, 0i64)), eperm, "io_uring_enter");
                assert_eq!(errno(libc::syscall(libc::SYS_io_uring_register, -1i64, 0i64, 0i64, 0i64)), eperm, "io_uring_register");
            }
            #[cfg(target_arch = "x86_64")]
            {
                // SAFETY: socket(AF_INET, SOCK_STREAM, 0) under its x32 number; no memory is passed.
                let x32 =
                    unsafe { libc::syscall(libc::SYS_socket | 0x4000_0000, libc::AF_INET as c_long, libc::SOCK_STREAM as c_long, 0i64) };
                assert_eq!(errno(x32), eperm, "x32 socket");
                assert_eq!(i386_socket(), -libc::EPERM, "i386 socket (int 0x80)");
            }
        }

        /// socket(AF_INET, SOCK_STREAM, 0) through the i386 table (int 0x80), where it is number 359; returns eax.
        #[cfg(target_arch = "x86_64")]
        fn i386_socket() -> i32 {
            let mut ax: u64 = 359;
            // SAFETY: int 0x80 takes its arguments in ebx, ecx and edx and gets no memory here. rbx cannot be an asm operand
            // (LLVM keeps it), so it is swapped in and back; r8-r11 may be clobbered on this path.
            unsafe {
                std::arch::asm!(
                    "xchg {b}, rbx", "int 0x80", "xchg {b}, rbx",
                    b = inout(reg) 2u64 => _, inout("rax") ax, in("rcx") 1u64, in("rdx") 0u64,
                    out("r8") _, out("r9") _, out("r10") _, out("r11") _,
                );
            }
            ax as i32
        }

        /// `enter()` as `check`, `report` and `verify` call it. A test runs on a thread of its own, so the namespace is refused
        /// here (it needs a single thread) and this proves the seccomp layer as `enter()` installs it; tests/cli.rs has both.
        #[test]
        #[cfg(target_os = "linux")]
        fn enter_refuses_tcp_udp_unix_and_io_uring_in_a_child() {
            if !in_child("enter_refuses_tcp_udp_unix_and_io_uring_in_a_child") {
                return;
            }
            let closed = closed_port();
            let (go, wait) = std::sync::mpsc::channel::<()>();
            let older = std::thread::spawn(move || {
                wait.recv().unwrap();
                std::net::UdpSocket::bind("127.0.0.1:0").err().map(|e| e.kind())
            });
            assert!(enter());
            go.send(()).unwrap();
            assert_eq!(older.join().unwrap(), Some(std::io::ErrorKind::PermissionDenied), "a thread from before enter() (TSYNC)");
            every_socket_is_refused(closed);
        }

        /// The seccomp layer alone, as on a host that refuses user namespaces (Ubuntu 24.04 by default).
        #[test]
        #[cfg(target_os = "linux")]
        fn the_seccomp_filter_alone_refuses_them_too() {
            if !in_child("the_seccomp_filter_alone_refuses_them_too") {
                return;
            }
            let closed = closed_port();
            assert_eq!(install(false), vec![("seccomp socket filter", Ok(()))]);
            every_socket_is_refused(closed);
        }
    }
}
