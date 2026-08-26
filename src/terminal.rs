//! Putting the terminal into the mode the TUI needs.
//!
//! Raw mode and the alternate screen are state of the *terminal emulator*, not
//! of this process. Nothing undoes them when GitLoom exits, so any path that
//! skips the teardown hands the user back a shell with no echo and no line
//! editing: their next command is invisible as they type it. That damage
//! outlives the program, which is why the teardown lives in a [`Drop`] guard
//! and a panic hook instead of at the end of the happy path.
//!
//! Nothing here is unit-tested, deliberately. Every function's effect is on a
//! real tty: `enable_raw_mode` fails outright without one, so a test could only
//! assert that it failed, and [`install_panic_hook`] mutates process-global
//! state that the other tests in the binary share.

use crossterm::{
    cursor::Show,
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::io;

/// Owns the terminal's raw mode and alternate screen for as long as it lives.
///
/// `main` holds one for the lifetime of the TUI. Because the teardown is in
/// [`Drop`] rather than at the end of a function, it also runs on the paths that
/// used to be missed: a `?` returning early, and a panic unwinding.
pub struct TerminalGuard;

impl TerminalGuard {
    /// Enters raw mode and the alternate screen.
    ///
    /// This is the only way to construct the guard, so "entered the alternate
    /// screen without arranging to leave it" is not a state the caller can
    /// express.
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;

        // Constructed before the next fallible step, not after: if entering the
        // alternate screen fails, the `?` below drops `guard` on its way out and
        // raw mode is switched back off. Returning `Ok(Self)` at the end instead
        // would leave a failure here with raw mode still on and no guard in
        // existence to undo it.
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if let Err(e) = restore() {
            // `drop` cannot report failure, and there is no recovery to attempt;
            // the terminal is already in a state this code could not fix. Say
            // so rather than exiting silently, unless a panic is already
            // unwinding, whose message is the more useful of the two.
            if !std::thread::panicking() {
                eprintln!("gitloom: could not restore the terminal: {e}");
            }
        }
    }
}

/// Undoes [`TerminalGuard::enter`], best effort.
///
/// Every step is attempted even when an earlier one fails, and the first error
/// is returned afterwards. Leaving raw mode while the alternate screen stays
/// stuck is a far better outcome for the user than doing neither, which is what
/// a chain of `?`s would have produced.
///
/// Safe to call when there is nothing to undo: `disable_raw_mode` is a no-op
/// unless crossterm has a saved mode to put back, and both escape sequences are
/// ignored by a terminal that is already in that state. That is what makes it
/// safe to run from the panic hook and the guard both.
fn restore() -> io::Result<()> {
    let raw = disable_raw_mode();
    // `Show` because ratatui hides the cursor while it draws; without it the
    // shell prompt comes back with an invisible caret.
    let screen = execute!(io::stdout(), LeaveAlternateScreen, Show);
    raw.and(screen)
}

/// Restores the terminal *before* a panic message is printed.
///
/// Must be called from the thread that owns the terminal, because it records
/// that thread's id to recognise it later.
///
/// Without this the default hook writes into the alternate screen while raw mode
/// is still on, so the message staircases down the screen and then vanishes with
/// the alternate screen a moment later. The user is left with a broken shell and
/// no idea why, like actually :( and Restoring first puts the message on the normal screen, where it
/// is still there after GitLoom is gone.
///
/// The default hook is kept and called rather than replaced, so `RUST_BACKTRACE`
/// and the standard message format keep working.
pub fn install_panic_hook() {
    let terminal_owner = std::thread::current().id();
    let default_hook = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        // A panic on a worker thread kills that thread and nothing else: the TUI
        // carries on drawing. Restoring the terminal from under it would turn a
        // background failure into a corrupted screen, so only the owning thread
        // whose panic really is the end of the process tears it down.
        if std::thread::current().id() == terminal_owner {
            // Already panicking, so a failure here has nowhere useful to go, and
            // the message the default hook is about to print matters more.
            let _ = restore();
        }
        default_hook(info);
    }));
}
