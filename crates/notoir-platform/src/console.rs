/// Attach CLI-only invocations to their caller's terminal. Never creates a
/// console; inherited redirected handles (including MCP pipes) stay untouched.
pub fn attach_parent_console() {
    #[cfg(windows)]
    {
        use win32console::console::{HandleType, WinConsole};
        // AttachConsole may replace the standard handles. Retain every
        // explicitly inherited handle and fill only previously missing ones.
        let handles = [HandleType::Input, HandleType::Output, HandleType::Error].map(|kind| {
            (
                kind,
                WinConsole::get_std_handle(kind)
                    .ok()
                    .filter(|h| !h.get_raw().is_null()),
            )
        });
        if WinConsole::attach_console(u32::MAX).is_ok() {
            for (kind, handle) in handles {
                if let Some(handle) = handle {
                    let _ = WinConsole::set_std_handle(kind, handle);
                }
            }
        }
    }
}
