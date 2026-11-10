use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

pub fn quit_signals() -> std::io::Result<Signals> {
    Signals::new([SIGTERM, SIGHUP, SIGINT])
}
