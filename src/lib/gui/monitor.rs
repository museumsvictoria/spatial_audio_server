//! A thread dedicated to receiving monitoring updates from the audio output thread and propagating
//! them to the GUI.
//!
//! We use an intermediary thread in order to avoid having the audio output thread be responsible
//! for waking the main (GUI) thread using the app proxy. This is because the app proxy may or may
//! not require performing some kind of I/O depending on the platform, in turn taking an
//! unpredictable amount of time.

use gui;
use nannou;
use std::io;
use std::sync::{mpsc, Arc};
use std::sync::atomic::{self, AtomicBool};
use std::thread;

pub type Sender = mpsc::Sender<gui::AudioMonitorMessage>;
pub type Receiver = mpsc::Receiver<gui::AudioMonitorMessage>;
pub type Spawned = (Monitor, Sender, Receiver);

/// A handle to the GUI monitoring thread.
pub struct Monitor {
    thread: thread::JoinHandle<()>,
    is_closed: Arc<AtomicBool>,
}

impl Monitor {
    /// Closes the monitoring thread.
    pub fn close(&self) {
        self.is_closed.store(true, atomic::Ordering::Relaxed);
    }

    /// Waits for the thread
    pub fn join(self) -> thread::Result<()> {
        self.close();
        let Monitor { thread, .. } = self;
        thread.join()
    }
}

/// Spawn the intermediary monitoring thread and return the communication channels.
pub fn spawn(app_proxy: nannou::app::Proxy) -> io::Result<Spawned> {
    let (audio_tx, audio_rx) = mpsc::channel();
    let (gui_tx, gui_rx) = mpsc::channel();
    let is_closed = Arc::new(AtomicBool::new(false));
    let is_closed_2 = is_closed.clone();

    let thread = thread::Builder::new()
        .name("gui_audio_monitor".into())
        .spawn(move || {
            // TODO: Work out how to use this in a way that is not so expensive on Mac.
            // Perhaps use some atomic bool flag which indicates whether or not the GUI needs
            // waking up.
            // Attempt to forward every message and wakeup the GUI when successful.
            let mut msgs = vec![];
            'run: while !is_closed_2.load(atomic::Ordering::Relaxed) {
                // Receive all pending msgs.
                msgs.extend(audio_rx.try_iter());

                // If there are no pending messages, wait for the next one.
                if msgs.is_empty() {
                    msgs.extend(audio_rx.recv().ok());
                }

                // Process the messages.
                for msg in msgs.drain(..) {
                    match gui_tx.send(msg) {
                        Ok(()) => {
                            // Proxy is currently buggy on linux so we only enable this for macos.
                            if cfg!(target_os = "macos") {
                                if app_proxy.wakeup().is_err() {
                                    eprintln!("audio_monitor proxy could not wakeup app");
                                    break 'run;
                                }
                            }
                        },
                        Err(_) => break 'run,
                    }
                }
            }
        })?;

    let monitor = Monitor { thread, is_closed };
    Ok((monitor, audio_tx, gui_rx))
}
