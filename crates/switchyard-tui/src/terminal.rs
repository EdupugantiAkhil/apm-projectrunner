use std::{io, panic, sync::Arc};

use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

type PanicHook = dyn Fn(&panic::PanicHookInfo<'_>) + Send + Sync + 'static;

pub(crate) struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    previous_hook: Arc<PanicHook>,
}

impl TerminalSession {
    pub(crate) fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let previous_hook: Arc<PanicHook> = panic::take_hook().into();
        let panic_hook = Arc::clone(&previous_hook);
        panic::set_hook(Box::new(move |info| {
            restore();
            panic_hook(info);
        }));
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableBracketedPaste) {
            restore_hook(&previous_hook);
            restore();
            return Err(error);
        }
        let terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                restore_hook(&previous_hook);
                restore();
                return Err(error);
            }
        };
        Ok(Self {
            terminal,
            previous_hook,
        })
    }

    pub(crate) fn draw(&mut self, render: impl FnOnce(&mut ratatui::Frame<'_>)) -> io::Result<()> {
        self.terminal.draw(render).map(|_| ())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        restore();
        restore_hook(&self.previous_hook);
    }
}

fn restore_hook(previous_hook: &Arc<PanicHook>) {
    let _ = panic::take_hook();
    let previous = Arc::clone(previous_hook);
    panic::set_hook(Box::new(move |info| previous(info)));
}

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
}
