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
use std::sync::mpsc;
use std::thread;

pub type Sender = mpsc::SyncSender<gui::AudioMonitorMessage>;
pub type Receiver = mpsc::Receiver<gui::AudioMonitorMessage>;
pub type Spawned = (thread::JoinHandle<()>, Sender, Receiver);

/// Spawn the intermediary monitoring thread and return the communication channels.
pub fn spawn(proxy: nannou::app::Proxy) -> io::Result<Spawned> {
    let (audio_tx, audio_rx) = mpsc::sync_channel(1024);
    let (gui_tx, gui_rx) = mpsc::sync_channel(1024);

    let handle = thread::Builder::new()
        .name("gui_audio_monitor".into())
        .stack_size(1024) // 512 bytes - a tiny stack for a tiny job.
        .spawn(move || {
            // Attempt to forward every message and wakeup the GUI when successful.
            let mut msgs = vec![];
            'run: loop {
                // Receive all pending msgs.
                msgs.extend(audio_rx.try_iter());

                // If there are no pending messages, wait for the next one.
                if msgs.is_empty() {
                    msgs.extend(audio_rx.recv().ok());
                }

                // Process the messages.
                for msg in msgs.drain(..) {
                    match gui_tx.try_send(msg) {
                        Ok(_) => {
                            if proxy.wakeup().is_err() {
                                eprintln!("audio_monitor proxy could not wakeup app");
                                break 'run;
                            }
                        },
                        Err(mpsc::TrySendError::Disconnected(_)) => break 'run,
                        _ => (),
                    }
                }
            }
        })?;

    Ok((handle, audio_tx, gui_rx))
}
