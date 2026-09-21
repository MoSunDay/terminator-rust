//! Global X11 pointer poll, the ground truth for a live edge-resize.
//!
//! While we drive a resize with per-frame `ViewportCommand`s, the WM
//! reconfigures the window under us (openbox does this for every
//! XMoveWindow of a west/north drag) and that reconfiguration can break
//! the core pointer grab. Motion and ButtonRelease then reroute to the
//! root window: egui's `latest_pos` freezes at the press, the release
//! is never seen, and a gesture armed on events alone would linger or
//! wedge the next one. `XQueryPointer` answers both questions straight
//! from the server - global position and live button mask - with no
//! event delivery involved.

/// Poll result in screen pixels plus the primary-button state.
pub struct Polled {
    pub x: f32,
    pub y: f32,
    pub primary_down: bool,
}

/// Current global pointer, or `None` when X11 is unavailable (headless
/// tests, Wayland-only hosts): callers fall back to event-driven input.
pub fn poll() -> Option<Polled> {
    query_x11()
}

#[cfg(target_os = "linux")]
fn query_x11() -> Option<Polled> {
    use std::cell::RefCell;
    use x11_dl::xlib::{Display, Xlib};

    enum Conn {
        Idle,
        Dead,
        Live(Box<Xlib>, *mut Display),
    }
    // SAFETY: the connection lives on this thread only and Xlib is a
    // table of function pointers with no interior mutability.
    unsafe impl Send for Conn {}

    thread_local! {
        static CONN: RefCell<Conn> = const { RefCell::new(Conn::Idle) };
    }

    const BUTTON1_MASK: u32 = 1 << 8;

    CONN.with(|cell| {
        let mut conn = cell.borrow_mut();
        if let Conn::Idle = &*conn {
            *conn = match Xlib::open() {
                Ok(x) => {
                    // Leak the display: one lazy connection per thread
                    // for the life of the process.
                    let dpy = unsafe { (x.XOpenDisplay)(std::ptr::null()) };
                    if dpy.is_null() {
                        Conn::Dead
                    } else {
                        Conn::Live(Box::new(x), dpy)
                    }
                }
                Err(_) => Conn::Dead,
            };
        }
        let Conn::Live(x, dpy) = &*conn else {
            return None;
        };
        // SAFETY: dpy comes from our own XOpenDisplay and outlives the
        // query; root is a server-side constant for the default screen.
        unsafe {
            let screen = (x.XDefaultScreen)(*dpy);
            let root = (x.XRootWindow)(*dpy, screen);
            if root == 0 {
                return None;
            }
            let mut r_root = 0u64;
            let mut r_child = 0u64;
            let mut rx = 0i32;
            let mut ry = 0i32;
            let mut wx = 0i32;
            let mut wy = 0i32;
            let mut mask = 0u32;
            let ok = (x.XQueryPointer)(
                *dpy,
                root,
                &mut r_root,
                &mut r_child,
                &mut rx,
                &mut ry,
                &mut wx,
                &mut wy,
                &mut mask,
            );
            if ok == 0 {
                return None;
            }
            Some(Polled {
                x: rx as f32,
                y: ry as f32,
                primary_down: mask & BUTTON1_MASK != 0,
            })
        }
    })
}

#[cfg(not(target_os = "linux"))]
fn query_x11() -> Option<Polled> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_is_callable_anywhere() {
        // Must not panic with or without a display; on CI there is an
        // Xvfb, locally there may not be.
        let _ = poll();
    }
}
