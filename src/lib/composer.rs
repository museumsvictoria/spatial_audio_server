use audio;
use std;

pub enum Message {
    AddSource(audio::Source),
    RemoveSource(audio::source::Id),
}

/// Spawn the "composer" thread.
///
/// The role of the composer thread is as follows:
///
/// 1. Compose `Sound`s from a stack of `Source` -> `[Effect]`.
/// 2. Compose the path of travel through the space (including rotations for multi-channel sounds).
/// 3. Send the `Sound`s to the audio thread and accompanying monitoring stuff to the GUI thread
///    (for tracking positions, RMS, etc).
pub fn spawn() -> std::thread::JoinHandle<()> {

    let handle = std::thread::Builder::new()
        .name("composer".into())
        .spawn(move || run())
        .unwrap();

    handle
}

fn run() {

}
