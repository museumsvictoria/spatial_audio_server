use audio;
use installation::Installation;
use metres::Metres;
use nannou::math::Point2;
use std;
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};

/// The kinds of messages received by the soundscape thread.
enum Message {
    Update(UpdateFn),
    Exit,
}

/// The update function applied to a source.
///
/// This is a workaround for the current inability to call a `Box<FnOnce>`
pub struct UpdateFn {
    function: Box<FnMut(&mut Model) + Send>,
}

/// The handle to the soundscape that can be used and shared amonth the main thread.
#[derive(Clone)]
pub struct Soundscape {
    tx: mpsc::Sender<Message>,
    // Keep the thread handle in an `Option` so we can take it from the mutex upon exit.
    thread: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
}

/// Data related to a single speaker that is relevant to the soundscape.
pub struct Speaker {
    /// The position of the speaker in metres.
    pub point: Point2<Metres>,
}

/// The model containing all state running on the soundscape thread.
pub struct Model {
    /// All sources available to the soundscape for producing audio.
    sources: HashMap<audio::source::Id, audio::Source>,
    /// This is used to determine the "area" for each installation.
    installation_speakers: HashMap<Installation, HashMap<audio::speaker::Id, audio::Speaker>>,
    /// A handle for submitting new sounds to the output stream.
    audio_output_stream: audio::output::Stream,
    /// For generating unique IDs for each new sound.
    sound_id_gen: audio::sound::IdGenerator,
}

impl Soundscape {
    /// Send a `FnOnce(&mut Model)` function to update the soundscape thread model.
    pub fn send<F>(&self, update: F) -> Result<(), mpsc::SendError<()>>
    where
        F: FnOnce(&mut Model) + Send + 'static,
    {
        let update = UpdateFn::from(update);
        let msg = Message::Update(update);
        if let Err(mpsc::SendError(_)) = self.tx.send(msg) {
            return Err(mpsc::SendError(()));
        }
        Ok(())
    }

    /// Stops the soundscape thread and returns the raw handle to its thread.
    pub fn exit(self) -> Option<std::thread::JoinHandle<()>> {
        self.tx.send(Message::Exit).ok();
        self.thread.lock().unwrap().take()
    }
}

impl Model {
    /// Insert a source into the inner hashmap.
    pub fn insert_source(
        &mut self,
        id: audio::source::Id,
        source: audio::Source,
    ) -> Option<audio::Source>
    {
        self.sources.insert(id, source)
    }

    /// Updates the source with the given function.
    ///
    /// Returns `false` if the source wasn't there.
    pub fn update_source<F>(&mut self, id: &audio::source::Id, update: F) -> bool
    where
        F: FnOnce(&mut audio::Source),
    {
        match self.sources.get_mut(id) {
            None => false,
            Some(s) => {
                update(s);
                true
            },
        }
    }

    /// Insert a source into the inner hashmap.
    pub fn remove_source(&mut self, id: &audio::source::Id) -> Option<audio::Source> {
        self.sources.remove(id)
    }
}

impl UpdateFn {
    // Consume self and call the update function with the given source.
    fn call(mut self, model: &mut Model) {
        (self.function)(model)
    }
}

impl<F> From<F> for UpdateFn
where
    F: FnOnce(&mut Model) + Send + 'static,
{
    fn from(f: F) -> Self {
        let mut f_opt = Some(f);
        let fn_mut = move |source: &mut Model| {
            if let Some(f) = f_opt.take() {
                f(source);
            }
        };
        UpdateFn {
            function: Box::new(fn_mut) as _,
        }
    }
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
) -> Soundscape {
    let (tx, rx) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("soundscape".into())
        .spawn(move || run(rx, audio_output_stream, sound_id_gen))
        .unwrap();
    let thread = Arc::new(Mutex::new(Some(thread)));
    Soundscape { tx, thread }
}

// A blocking function that is run on the unique soundscape thread (called by spawn).
fn run(
    msg_rx: mpsc::Receiver<Message>,
    audio_output_stream: audio::output::Stream,
    sound_id_gen: audio::sound::IdGenerator,
) {
    let sources = HashMap::new();
    let installation_speakers = HashMap::new();

    // The model maintaining state between messages.
    let mut model = Model {
        sources,
        installation_speakers,
        audio_output_stream,
        sound_id_gen,
    };

    // Wait for messages.
    for msg in msg_rx {
        match msg {
            Message::Update(update) => update.call(&mut model),
            Message::Exit => break,
        }
    }
}
