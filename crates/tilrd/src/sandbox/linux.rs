//! The sandbox on Linux (S5): a Landlock ruleset for files and TCP, a seccomp filter
//! for what Landlock does not govern.
//!
//! Both are built in the pane process before the fork ([`Linux::prepare`]): the ruleset
//! as a Landlock descriptor holding every rule, the filter as a BPF program. The child
//! only installs them ([`Linux::apply_in_child`]), with raw syscalls that are
//! async-signal-safe: mark every descriptor above 2 close-on-exec, `PR_SET_NO_NEW_PRIVS`,
//! `landlock_restrict_self`, `seccomp(SECCOMP_SET_MODE_FILTER)`. It runs after `setsid`
//! and `TIOCSCTTY`, so the terminal and job control are set up before the restriction,
//! and no ioctl is filtered.
//!
//! The ruleset handles every filesystem right of the ABI it is built for (no best-effort
//! downgrade: the `landlock` crate in `HardRequirement` mode), so a right the ruleset
//! does not grant is refused.

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use landlock::{
    ABI, Access, AccessFs, AccessNet, BitFlags, CompatLevel, Compatible, PathBeneath, Ruleset,
    RulesetAttr, RulesetCreated, RulesetCreatedAttr, Scope,
};

use super::{Access as Grantee, Dirs, Features, Grant, Network, Platform};

pub struct Linux;

/// The restriction of one pane, built before the fork.
pub struct Prepared {
    /// The Landlock ruleset (close-on-exec: it does not reach the program).
    ruleset: OwnedFd,
    filter: Vec<libc::sock_filter>,
}

/// The syscalls the filter looks at on one architecture, by number. A filter checks the
/// `AUDIT_ARCH_*` first, so a syscall made through another ABI (32-bit, x32) cannot slip
/// past it with numbers of its own. Both tables are built on every Linux host, so the
/// tests check the filter of each.
#[derive(Clone, Copy, Debug)]
struct Arch {
    audit: u32,
    /// x86_64: an x32 syscall has the same `AUDIT_ARCH` and bit 30 set in its number.
    x32: bool,
    socket: u32,
    socketpair: u32,
    listen: u32,
    io_uring: [u32; 3],
    /// `keyctl`, `add_key`, `request_key`, `bpf`, `perf_event_open`, `userfaultfd`,
    /// `setns`: refused with `EPERM`, as Docker's default profile does.
    refused: [u32; 7],
    unshare: u32,
    clone: u32,
    clone3: u32,
}

#[cfg(any(test, target_arch = "x86_64"))]
const X86_64: Arch = Arch {
    audit: 0xC000_003E,
    x32: true,
    socket: 41,
    socketpair: 53,
    listen: 50,
    io_uring: [425, 426, 427],
    refused: [250, 248, 249, 321, 298, 323, 308],
    unshare: 272,
    clone: 56,
    clone3: 435,
};

#[cfg(any(test, target_arch = "aarch64"))]
const AARCH64: Arch = Arch {
    audit: 0xC000_00B7,
    x32: false,
    socket: 198,
    socketpair: 199,
    listen: 201,
    io_uring: [425, 426, 427],
    refused: [219, 217, 218, 280, 241, 282, 268],
    unshare: 97,
    clone: 220,
    clone3: 435,
};

/// The table of the architecture this build runs on.
#[cfg(all(target_arch = "x86_64", target_endian = "little"))]
const HOST: Option<Arch> = Some(X86_64);
#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
const HOST: Option<Arch> = Some(AARCH64);
#[cfg(not(any(
    all(target_arch = "x86_64", target_endian = "little"),
    all(target_arch = "aarch64", target_endian = "little")
)))]
const HOST: Option<Arch> = None;

/// The namespace flags of `clone` and `unshare` (`CLONE_NEWNS`, `CLONE_NEWCGROUP`,
/// `CLONE_NEWUTS`, `CLONE_NEWIPC`, `CLONE_NEWUSER`, `CLONE_NEWPID`, `CLONE_NEWNET`).
const NAMESPACES: u32 = 0x7E02_0000;

