//! The pane owns the original argument/environment string area before it starts threads.
use std::ffi::{CStr, CString, OsString};
use std::os::unix::ffi::OsStrExt;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod startup {
    use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
    pub static ARGV: AtomicPtr<*mut libc::c_char> = AtomicPtr::new(std::ptr::null_mut());
    pub static ARGC: AtomicUsize = AtomicUsize::new(0);
    // The supported Linux targets use glibc, which passes these to ELF constructors.
    #[used]
    #[unsafe(link_section = ".init_array.00099")]
    static CAPTURE: unsafe extern "C" fn(libc::c_int, *mut *mut libc::c_char) = capture;
    unsafe extern "C" fn capture(argc: libc::c_int, argv: *mut *mut libc::c_char) {
        ARGC.store(argc as usize, Ordering::Relaxed);
        ARGV.store(argv, Ordering::Relaxed);
    }
}

pub struct Title {
    area: &'static mut [u8],
    prefix: Vec<u8>,
    last: Vec<u8>,
}

impl Title {
    /// Called only by pane::run, before its runtime or any threads exist.
    pub fn new(args: &[OsString]) -> Self {
        let prefix = args
            .iter()
            .map(|a| a.as_bytes())
            .collect::<Vec<_>>()
            .join(&b' ');
        // SAFETY: this process is single threaded here. No C environment pointers
        // are retained by callers; owned Rust environment values remain valid.
        let area = unsafe { move_environment() };
        if area.len() <= prefix.len() + 2 {
            tracing::warn!("pane title: no room");
            return Self {
                area: &mut [],
                prefix,
                last: Vec::new(),
            };
        }
        Self {
            area,
            prefix,
            last: Vec::new(),
        }
    }

    pub fn set(&mut self, pid: libc::pid_t, cwd: &str, command: Option<&[u8]>) {
        if self.area.is_empty() {
            return;
        }
        let mut title = self.prefix.clone();
        title.extend_from_slice(format!(" [shell {pid} in {cwd}").as_bytes());
        if let Some(command) = command {
            title.extend_from_slice(b": ");
            title.extend_from_slice(command);
        }
        title.push(b']');
        if title != self.last {
            write_area(self.area, &title);
            self.last = title;
        }
    }
}

fn write_area(area: &mut [u8], title: &[u8]) {
    area.fill(0);
    let len = title.len().min(area.len().saturating_sub(1));
    area[..len].copy_from_slice(&title[..len]);
}

unsafe fn move_environment() -> &'static mut [u8] {
    #[cfg(target_os = "linux")]
    debug_assert_eq!(
        std::fs::read_dir("/proc/self/task")
            .map(|tasks| tasks.count())
            .unwrap_or(1),
        1
    );
    #[cfg(target_os = "linux")]
    unsafe extern "C" {
        static mut environ: *mut *mut libc::c_char;
    }
    #[cfg(target_os = "macos")]
    // SAFETY: the C runtime owns these initialized pointers for the process lifetime.
    let (argc, argv) = unsafe { (*libc::_NSGetArgc() as usize, *libc::_NSGetArgv()) };
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    let (argc, argv) = (
        startup::ARGC.load(std::sync::atomic::Ordering::Relaxed),
        startup::ARGV.load(std::sync::atomic::Ordering::Relaxed),
    );
    #[cfg(all(target_os = "linux", not(target_env = "gnu")))]
    let (argc, argv) = (0usize, std::ptr::null_mut::<*mut libc::c_char>());
    if argc == 0 || argv.is_null() {
        return &mut [];
    }
    // SAFETY: argv contains argc non-null, NUL-terminated strings. Only the
    // contiguous prefix is reused. Copies and the new environ array are leaked
    // deliberately: libc may read them until exec/exit, including during drop.
    unsafe {
        #[cfg(target_os = "macos")]
        let environment = libc::_NSGetEnviron();
        #[cfg(target_os = "linux")]
        let environment = &raw mut environ;
        move_strings(argc, argv, environment)
    }
}

// SAFETY: argv/environ describe writable C runtime arrays and NUL-terminated
// strings; no concurrent reader or retained borrowed environment string exists.
unsafe fn move_strings(
    argc: usize,
    argv: *mut *mut libc::c_char,
    environ: *mut *mut *mut libc::c_char,
) -> &'static mut [u8] {
    unsafe {
        let start = *argv;
        let mut end = start;
        for i in 0..argc {
            let p = *argv.add(i);
            if p != end {
                break;
            }
            end = p.add(CStr::from_ptr(p).to_bytes_with_nul().len());
        }
        let mut copies = Vec::new();
        let mut env = *environ;
        // Extend only through the contiguous prefix. A setenv replacement may
        // live on the heap; it must not discard space before it or bridge a gap.
        while !(*env).is_null() && *env == end {
            end = end.add(CStr::from_ptr(*env).to_bytes_with_nul().len());
            env = env.add(1);
        }
        env = *environ;
        while !(*env).is_null() {
            let p = *env;
            let value = CStr::from_ptr(p);
            copies.push(CString::from(value).into_raw());
            env = env.add(1);
        }
        copies.push(std::ptr::null_mut());
        *environ = Box::leak(copies.into_boxed_slice()).as_mut_ptr();
        let len = end as usize - start as usize;
        // Runtime argument readers keep seeing the start arguments as well.
        // The kernel's ps interface reads the original string area, not argv.
        for i in 0..argc {
            *argv.add(i) = CString::from(CStr::from_ptr(*argv.add(i))).into_raw();
        }
        std::slice::from_raw_parts_mut(start.cast(), len)
    }
}

