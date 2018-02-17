use audio;
use std;
use std::collections::HashMap;
use std::sync::mpsc;

/// The kinds of messages received by the soundscape thread.
pub enum Message {
    InsertSource(audio::source::Id, audio::Source),
    UpdateSource(audio::source::Id, UpdateSourceFn),
    RemoveSource(audio::source::Id),
    Exit,
}

/// The update function applied to a source.
///
/// This is a workaround for the current inability to call a `Box<FnOnce>`
pub struct UpdateSourceFn {
    function: Box<FnMut(&mut audio::Source) + Send>,
}

/// Spawn the "soundscape" thread.
///
/// The role of the soundscape thread is as follows:
///
/// 1. Compose `Sound`s from a stack of `Source` -> `[Effect]`.
/// 2. Compose the path of travel through the space (including rotations for multi-channel sounds).
/// 3. Send the `Sound`s to the audio thread and accompanying monitoring stuff to the GUI thread
///    (for tracking positions, RMS, etc).
pub fn spawn(
    audio_output_stream: audio::output::Stream,
    sound_id_gen: audio::sound::IdGenerator,
) -> (std::thread::JoinHandle<()>, mpsc::Sender<Message>) {
    let (tx, rx) = mpsc::channel();

    let handle = std::thread::Builder::new()
        .name("soundscape".into())
        .spawn(move || run(rx, audio_output_stream, sound_id_gen))
        .unwrap();

    (handle, tx)
}

fn run(
    msg_rx: mpsc::Receiver<Message>,
    _audio_output_stream: audio::output::Stream,
    _sound_id_gen: audio::sound::IdGenerator,
) {
    // A map for storing all audio sources.
    let mut sources = HashMap::new();

    // Wait for messages.
    for msg in msg_rx {
        match msg {
            Message::InsertSource(id, source) => {
                sources.insert(id, source);
            }
            Message::UpdateSource(id, update) => {
                if let Some(source) = sources.get_mut(&id) {
                    update.call(source);
                }
            }

            Message::RemoveSource(id) => {
                sources.remove(&id);
            }

            Message::Exit => break,
        }
    }
}

impl UpdateSourceFn {
    /// Consume self and call the update function with the given source.
    pub fn call(mut self, source: &mut audio::Source) {
        (self.function)(source)
    }
}

impl<F> From<F> for UpdateSourceFn
where
    F: FnOnce(&mut audio::Source) + Send + 'static,
{
    fn from(f: F) -> Self {
        let mut f_opt = Some(f);
        let fn_mut = move |source: &mut audio::Source| {
            if let Some(f) = f_opt.take() {
                f(source);
            }
        };
        UpdateSourceFn {
            function: Box::new(fn_mut) as _,
        }
    }
}