/// `LANDLOCK_CREATE_RULESET_VERSION`.
const LANDLOCK_VERSION: u32 = 1;

/// The Landlock ABI of the running kernel, or why there is none.
fn landlock_abi() -> Result<u32, String> {
    // SAFETY: with a null attribute, size 0 and the version flag, the call only
    // returns the ABI version (or -1); it reads and writes no memory.
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<u8>(),
            0usize,
            LANDLOCK_VERSION,
        )
    };
    if abi < 0 {
        let err = io::Error::last_os_error();
        return Err(match err.raw_os_error() {
            Some(libc::ENOSYS) => "the kernel has no Landlock".into(),
            Some(libc::EOPNOTSUPP) => "Landlock is not enabled in this kernel".into(),
            _ => format!("Landlock: {err}"),
        });
    }
    u32::try_from(abi).map_err(|_| format!("Landlock: ABI {abi}"))
}

/// Whether seccomp can return `action` for a filtered syscall.
fn seccomp_action(action: u32) -> io::Result<()> {
    // SAFETY: SECCOMP_GET_ACTION_AVAIL reads one u32 from the pointer, which is valid.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_GET_ACTION_AVAIL,
            0u32,
            &raw const action,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

impl Platform for Linux {
    type Prepared = Prepared;

    /// Landlock ABI 4 (Linux 6.7: TCP rules) or later and seccomp filters with the
    /// actions the filter uses.
    fn check() -> Result<Features, String> {
        if HOST.is_none() {
            return Err("no seccomp filter for this architecture".into());
        }
        let abi = landlock_abi()?;
        if abi < 4 {
            return Err(format!("Landlock ABI {abi} < 4 (Linux 6.7)"));
        }
        for (action, name) in [
            (libc::SECCOMP_RET_ERRNO, "SECCOMP_RET_ERRNO"),
            (libc::SECCOMP_RET_KILL_PROCESS, "SECCOMP_RET_KILL_PROCESS"),
            (libc::SECCOMP_RET_ALLOW, "SECCOMP_RET_ALLOW"),
        ] {
            seccomp_action(action).map_err(|err| format!("seccomp {name}: {err}"))?;
        }
        Ok(Features { landlock_abi: abi })
    }

    fn prepare(
        grants: &[Grant],
        network: Network,
        features: Features,
        dirs: &Dirs,
    ) -> Result<Prepared, String> {
        let abi = ABI::from(i32::try_from(features.landlock_abi).unwrap_or(i32::MAX));
        let ruleset = ruleset(grants, network, abi, dirs)?;
        let ruleset: Option<OwnedFd> = ruleset.into();
        let ruleset = ruleset.ok_or("Landlock: the ruleset has no descriptor")?;
        let filter = filter(
            network,
            &HOST.ok_or("no seccomp filter for this architecture")?,
        );
        Ok(Prepared { ruleset, filter })
    }

    fn apply_in_child(prepared: &Prepared) -> io::Result<()> {
        let check = |rc: libc::c_long| {
            if rc == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        // SAFETY: raw syscalls with plain integer arguments, and for seccomp a pointer
        // to a `sock_fprog` on this stack whose filter lives in `prepared`; all four are
        // async-signal-safe and allocate nothing.
        unsafe {
            // Nothing the pane process holds reaches the program, whatever it forgot
            // to mark: a Unix socket handed down would bypass the filter.
            check(libc::syscall(
                libc::SYS_close_range,
                3u32,
                u32::MAX,
                libc::CLOSE_RANGE_CLOEXEC,
            ))?;
            check(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0).into())?;
            check(libc::syscall(
                libc::SYS_landlock_restrict_self,
                prepared.ruleset.as_raw_fd(),
                0u32,
            ))?;
            let program = libc::sock_fprog {
                len: prepared.filter.len() as libc::c_ushort,
                filter: prepared.filter.as_ptr().cast_mut(),
            };
            check(libc::syscall(
                libc::SYS_seccomp,
                libc::SECCOMP_SET_MODE_FILTER,
                0u32,
                &raw const program,
            ))?;
        }
        Ok(())
    }
}

/// The paths every sandboxed pane can use (S5): programs and libraries, the system's
/// configuration and process information, the terminal devices.
const SYSTEM_PROGRAMS: [&str; 6] = ["/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64"];
const SYSTEM_READ: [&str; 3] = ["/etc", "/proc", "/sys"];
/// Not `/dev/pts`: any terminal of the user is there, other panes' too. The pane's own
/// terminal is granted by name ([`Grantee::Terminal`]).
const DEVICES: [&str; 7] = [
    "/dev/null",
    "/dev/zero",
    "/dev/full",
    "/dev/random",
    "/dev/urandom",
    "/dev/tty",
    "/dev/ptmx",
];

/// The Landlock ruleset of a pane, with every rule added.
///
/// Each path is opened once (`O_PATH`), a grant of the profile is checked against the
/// server's directories by what was opened (S6), and that same descriptor becomes the
/// rule: a path swapped for a link after [`plan`](super::plan) looked at it cannot slip a
/// server directory in.
fn ruleset(
    grants: &[Grant],
    network: Network,
    abi: ABI,
    dirs: &Dirs,
) -> Result<RulesetCreated, String> {
    let landlock = |err: landlock::RulesetError| format!("Landlock: {err}");
    let all = AccessFs::from_all(abi);
    let read_exec = all & (AccessFs::ReadFile | AccessFs::ReadDir | AccessFs::Execute);
    let read = all & (AccessFs::ReadFile | AccessFs::ReadDir);
    let device =
        all & (AccessFs::ReadFile | AccessFs::WriteFile | AccessFs::Truncate | AccessFs::IoctlDev);
    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(all)
        .map_err(landlock)?;
    let net: BitFlags<AccessNet> = match network {
        Network::None => AccessNet::from_all(abi),
        Network::Client => AccessNet::BindTcp.into(),
        Network::Server => BitFlags::empty(),
    };
    if !net.is_empty() {
        ruleset = ruleset.handle_access(net).map_err(landlock)?;
    }
    let scopes = Scope::from_all(abi);
    if !scopes.is_empty() {
        ruleset = ruleset.scope(scopes).map_err(landlock)?;
    }
    let mut created = ruleset.create().map_err(landlock)?;
    /// A rule to add: the path, its rights, and for a grant of the request whether it
    /// is checked against the server's directories (and as a write).
    struct Rule<'a> {
        path: &'a Path,
        access: BitFlags<AccessFs>,
        asked: bool,
        check: Option<bool>,
    }
    let system = |path: &'static str, access| Rule {
        path: Path::new(path),
        access,
        asked: false,
        check: None,
    };
    let mut rules: Vec<Rule<'_>> = Vec::new();
    for path in SYSTEM_PROGRAMS {
        rules.push(system(path, read_exec));
    }
    for path in SYSTEM_READ {
        rules.push(system(path, read));
    }
    for path in DEVICES {
        rules.push(system(path, device));
    }
    // /etc/resolv.conf is often a link out of /etc (systemd-resolved's /run/...):
    // what a client needs to look names up.
    let resolv = std::fs::canonicalize("/etc/resolv.conf").ok();
    if network != Network::None
        && let Some(resolv) = &resolv
        && !resolv.starts_with("/etc")
    {
        rules.push(Rule {
            path: resolv,
            access: read,
            asked: false,
            check: None,
        });
    }
    for grant in grants {
        let access = match grant.access {
            Grantee::ReadOnly => read_exec,
            Grantee::ReadWrite => all,
            Grantee::Program => all & (AccessFs::ReadFile | AccessFs::Execute),
            Grantee::Terminal => device,
        };
        rules.push(Rule {
            path: &grant.path,
            access,
            asked: true,
            check: grant.checked.then_some(grant.access == Grantee::ReadWrite),
        });
    }
    for rule in rules {
        let path = rule.path;
        // O_PATH: a handle on the place, opened neither for reading nor writing.
        let file = match File::options()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
            .open(path)
        {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                if rule.asked {
                    tracing::info!("sandbox: {} does not exist, not granted", path.display());
                }
                continue;
            }
            Err(err) => return Err(format!("open {}: {err}", path.display())),
        };
        let meta = file
            .metadata()
            .map_err(|err| format!("stat {}: {err}", path.display()))?;
        if let Some(write) = rule.check {
            // The kernel's name of what was opened, whatever links led there.
            let opened = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
                .map_err(|err| format!("the opened {}: {err}", path.display()))?;
            super::refuse_opened(path, &opened, (meta.dev(), meta.ino()), write, dirs)?;
        }
        let access = if meta.is_dir() {
            rule.access
        } else {
            rule.access & AccessFs::from_file(abi)
        };
        if access.is_empty() {
            continue;
        }
        created = created
            .add_rule(PathBeneath::new(file, access))
            .map_err(|err| format!("Landlock: {}: {err}", path.display()))?;
    }
    Ok(created)
}

