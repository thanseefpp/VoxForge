//! VoxForge — Focus management.
//! Continuously tracks the frontmost non-VoxForge app so we can restore focus after recording.

use std::sync::atomic::{AtomicI32, Ordering};

static LAST_EXTERNAL_PID: AtomicI32 = AtomicI32::new(-1);
static OUR_PID: AtomicI32 = AtomicI32::new(-1);

/// Start background thread polling frontmost app every 300ms.
pub fn start_focus_tracker() {
    OUR_PID.store(std::process::id() as i32, Ordering::SeqCst);
    std::thread::spawn(|| loop {
        poll_frontmost();
        std::thread::sleep(std::time::Duration::from_millis(300));
    });
    eprintln!("[VoxForge] Focus tracker started (PID: {})", std::process::id());
}

#[cfg(target_os = "macos")]
fn poll_frontmost() {
    use cocoa::base::nil;
    use objc::{msg_send, sel, sel_impl, runtime::Object};
    unsafe {
        let ws: *mut Object = msg_send![objc::class!(NSWorkspace), sharedWorkspace];
        let app: *mut Object = msg_send![ws, frontmostApplication];
        if app == nil { return; }
        let pid: i32 = msg_send![app, processIdentifier];
        if pid > 0 && pid != OUR_PID.load(Ordering::SeqCst) {
            LAST_EXTERNAL_PID.store(pid, Ordering::SeqCst);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn poll_frontmost() {}

/// Activate the last known external app.
#[cfg(target_os = "macos")]
pub fn restore_focus() {
    use cocoa::base::nil;
    use objc::{msg_send, sel, sel_impl, runtime::Object};
    let pid = LAST_EXTERNAL_PID.load(Ordering::SeqCst);
    if pid <= 0 { return; }
    unsafe {
        let app: *mut Object = msg_send![
            objc::class!(NSRunningApplication),
            runningApplicationWithProcessIdentifier: pid
        ];
        if app == nil { return; }
        let _: bool = msg_send![app, activateWithOptions: 2u64];
        eprintln!("[VoxForge] Restored focus to PID {}", pid);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn restore_focus() {}
