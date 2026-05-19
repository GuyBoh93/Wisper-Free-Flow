// Process priority management. We raise the whole process to
// ABOVE_NORMAL while transcribing so whisper.cpp's worker threads (which
// we don't directly control) all get scheduled ahead of background apps,
// then drop back to NORMAL when idle. Boosts cold-machine inference by a
// few percent and helps a lot when something else is eating CPU.
//
// Process priority — not thread priority — because whisper.cpp spawns
// its own thread pool internally; bumping just our spawning thread would
// miss the workers that actually do the math.

#[cfg(target_os = "windows")]
pub fn boost() {
    use windows_sys::Win32::System::Threading::{
        ABOVE_NORMAL_PRIORITY_CLASS, GetCurrentProcess, SetPriorityClass,
    };
    unsafe {
        if SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS) == 0 {
            tracing::debug!("SetPriorityClass(ABOVE_NORMAL) failed");
        }
    }
}

#[cfg(target_os = "windows")]
pub fn restore() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, NORMAL_PRIORITY_CLASS, SetPriorityClass,
    };
    unsafe {
        if SetPriorityClass(GetCurrentProcess(), NORMAL_PRIORITY_CLASS) == 0 {
            tracing::debug!("SetPriorityClass(NORMAL) failed");
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn boost() {}

#[cfg(not(target_os = "windows"))]
pub fn restore() {}