/// Assembles a classic BPF program with forward jumps to labels.
struct Asm {
    code: Vec<(libc::sock_filter, Option<(Label, Label)>)>,
    labels: Vec<(Label, usize)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Label {
    Next,
    Allow,
    Deny,
    Kill,
    NoSys,
    EPerm,
    Inet,
    Stream,
    Netlink,
    Pair,
    Namespaces,
}

const ARCH_OFFSET: u32 = 4;
const NR_OFFSET: u32 = 0;
/// The low 32 bits of argument `n` (little endian): what the kernel reads of an `int`.
const fn arg(n: u32) -> u32 {
    16 + 8 * n
}

impl Asm {
    fn new() -> Self {
        Asm {
            code: Vec::new(),
            labels: Vec::new(),
        }
    }

    fn stmt(&mut self, code: u32, k: u32) {
        self.code.push((
            libc::sock_filter {
                code: code as u16,
                jt: 0,
                jf: 0,
                k,
            },
            None,
        ));
    }

    fn load(&mut self, offset: u32) {
        self.stmt(libc::BPF_LD | libc::BPF_W | libc::BPF_ABS, offset);
    }

    fn jump(&mut self, op: u32, k: u32, jt: Label, jf: Label) {
        self.code.push((
            libc::sock_filter {
                code: (libc::BPF_JMP | op | libc::BPF_K) as u16,
                jt: 0,
                jf: 0,
                k,
            },
            Some((jt, jf)),
        ));
    }

