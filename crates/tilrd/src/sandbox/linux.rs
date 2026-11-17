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
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use landlock::{
    ABI, Access, AccessFs, AccessNet, BitFlags, CompatLevel, Compatible, PathBeneath, Ruleset,
    RulesetAttr, RulesetCreated, RulesetCreatedAttr, Scope,
};

use super::{Access as Grantee, Features, Grant, Network, Platform};

pub struct Linux;

/// The restriction of one pane, built before the fork.
pub struct Prepared {
    /// The Landlock ruleset (close-on-exec: it does not reach the program).
    ruleset: OwnedFd,
    filter: Vec<libc::sock_filter>,
}

/// The `AUDIT_ARCH_*` of the architecture this build runs on: a filter checks it first,
/// so a syscall made through another ABI (32-bit, x32) cannot slip past it.
#[cfg(all(target_arch = "x86_64", target_endian = "little"))]
const AUDIT_ARCH: Option<u32> = Some(0xC000_003E);
#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
const AUDIT_ARCH: Option<u32> = Some(0xC000_00B7);
#[cfg(not(any(
    all(target_arch = "x86_64", target_endian = "little"),
    all(target_arch = "aarch64", target_endian = "little")
)))]
const AUDIT_ARCH: Option<u32> = None;

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
        if AUDIT_ARCH.is_none() {
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

    fn prepare(grants: &[Grant], network: Network, features: Features) -> Result<Prepared, String> {
        let abi = ABI::from(i32::try_from(features.landlock_abi).unwrap_or(i32::MAX));
        let ruleset = ruleset(grants, network, abi)?;
        let ruleset: Option<OwnedFd> = ruleset.into();
        let ruleset = ruleset.ok_or("Landlock: the ruleset has no descriptor")?;
        let filter = filter(
            network,
            AUDIT_ARCH.ok_or("no seccomp filter for this architecture")?,
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
fn ruleset(grants: &[Grant], network: Network, abi: ABI) -> Result<RulesetCreated, String> {
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
    let mut rules: Vec<(&Path, BitFlags<AccessFs>, bool)> = Vec::new();
    for path in SYSTEM_PROGRAMS {
        rules.push((Path::new(path), read_exec, false));
    }
    for path in SYSTEM_READ {
        rules.push((Path::new(path), read, false));
    }
    for path in DEVICES {
        rules.push((Path::new(path), device, false));
    }
    // /etc/resolv.conf is often a link out of /etc (systemd-resolved's /run/...):
    // what a client needs to look names up.
    let resolv = std::fs::canonicalize("/etc/resolv.conf").ok();
    if network != Network::None
        && let Some(resolv) = &resolv
        && !resolv.starts_with("/etc")
    {
        rules.push((resolv, read, false));
    }
    for grant in grants {
        let access = match grant.access {
            Grantee::ReadOnly => read_exec,
            Grantee::ReadWrite => all,
            Grantee::Program => all & (AccessFs::ReadFile | AccessFs::Execute),
            Grantee::Terminal => device,
        };
        rules.push((&grant.path, access, true));
    }
    for (path, access, asked) in rules {
        // O_PATH: a handle on the place, opened neither for reading nor writing.
        let file = match File::options()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
            .open(path)
        {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                if asked {
                    tracing::info!("sandbox: {} does not exist, not granted", path.display());
                }
                continue;
            }
            Err(err) => return Err(format!("open {}: {err}", path.display())),
        };
        let is_dir = file.metadata().is_ok_and(|meta| meta.is_dir());
        let access = if is_dir {
            access
        } else {
            access & AccessFs::from_file(abi)
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
    Inet,
    Stream,
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

/// The seccomp filter of a pane (S5): `socket(AF_UNIX)` refused always (Landlock does
/// not govern `connect` to a named Unix socket, and the server's sockets accept any
/// process of the user); with `none` every socket but netlink, with `client` all but
/// TCP streams and datagrams of IPv4 and IPv6, with both `listen` (which would bind an
/// unbound TCP socket to a free port behind Landlock's back); io_uring always (its
/// socket and connect operations bypass the filter). Everything else is allowed.
fn filter(network: Network, audit_arch: u32) -> Vec<libc::sock_filter> {
    let deny = libc::SECCOMP_RET_ERRNO | (libc::EACCES as u32 & libc::SECCOMP_RET_DATA);
    let nosys = libc::SECCOMP_RET_ERRNO | (libc::ENOSYS as u32 & libc::SECCOMP_RET_DATA);
    let nr = |n: libc::c_long| u32::try_from(n).expect("syscall numbers are small");
    let mut asm = Asm::new();
    asm.load(ARCH_OFFSET);
    asm.jump(libc::BPF_JEQ, audit_arch, Label::Next, Label::Kill);
    asm.load(NR_OFFSET);
    if cfg!(target_arch = "x86_64") {
        // The x32 ABI: same AUDIT_ARCH, syscall numbers with bit 30 set.
        asm.jump(libc::BPF_JGE, 0x4000_0000, Label::NoSys, Label::Next);
    }
    for syscall in [
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
    ] {
        asm.if_eq(nr(syscall), Label::Deny);
    }
    if network != Network::Server {
        asm.if_eq(nr(libc::SYS_listen), Label::Deny);
    }
    asm.jump(
        libc::BPF_JEQ,
        nr(libc::SYS_socket),
        Label::Next,
        Label::Allow,
    );
    asm.load(arg(0));
    asm.if_eq(libc::AF_UNIX as u32, Label::Deny);
    asm.if_eq(libc::AF_NETLINK as u32, Label::Allow);
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
            // The type without SOCK_NONBLOCK and SOCK_CLOEXEC.
            asm.stmt(libc::BPF_ALU | libc::BPF_AND | libc::BPF_K, 0xf);
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
    asm.label(Label::Allow);
    asm.ret(libc::SECCOMP_RET_ALLOW);
    asm.label(Label::Deny);
    asm.ret(deny);
    asm.label(Label::NoSys);
    asm.ret(nosys);
    asm.label(Label::Kill);
    asm.ret(libc::SECCOMP_RET_KILL_PROCESS);
    asm.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_network_mode_has_a_filter_that_assembles() {
        for network in [Network::None, Network::Client, Network::Server] {
            let filter = filter(network, 0xC000_003E);
            assert!(
                filter.len() < 64,
                "{network:?}: {} instructions",
                filter.len()
            );
            let last = filter.last().unwrap();
            assert_eq!(last.k, libc::SECCOMP_RET_KILL_PROCESS);
        }
    }
}
