use analytics_ipc::tui::{App, render};
use analytics_ipc::{DEFAULT_MAX_ROWS, connect_snapshot};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::env;
use std::io::{self, stdout};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread;
use std::time::Duration;

struct Args {
    socket: PathBuf,
    limit: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut args = env::args().skip(1);
    let mut socket = None;
    let mut limit = 1_000;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => socket = args.next().map(PathBuf::from),
            "--limit" => {
                limit = args
                    .next()
                    .ok_or("--limit requires a value")?
                    .parse()
                    .map_err(|_| "--limit must be an integer")?;
            }
            "-h" | "--help" => return Err("usage: lqtop --socket PATH [--limit N]".into()),
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    let socket = socket.ok_or("--socket PATH is required")?;
    if !(1..=DEFAULT_MAX_ROWS).contains(&limit) {
        return Err(format!("--limit must be between 1 and {DEFAULT_MAX_ROWS}"));
    }
    Ok(Args { socket, limit })
}

fn poller(
    socket: PathBuf,
    limit: usize,
) -> Receiver<Result<analytics_ipc::AnalyticsSnapshot, String>> {
    let (tx, rx) = sync_channel(1);
    thread::spawn(move || {
        loop {
            let result = connect_snapshot(&socket, limit).map_err(|error| error.to_string());
            let _ = tx.try_send(result);
            thread::sleep(Duration::from_secs(1));
        }
    });
    rx
}

fn run(args: Args) -> io::Result<()> {
    enable_raw_mode()?;
    let mut output = stdout();
    execute!(output, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(output);
    let mut terminal = Terminal::new(backend)?;
    let rx = poller(args.socket, args.limit);
    let mut app = App::default();
    let result = loop {
        while let Ok(snapshot) = rx.try_recv() {
            match snapshot {
                Ok(snapshot) => app.apply_snapshot(snapshot),
                Err(_) => app.mark_stale(),
            }
        }
        terminal.draw(|frame| render(frame, &mut app))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            app.handle_key(key);
        }
        if app.should_quit {
            break Ok(());
        }
    };
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn main() {
    let args = parse_args().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    if let Err(error) = run(args) {
        eprintln!("lqtop: {error}");
        std::process::exit(1);
    }
}
