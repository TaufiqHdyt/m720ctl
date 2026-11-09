//! Accessibility / Input Monitoring checks.
//!
//! Asking once makes macOS add the binary to both lists in System Settings
//! (switched off), so the user only has to flip the toggles.

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

/// Is the process trusted for Accessibility? With `prompt`, macOS shows its
/// "would like to control this computer" dialog / registers the binary.
pub fn accessibility(prompt: bool) -> bool {
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let opts = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::from(prompt))]);
        AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef())
    }
}

/// Does the process have Input Monitoring? With `prompt`, ask for it.
pub fn input_monitoring(prompt: bool) -> bool {
    unsafe {
        if prompt {
            CGRequestListenEventAccess()
        } else {
            CGPreflightListenEventAccess()
        }
    }
}

pub fn help() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "m720ctl".into());
    format!(
        "Grant access in System Settings -> Privacy & Security:\n  \
         * Accessibility     (needed to modify/swallow events)\n  \
         * Input Monitoring  (needed to read mouse input)\n\
         Binary: {exe}  (add it with '+' if it isn't listed).\n\
         When run from a terminal, the terminal app needs these permissions instead."
    )
}
