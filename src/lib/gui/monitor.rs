//! A thread dedicated to receiving monitoring updates from the audio output thread and propagating
//! them to the GUI.
//!
//! We use an intermediary thread in order to avoid having the audio output thread be responsible
//! for waking the main (GUI) thread using the app proxy. This is because the app proxy may or may
//! not require performing some kind of I/O depending on the platform, in turn taking an
//! unpredictable amount of time.

use crossbeam::sync::{MsQueue, SegQueue};
use gui;
use nannou;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{self, AtomicBool};
use std::thread;

pub type Sender = Arc<MsQueue<gui::AudioMonitorMessage>>;
pub type Receiver = Arc<SegQueue<gui::AudioMonitorMessage>>;
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
    let audio_queue = Arc::new(MsQueue::new());
    let audio_tx = audio_queue.clone();
    let audio_rx = audio_queue;

    let gui_queue = Arc::new(SegQueue::new());
    let gui_tx = gui_queue.clone();
    let gui_rx = gui_queue;

    let is_closed = Arc::new(AtomicBool::new(false));
    let is_closed_2 = is_closed.clone();

    let thread = thread::Builder::new()
        .name("gui_audio_monitor".into())
        .spawn(move || {
            // TODO: Work out how to use this in a way that is not so expensive on Mac.
            // Perhaps use some atomic bool flag which indicates whether or not the GUI needs
            // waking up.
            // Attempt to forward every message and wakeup the GUI when successful.
            'run: while !is_closed_2.load(atomic::Ordering::Relaxed) {
                let msg = audio_rx.pop();
                gui_tx.push(msg);
                // Proxy is currently buggy on linux so we only enable this for macos.
                if cfg!(target_os = "macos") {
                    if app_proxy.wakeup().is_err() {
                        eprintln!("audio_monitor proxy could not wakeup app");
                        break 'run;
                    }
                }
            }
        })?;

    let monitor = Monitor { thread, is_closed };
    Ok((monitor, audio_tx, gui_rx))
}