    /// Jump to `to` if the accumulator equals `k`, else go on.
    fn if_eq(&mut self, k: u32, to: Label) {
        self.jump(libc::BPF_JEQ, k, to, Label::Next);
    }

    fn and(&mut self, k: u32) {
        self.stmt(libc::BPF_ALU | libc::BPF_AND | libc::BPF_K, k);
    }

    fn label(&mut self, label: Label) {
        self.labels.push((label, self.code.len()));
    }

    fn ret(&mut self, value: u32) {
        self.stmt(libc::BPF_RET | libc::BPF_K, value);
    }

    fn finish(self) -> Vec<libc::sock_filter> {
        let at = |label: Label| {
            self.labels
                .iter()
                .find(|(l, _)| *l == label)
                .map(|(_, at)| *at)
                .unwrap_or_else(|| panic!("label {label:?} is not placed"))
        };
        self.code
            .iter()
            .enumerate()
            .map(|(i, (insn, jumps))| {
                let mut insn = *insn;
                if let Some((jt, jf)) = jumps {
                    let offset = |label: Label| {
                        if label == Label::Next {
                            return 0;
                        }
                        let to = at(label);
                        assert!(to > i, "only forward jumps");
                        u8::try_from(to - i - 1).expect("jump within 255 instructions")
                    };
                    insn.jt = offset(*jt);
                    insn.jf = offset(*jf);
                }
                insn
            })
            .collect()
    }
}

/// The seccomp filter of a pane (S5), for the syscall numbers of `arch`:
///
/// - a syscall of another ABI kills the process (32-bit), or gets `ENOSYS` (x32);
/// - io_uring is refused always (its socket and connect operations bypass the filter);
/// - `keyctl`, `add_key`, `request_key` (the user's keyrings), `bpf`, `perf_event_open`,
///   `userfaultfd` and `setns` get `EPERM`;
/// - `clone` and `unshare` with a namespace flag get `EPERM` (a user namespace would
///   give the pane capabilities of its own), `clone3` gets `ENOSYS` (its flags are behind
///   a pointer; glibc then uses `clone`), as Docker and Flatpak do;
/// - `socket(AF_UNIX)` is refused always (Landlock does not govern `connect` to a named
///   Unix socket, and the server's sockets accept any process of the user), and
///   `socketpair` gives only a connected pair of streams or of sequenced packets (an
///   unbound datagram socket could `sendto` any named or abstract one);
/// - netlink only as `NETLINK_ROUTE` (protocol 0: interfaces and addresses, which
///   `getaddrinfo` reads);
/// - with `none` every other socket is refused, with `client` all but TCP streams and
///   datagrams of IPv4 and IPv6, with both `listen` (which would bind an unbound TCP
///   socket to a free port behind Landlock's back).
///
/// Everything else is allowed.
fn filter(network: Network, arch: &Arch) -> Vec<libc::sock_filter> {
    let errno = |errno: i32| libc::SECCOMP_RET_ERRNO | (errno as u32 & libc::SECCOMP_RET_DATA);
    let deny = errno(libc::EACCES);
    let mut asm = Asm::new();
    asm.load(ARCH_OFFSET);
    asm.jump(libc::BPF_JEQ, arch.audit, Label::Next, Label::Kill);
    asm.load(NR_OFFSET);
    if arch.x32 {
        asm.jump(libc::BPF_JGE, 0x4000_0000, Label::NoSys, Label::Next);
    }
    for nr in arch.io_uring {
        asm.if_eq(nr, Label::Deny);
    }
    for nr in arch.refused {
        asm.if_eq(nr, Label::EPerm);
    }
    asm.if_eq(arch.clone3, Label::NoSys);
    asm.if_eq(arch.unshare, Label::Namespaces);
    asm.if_eq(arch.clone, Label::Namespaces);
    if network != Network::Server {
        asm.if_eq(arch.listen, Label::Deny);
    }
    asm.if_eq(arch.socketpair, Label::Pair);
    asm.jump(libc::BPF_JEQ, arch.socket, Label::Next, Label::Allow);
    asm.load(arg(0));
    asm.if_eq(libc::AF_UNIX as u32, Label::Deny);
    asm.if_eq(libc::AF_NETLINK as u32, Label::Netlink);
    match network {
        Network::None => asm.ret(deny),
        Network::Server => {
            asm.if_eq(libc::AF_INET as u32, Label::Allow);
            asm.if_eq(libc::AF_INET6 as u32, Label::Allow);
            asm.ret(deny);
        }
        Network::Client => {
            asm.if_eq(libc::AF_INET as u32, Label::Inet);
            asm.if_eq(libc::AF_INET6 as u32, Label::Inet);
            asm.ret(deny);
            asm.label(Label::Inet);
            asm.load(arg(1));
            asm.and(SOCK_TYPE);
            asm.if_eq(libc::SOCK_DGRAM as u32, Label::Allow);
            asm.if_eq(libc::SOCK_STREAM as u32, Label::Stream);
            asm.ret(deny);
            asm.label(Label::Stream);
            // TCP only: SCTP and MPTCP streams are not under Landlock's TCP rules.
            asm.load(arg(2));
            asm.if_eq(0, Label::Allow);
            asm.if_eq(libc::IPPROTO_TCP as u32, Label::Allow);
            asm.ret(deny);
        }
    }
    asm.label(Label::Netlink);
    asm.load(arg(2));
    asm.jump(
        libc::BPF_JEQ,
        libc::NETLINK_ROUTE as u32,
        Label::Allow,
        Label::Deny,
    );
    asm.label(Label::Pair);
    asm.load(arg(0));
    asm.jump(
        libc::BPF_JEQ,
        libc::AF_UNIX as u32,
        Label::Next,
        Label::Deny,
    );
    asm.load(arg(1));
    asm.and(SOCK_TYPE);
    asm.if_eq(libc::SOCK_STREAM as u32, Label::Allow);
    asm.if_eq(libc::SOCK_SEQPACKET as u32, Label::Allow);
    asm.ret(deny);
    asm.label(Label::Namespaces);
    asm.load(arg(0));
    asm.jump(libc::BPF_JSET, NAMESPACES, Label::EPerm, Label::Allow);
    asm.label(Label::Allow);
    asm.ret(libc::SECCOMP_RET_ALLOW);
    asm.label(Label::Deny);
    asm.ret(deny);
    asm.label(Label::EPerm);
    asm.ret(errno(libc::EPERM));
    asm.label(Label::NoSys);
    asm.ret(errno(libc::ENOSYS));
    asm.label(Label::Kill);
    asm.ret(libc::SECCOMP_RET_KILL_PROCESS);
    asm.finish()
}

/// The type of a socket without `SOCK_NONBLOCK` and `SOCK_CLOEXEC`.
const SOCK_TYPE: u32 = 0xf;

#[cfg(test)]
mod tests {
    use super::*;

