//! VoxForge — Paste/type module.
//! Copies text to clipboard and simulates Cmd+V to paste into the active app.

use arboard::Clipboard;

/// Copy text to clipboard, then simulate Cmd+V.
pub fn paste_text(text: &str) -> Result<(), String> {
    // Step 1: Copy to clipboard
    let mut clipboard = Clipboard::new().map_err(|e| format!("Clipboard error: {}", e))?;
    clipboard
        .set_text(text)
        .map_err(|e| format!("Failed to set clipboard: {}", e))?;
    eprintln!("[VoxForge] Copied {} chars to clipboard", text.len());

    // Step 2: Small delay for clipboard to settle
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Step 3: Simulate Cmd+V
    simulate_paste()?;
    eprintln!("[VoxForge] Paste simulated");

    Ok(())
}

/// Simulate Cmd+V using CGEvent on macOS.
#[cfg(target_os = "macos")]
fn simulate_paste() -> Result<(), String> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGKeyCode};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| "Failed to create event source".to_string())?;

    // 'v' key = keycode 9
    let key_down = CGEvent::new_keyboard_event(source.clone(), 9 as CGKeyCode, true)
        .map_err(|_| "Failed to create key down event".to_string())?;
    let key_up = CGEvent::new_keyboard_event(source, 9 as CGKeyCode, false)
        .map_err(|_| "Failed to create key up event".to_string())?;

    // Add Cmd modifier
    key_down.set_flags(CGEventFlags::CGEventFlagCommand);
    key_up.set_flags(CGEventFlags::CGEventFlagCommand);

    key_down.post(core_graphics::event::CGEventTapLocation::HID);
    key_up.post(core_graphics::event::CGEventTapLocation::HID);

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn simulate_paste() -> Result<(), String> {
    Err("Paste simulation only supported on macOS".to_string())
}

/// Check macOS accessibility permission with prompt.
#[cfg(target_os = "macos")]
pub fn check_accessibility() -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrustedWithOptions(options: core_foundation::base::CFTypeRef) -> bool;
    }

    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::true_value();
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);

    let trusted = unsafe { AXIsProcessTrustedWithOptions(options.as_CFTypeRef()) };
    eprintln!("[VoxForge] Accessibility: {}", trusted);
    trusted
}

#[cfg(not(target_os = "macos"))]
pub fn check_accessibility() -> bool {
    true
}
