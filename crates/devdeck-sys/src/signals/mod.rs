//! devdeck's own signals (U4): the ones that ask it to quit.

mod unix;

/// SIGTERM, SIGHUP and SIGINT, taken from their default action (quitting at once) from
/// now on, and waited for.
pub struct QuitSignals(signal_hook::iterator::Signals);

impl QuitSignals {
    pub fn new() -> std::io::Result<QuitSignals> {
        unix::quit_signals().map(QuitSignals)
    }

    /// Blocks until one comes: its number, or None when the wait ended.
    pub fn wait(&mut self) -> Option<i32> {
        self.0.forever().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hang_up_comes_through() {
        let mut signals = QuitSignals::new().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(signals.wait());
        });
        // Taken from now on: raising it doesn't end the test process.
        rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::HUP).unwrap();
        let got = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("the signal within 5 s");
        assert_eq!(got, Some(signal_hook::consts::SIGHUP));
    }
}