    /// Check a filter the way the kernel's checker would and more: only the instructions
    /// the assembler makes, loads inside `seccomp_data`, every jump inside the program
    /// and forward, and no way to run off its end, so every path ends in a return.
    fn validate(filter: &[libc::sock_filter]) {
        assert!(
            !filter.is_empty() && filter.len() <= 4096,
            "{}",
            filter.len()
        );
        let len = filter.len();
        for (i, insn) in filter.iter().enumerate() {
            let code = u32::from(insn.code);
            let class = code & 0x07;
            let inside = |to: usize| assert!(to < len, "instruction {i} jumps to {to} of {len}");
            if class == libc::BPF_LD {
                assert_eq!(code, libc::BPF_LD | libc::BPF_W | libc::BPF_ABS, "at {i}");
                assert!(insn.k % 4 == 0 && insn.k < 64, "load of {} at {i}", insn.k);
                inside(i + 1);
            } else if class == libc::BPF_ALU {
                assert_eq!(code, libc::BPF_ALU | libc::BPF_AND | libc::BPF_K, "at {i}");
                inside(i + 1);
            } else if class == libc::BPF_JMP {
                let op = code & 0xf0;
                assert!(
                    [libc::BPF_JEQ, libc::BPF_JGE, libc::BPF_JSET].contains(&op)
                        && code & 0x08 == libc::BPF_K,
                    "jump {code:#x} at {i}"
                );
                inside(i + 1 + usize::from(insn.jt));
                inside(i + 1 + usize::from(insn.jf));
            } else if class == libc::BPF_RET {
                assert_eq!(code, libc::BPF_RET | libc::BPF_K, "at {i}");
            } else {
                panic!("instruction {code:#x} at {i}");
            }
        }
        assert_eq!(
            u32::from(filter[len - 1].code),
            libc::BPF_RET | libc::BPF_K,
            "the last instruction returns"
        );
    }

