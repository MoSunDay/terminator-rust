//! macOS backend for [`crate::procfs`]: no /proc here, the equivalent is
//! libproc (all symbols re-exported by libc's apple module). pid table =
//! proc_listallpids + PROC_PIDT_SHORTBSDINFO, fd table =
//! PROC_PIDLISTFDS + per-fd vnode paths, cwd = PROC_PIDVNODEPATHINFO.
//! Forgiving like the Linux side: other users' processes fail with EPERM
//! and are skipped; the libproc convention is that a fill call returning
//! the full buffer size means success, anything else (0/-1) means skip.

use std::ffi::CStr;
use std::path::PathBuf;

use crate::procfs::{pick_db, ProcRow};

/// fd flavor of `proc_pidfdinfo` filling a `proc_vnodepathinfo`
/// (sys/proc_info.h fd-flavor enum) — the one constant libc does not
/// export.
const PROC_PIDFDVNODEPATHINFO: libc::c_int = 2;

/// Every process this uid may inspect as one row, pid order. Processes
/// owned by other users fail `proc_pidinfo` with EPERM and drop out.
pub fn proc_table() -> Vec<ProcRow> {
    list_all_pids()
        .into_iter()
        .filter_map(|pid| {
            let mut info: libc::proc_bsdshortinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of::<libc::proc_bsdshortinfo>() as libc::c_int;
            let filled = unsafe {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDT_SHORTBSDINFO,
                    0,
                    &mut info as *mut _ as *mut libc::c_void,
                    size,
                )
            };
            (filled == size).then(|| ProcRow {
                pid: info.pbsi_pid as i32,
                ppid: info.pbsi_ppid as i32,
                comm: cstr(info.pbsi_comm.as_ptr()),
            })
        })
        .collect()
}

/// Vnode paths of every fd whose path ends with "opencoder.db", handed
/// to the shared pick_db for the single-store rule.
pub fn find_db(pid: i32) -> Result<Option<PathBuf>, String> {
    let links: Vec<PathBuf> = list_fds(pid)
        .into_iter()
        .filter(|fd| fd.proc_fdtype == libc::PROX_FDTYPE_VNODE as u32)
        .filter_map(|fd| vnode_path(pid, fd.proc_fd))
        .filter(|p| p.to_string_lossy().ends_with("opencoder.db"))
        .collect();
    pick_db(links)
}

/// Working directory of `pid` via the vnode path info (empty PathBuf
/// when the call fails — e.g. the process exited mid-scan).
pub fn cwd_of(pid: i32) -> PathBuf {
    let mut pvi: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    let filled = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            &mut pvi as *mut _ as *mut libc::c_void,
            size,
        )
    };
    if filled != size {
        return PathBuf::new();
    }
    PathBuf::from(cstr(pvi.pvi_cdir.vip_path.as_ptr().cast::<libc::c_char>()))
}

/// All pids on the system. The sizing call (NULL buffer) reports the
/// needed BYTES; the fill call reports how many pids it wrote. Any 0/-1
/// return means "no table" (empty Vec).
fn list_all_pids() -> Vec<i32> {
    let bytes = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    let Some(cap) = positive_capacity(bytes) else {
        return Vec::new();
    };
    let mut buf: Vec<i32> = vec![0; cap];
    let filled = unsafe { libc::proc_listallpids(buf.as_mut_ptr().cast::<libc::c_void>(), bytes) };
    if filled <= 0 {
        return Vec::new();
    }
    buf.truncate(filled as usize);
    buf
}

/// The fd table of `pid`; each entry carries the fd number and its type.
fn list_fds(pid: i32) -> Vec<libc::proc_fdinfo> {
    let bytes =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    let elem = std::mem::size_of::<libc::proc_fdinfo>();
    let Some(cap) = positive_capacity(bytes).map(|n| n / elem) else {
        return Vec::new();
    };
    let mut buf: Vec<libc::proc_fdinfo> = vec![
        libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0,
        };
        cap
    ];
    let filled = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            buf.as_mut_ptr().cast::<libc::c_void>(),
            bytes,
        )
    };
    if filled <= 0 {
        return Vec::new();
    }
    buf.truncate(filled as usize / elem);
    buf
}

/// Path behind one vnode fd, None when the query comes back short.
fn vnode_path(pid: i32, fd: i32) -> Option<PathBuf> {
    let mut vpi: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    let filled = unsafe {
        libc::proc_pidfdinfo(
            pid,
            fd,
            PROC_PIDFDVNODEPATHINFO,
            &mut vpi as *mut _ as *mut libc::c_void,
            size,
        )
    };
    (filled == size)
        .then(|| PathBuf::from(cstr(vpi.pvi_cdir.vip_path.as_ptr().cast::<libc::c_char>())))
}

/// NUL-terminated C string field (kernel-strlcpy'd, so always terminated)
/// as an owned String, lossy for non-UTF-8 names.
fn cstr(field: *const libc::c_char) -> String {
    if field.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(field) }
        .to_string_lossy()
        .into_owned()
}

/// i32 element capacity behind a libproc byte count; None when the call
/// failed (0/-1) or the buffer would be empty.
fn positive_capacity(bytes: libc::c_int) -> Option<usize> {
    let cap = (bytes as usize) / std::mem::size_of::<i32>();
    (bytes > 0 && cap > 0).then_some(cap)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn proc_table_includes_self() {
        let table = proc_table();
        let me = std::process::id() as i32;
        assert!(
            table.iter().any(|row| row.pid == me),
            "self pid {me} missing from {} rows",
            table.len()
        );
        let row = table
            .iter()
            .find(|row| row.pid == me)
            .expect("self row just checked");
        // the test binary's parent is the test runner / cargo, never pid 0
        assert!(row.ppid > 0, "self row: {row:?}");
        assert!(!row.comm.is_empty(), "self comm: {row:?}");
    }

    #[test]
    fn descendants_and_cwd_reject_or_resolve_self() {
        let me = std::process::id() as i32;
        let tree = crate::procfs::descendants(me);
        assert_eq!(tree.first(), Some(&me), "root included: {tree:?}");
        // no opencoder under the test binary
        assert_eq!(crate::procfs::find_opencoder(me), Ok(None));
        // our own cwd is readable and non-root-relative
        let cwd = cwd_of(me);
        assert!(cwd.is_absolute(), "cwd: {}", cwd.display());
    }
}
