use std::io;

use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};

/// Enters raw mode + alt-screen on construction, restores the terminal on
/// drop (covers normal return, `?` early-return, and panics via the panic
/// hook installed in `enter`).
pub struct AltScreenGuard;

impl AltScreenGuard {
    pub fn enter() -> anyhow::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;

        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            default_hook(info);
        }));

        Ok(Self)
    }
}

impl Drop for AltScreenGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}
