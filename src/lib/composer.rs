use audio;
use std;
use std::collections::HashMap;
use std::sync::mpsc;

pub enum Message {
    UpdateSource(audio::source::Id, audio::Source),
    RemoveSource(audio::source::Id),
    Exit,
}

/// Spawn the "composer" thread.
///
/// The role of the composer thread is as follows:
///
/// 1. Compose `Sound`s from a stack of `Source` -> `[Effect]`.
/// 2. Compose the path of travel through the space (including rotations for multi-channel sounds).
/// 3. Send the `Sound`s to the audio thread and accompanying monitoring stuff to the GUI thread
///    (for tracking positions, RMS, etc).
pub fn spawn(
    audio_msg_tx: mpsc::Sender<audio::Message>,
    sound_id_gen: audio::sound::IdGenerator,
) -> (std::thread::JoinHandle<()>, mpsc::Sender<Message>) {
    let (tx, rx) = mpsc::channel();

    let handle = std::thread::Builder::new()
        .name("composer".into())
        .spawn(move || run(rx, audio_msg_tx, sound_id_gen))
        .unwrap();

    (handle, tx)
}

fn run(
    msg_rx: mpsc::Receiver<Message>,
    _audio_msg_tx: mpsc::Sender<audio::Message>,
    _sound_id_gen: audio::sound::IdGenerator,
) {
    // A map for storing all audio sources.
    let mut sources = HashMap::new();

    // Wait for messages.
    for msg in msg_rx {
        match msg {
            Message::UpdateSource(id, source) => {
                sources.insert(id, source);
            }

            Message::RemoveSource(id) => {
                sources.remove(&id);
            }

            Message::Exit => break,
        }
    }
}