    /// Run a filter on one syscall as the kernel would: its return value.
    fn run(filter: &[libc::sock_filter], audit: u32, nr: u32, args: [u64; 6]) -> u32 {
        let mut data = [0u8; 64];
        data[0..4].copy_from_slice(&nr.to_le_bytes());
        data[4..8].copy_from_slice(&audit.to_le_bytes());
        for (n, value) in args.iter().enumerate() {
            data[16 + 8 * n..24 + 8 * n].copy_from_slice(&value.to_le_bytes());
        }
        let (mut acc, mut pc) = (0u32, 0usize);
        loop {
            let insn = filter[pc];
            let code = u32::from(insn.code);
            pc += 1;
            match code & 0x07 {
                libc::BPF_LD => {
                    let at = insn.k as usize;
                    acc = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                }
                libc::BPF_ALU => acc &= insn.k,
                libc::BPF_JMP => {
                    let taken = match code & 0xf0 {
                        libc::BPF_JEQ => acc == insn.k,
                        libc::BPF_JGE => acc >= insn.k,
                        libc::BPF_JSET => acc & insn.k != 0,
                        op => panic!("jump {op:#x}"),
                    };
                    pc += usize::from(if taken { insn.jt } else { insn.jf });
                }
                libc::BPF_RET => return insn.k,
                class => panic!("class {class:#x}"),
            }
        }
    }

