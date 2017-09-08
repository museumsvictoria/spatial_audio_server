use sample::Signal;
use std;
use std::collections::HashMap;
use std::sync::mpsc;
pub use self::requester::Requester;

pub mod backend;
mod requester;


/// The maximum number of audio channels.
pub const MAX_CHANNELS: usize = 32;

/// A single frame of audio.
pub type Frame = [f32; MAX_CHANNELS];

/// Messages that drive the audio engine forward.
pub enum Message {
    /// A request for some frames from the audio backend thread.
    ///
    /// All frames in `buffer` should be written to and then sent back to the audio IO thread as
    /// soon as possible via the given `buffer_tx`.
    RequestAudio(requester::Buffer<Frame>, f64),
    /// Add a new audio source to the map.
    AddSource(SourceId, Source),
    /// Remove an audio source from the map.
    RemoveSource(SourceId),
}

impl requester::Message for Message {
    type Frame = Frame;
    fn audio_request(buffer: requester::Buffer<Frame>, sample_hz: f64) -> Self {
        Message::RequestAudio(buffer, sample_hz)
    }
}

/// Run the audio engine thread.
///
/// This should be run prior to the backend audio thread so that the audio engine is ready to start
/// processing audio.
pub fn spawn() -> mpsc::Sender<Message> {
    let (msg_tx, msg_rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("audio_engine".into())
        .spawn(move || { run(msg_rx); })
        .unwrap();

    msg_tx
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
struct SourceId(u64);
struct Source {
    signal: Box<Signal<Frame=[f32; 2]> + Send>,
}


// The function to be run onthe audio engine thread.
fn run(msg_rx: mpsc::Receiver<Message>) {

    // A map from audio source IDs to the audio sources themselves.
    let mut sources = HashMap::with_capacity(1024);

    let mut request_id: u64 = 0;

    // Wait for messages.
    for msg in msg_rx {
        match msg {
            Message::RequestAudio(buffer, sample_hz) => {
                request_id += 1;
            },

            Message::AddSource(id, source) => {
                sources.insert(id, source);
            }

            Message::RemoveSource(id) => {
                sources.remove(&id);
            }
        }
    }
}