pub fn command_line(pid: libc::pid_t) -> Option<Vec<u8>> {
    let raw = process_arguments(pid)?;
    let mut command = raw
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .collect::<Vec<_>>()
        .join(&b' ');
    command.truncate(80);
    Some(command)
}

#[cfg(target_os = "linux")]
fn process_arguments(pid: libc::pid_t) -> Option<Vec<u8>> {
    std::fs::read(format!("/proc/{pid}/cmdline")).ok()
}

#[cfg(target_os = "macos")]
fn process_arguments(pid: libc::pid_t) -> Option<Vec<u8>> {
    let mut max: libc::c_int = 0;
    let mut size = std::mem::size_of_val(&max);
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    // SAFETY: the output pointer and size describe max.
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&raw mut max).cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || max <= 0
    {
        return None;
    }
    let mut buf = vec![0u8; max as usize];
    size = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    // SAFETY: sysctl receives the allocated buffer's exact capacity.
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    buf.truncate(size);
    mac_arguments(&buf)
}

#[cfg(any(target_os = "macos", test))]
fn mac_arguments(buf: &[u8]) -> Option<Vec<u8>> {
    let count = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    let mut rest = buf.get(4..)?;
    rest = rest.get(rest.iter().position(|b| *b == 0)?..)?;
    rest = rest.get(rest.iter().position(|b| *b != 0)?..)?;
    let mut args = Vec::new();
    for _ in 0..count {
        let end = rest.iter().position(|b| *b == 0)?;
        args.extend_from_slice(&rest[..=end]);
        rest = &rest[end + 1..];
    }
    Some(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_title_is_cut_to_the_area() {
        let mut area = [42; 8];
        write_area(&mut area, b"a longer title");
        assert_eq!(&area, b"a longe\0");
        write_area(&mut area, b"short");
        assert_eq!(&area, b"short\0\0\0");
    }
    #[test]
    fn the_environment_is_unchanged_after_the_move() {
        let bytes = Box::leak(
            b"tilrd\0pane\0HOME=/tmp\0TILR=value\0"
                .to_vec()
                .into_boxed_slice(),
        );
        let p = bytes.as_mut_ptr().cast::<libc::c_char>();
        // SAFETY: all pointers below point to NUL-terminated strings in bytes.
        unsafe {
            let mut argv = [p, p.add(6), std::ptr::null_mut()];
            let mut env = [p.add(11), p.add(21), std::ptr::null_mut()];
            let mut environ = env.as_mut_ptr();
            let area = move_strings(2, argv.as_mut_ptr(), &raw mut environ);
            write_area(area, b"a title replacing the original environment");
            assert_eq!(CStr::from_ptr(*environ).to_bytes(), b"HOME=/tmp");
            assert_eq!(CStr::from_ptr(*environ.add(1)).to_bytes(), b"TILR=value");
            assert!((*environ.add(2)).is_null());
            assert_eq!(CStr::from_ptr(argv[0]).to_bytes(), b"tilrd");
            assert_eq!(CStr::from_ptr(argv[1]).to_bytes(), b"pane");
        }
    }
    #[test]
    fn a_moved_environment_entry_keeps_the_contiguous_prefix() {
        let bytes = Box::leak(
            b"tilrd\0pane\0HOME=/tmp\0LAST=kept\0"
                .to_vec()
                .into_boxed_slice(),
        );
        let p = bytes.as_mut_ptr().cast::<libc::c_char>();
        let moved = CString::new("TILR=moved").unwrap();
        // SAFETY: these arrays contain writable, NUL-terminated strings and
        // remain alive throughout move_strings; only the returned area is reused.
        unsafe {
            let mut argv = [p, p.add(6), std::ptr::null_mut()];
            // The final entry still starts at the end of HOME. Once a gap is
            // seen, extension must stop even if a later pointer matches end.
            let mut env = [
                p.add(11),
                moved.as_ptr().cast_mut(),
                p.add(21),
                std::ptr::null_mut(),
            ];
            let mut environ = env.as_mut_ptr();
            let area = move_strings(2, argv.as_mut_ptr(), &raw mut environ);
            assert_eq!(area.len(), 21, "argv plus HOME, up to the first gap");
            write_area(area, b"a title overwrites HOME");
            assert_eq!(CStr::from_ptr(*environ).to_bytes(), b"HOME=/tmp");
            assert_eq!(CStr::from_ptr(*environ.add(1)).to_bytes(), b"TILR=moved");
            assert_eq!(CStr::from_ptr(*environ.add(2)).to_bytes(), b"LAST=kept");
            assert!((*environ.add(3)).is_null());
            assert_eq!(CStr::from_ptr(argv[0]).to_bytes(), b"tilrd");
            assert_eq!(CStr::from_ptr(argv[1]).to_bytes(), b"pane");
            assert_eq!(CStr::from_ptr(p.add(21)).to_bytes(), b"LAST=kept");
        }
    }
    #[test]
    fn mac_arguments_exclude_the_executable_path_and_environment() {
        let mut buf = 2i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/bin/sleep\0\0sleep\0");
        buf.extend_from_slice(b"30\0SECRET=value\0");
        assert_eq!(
            mac_arguments(&buf).unwrap(),
            b"sleep\0"
                .iter()
                .chain(b"30\0")
                .copied()
                .collect::<Vec<_>>()
        );
    }
}