    const ALLOW: u32 = libc::SECCOMP_RET_ALLOW;
    const KILL: u32 = libc::SECCOMP_RET_KILL_PROCESS;
    const fn errno(errno: i32) -> u32 {
        libc::SECCOMP_RET_ERRNO | errno as u32
    }
    const EACCES: u32 = errno(libc::EACCES);
    const EPERM: u32 = errno(libc::EPERM);
    const ENOSYS: u32 = errno(libc::ENOSYS);

    #[test]
    fn every_network_mode_has_a_filter_that_assembles() {
        for arch in [X86_64, AARCH64] {
            for network in [Network::None, Network::Client, Network::Server] {
                let filter = filter(network, &arch);
                validate(&filter);
                assert_eq!(filter.last().unwrap().k, KILL, "{network:?} {arch:?}");
            }
        }
    }

    /// What each filter answers, on both architectures whatever this host is.
    #[test]
    fn the_filter_refuses_what_the_sandbox_does_not_allow() {
        let (unix, inet, inet6, netlink) = (
            libc::AF_UNIX as u64,
            libc::AF_INET as u64,
            libc::AF_INET6 as u64,
            libc::AF_NETLINK as u64,
        );
        let (stream, dgram, seqpacket) = (
            libc::SOCK_STREAM as u64,
            libc::SOCK_DGRAM as u64,
            libc::SOCK_SEQPACKET as u64,
        );
        let flags = (libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK) as u64;
        let newuser = libc::CLONE_NEWUSER as u64;
        let sigchld = libc::SIGCHLD as u64;
        let thread = (libc::CLONE_VM
            | libc::CLONE_FS
            | libc::CLONE_FILES
            | libc::CLONE_SIGHAND
            | libc::CLONE_THREAD
            | libc::CLONE_SYSVSEM) as u64;
        let args = |a: u64, b: u64, c: u64| [a, b, c, 0, 0, 0];
        for arch in [X86_64, AARCH64] {
            for network in [Network::None, Network::Client, Network::Server] {
                let filter = filter(network, &arch);
                let call = |nr: u32, args: [u64; 6]| run(&filter, arch.audit, nr, args);
                let at = format!("{arch:?} {network:?}");
                // Another ABI: killed, or (x32) no such call.
                assert_eq!(run(&filter, 0x4000_0003, 0, [0; 6]), KILL, "{at}");
                if arch.x32 {
                    assert_eq!(call(0x4000_0000 | arch.socket, [0; 6]), ENOSYS, "{at}");
                }
                // An ordinary call (read is 0 on x86_64, 63 on aarch64).
                assert_eq!(call(0, [0; 6]), ALLOW, "{at}");
                assert_eq!(call(63, [0; 6]), ALLOW, "{at}");
                for nr in arch.io_uring {
                    assert_eq!(call(nr, [0; 6]), EACCES, "{at} io_uring {nr}");
                }
                for nr in arch.refused {
                    assert_eq!(call(nr, [0; 6]), EPERM, "{at} {nr}");
                }
                // Namespaces: no clone or unshare that makes one; clone3 does not exist.
                assert_eq!(call(arch.clone3, [0; 6]), ENOSYS, "{at}");
                for ns in [newuser, libc::CLONE_NEWNET as u64, libc::CLONE_NEWNS as u64] {
                    assert_eq!(call(arch.unshare, args(ns, 0, 0)), EPERM, "{at} {ns:#x}");
                    assert_eq!(call(arch.clone, args(ns | sigchld, 0, 0)), EPERM, "{at}");
                }
                assert_eq!(
                    call(arch.unshare, args(libc::CLONE_FILES as u64, 0, 0)),
                    ALLOW
                );
                assert_eq!(call(arch.clone, args(sigchld, 0, 0)), ALLOW, "{at} fork");
                assert_eq!(call(arch.clone, args(thread, 0, 0)), ALLOW, "{at} thread");
                // Unix sockets: never one of its own; a connected pair only.
                assert_eq!(call(arch.socket, args(unix, stream, 0)), EACCES, "{at}");
                assert_eq!(call(arch.socket, args(unix, dgram, 0)), EACCES, "{at}");
                for kind in [stream, seqpacket, stream | flags] {
                    assert_eq!(call(arch.socketpair, args(unix, kind, 0)), ALLOW, "{at}");
                }
                for kind in [dgram, dgram | flags, libc::SOCK_RAW as u64] {
                    assert_eq!(call(arch.socketpair, args(unix, kind, 0)), EACCES, "{at}");
                }
                assert_eq!(call(arch.socketpair, args(inet, stream, 0)), EACCES, "{at}");
                // Netlink: routes and addresses only.
                let raw = libc::SOCK_RAW as u64;
                assert_eq!(call(arch.socket, args(netlink, raw, 0)), ALLOW, "{at}");
                let diag = libc::NETLINK_SOCK_DIAG as u64;
                assert_eq!(call(arch.socket, args(netlink, raw, diag)), EACCES, "{at}");
                // The network modes.
                let (tcp, sctp) = (libc::IPPROTO_TCP as u64, libc::IPPROTO_SCTP as u64);
                let (connect, udp, listen, sctp_stream) = (
                    call(arch.socket, args(inet6, stream | flags, tcp)),
                    call(arch.socket, args(inet, dgram, 0)),
                    call(arch.listen, args(3, 1, 0)),
                    call(arch.socket, args(inet, stream, sctp)),
                );
                let expected = match network {
                    Network::None => [EACCES, EACCES, EACCES, EACCES],
                    Network::Client => [ALLOW, ALLOW, EACCES, EACCES],
                    Network::Server => [ALLOW, ALLOW, ALLOW, ALLOW],
                };
                assert_eq!([connect, udp, listen, sctp_stream], expected, "{at}");
                assert_eq!(call(arch.socket, args(40, stream, 0)), EACCES, "{at} vsock");
            }
        }
    }

    /// The tables' numbers are this host's, where the host has one.
    #[test]
    fn the_filters_syscall_numbers_are_the_hosts() {
        let Some(host) = HOST else { return };
        let nr = |n: libc::c_long| u32::try_from(n).unwrap();
        assert_eq!(host.socket, nr(libc::SYS_socket));
        assert_eq!(host.socketpair, nr(libc::SYS_socketpair));
        assert_eq!(host.listen, nr(libc::SYS_listen));
        assert_eq!(
            host.io_uring,
            [
                nr(libc::SYS_io_uring_setup),
                nr(libc::SYS_io_uring_enter),
                nr(libc::SYS_io_uring_register),
            ]
        );
        assert_eq!(
            host.refused,
            [
                nr(libc::SYS_keyctl),
                nr(libc::SYS_add_key),
                nr(libc::SYS_request_key),
                nr(libc::SYS_bpf),
                nr(libc::SYS_perf_event_open),
                nr(libc::SYS_userfaultfd),
                nr(libc::SYS_setns),
            ]
        );
        assert_eq!(host.unshare, nr(libc::SYS_unshare));
        assert_eq!(host.clone, nr(libc::SYS_clone));
        assert_eq!(host.clone3, nr(libc::SYS_clone3));
        let namespaces = libc::CLONE_NEWNS
            | libc::CLONE_NEWCGROUP
            | libc::CLONE_NEWUTS
            | libc::CLONE_NEWIPC
            | libc::CLONE_NEWUSER
            | libc::CLONE_NEWPID
            | libc::CLONE_NEWNET;
        assert_eq!(NAMESPACES, namespaces as u32);
    }
}
