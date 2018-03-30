//! Items related to the realtime audio input sound source kind.

use std::ops;
use std::sync::mpsc;
use time_calc::Ms;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Realtime {
    // Duration for which the realtime input is played.
    pub duration: Ms,
    // The range of channels occuppied by the source.
    pub channels: ops::Range<usize>,
}

/// The signal end of a `Realtime` audio source.
///
/// Implemented as a simple wrapper around an spsc receiver.
///
/// Returns all samples as it receives them.
///
/// Returns `None` as soon as the inner receiver either runs out of samples due to falling behind
/// or if the channel is disconneceted as the sound has played out its duration.
pub struct Signal {
    pub sample_rx: mpsc::Receiver<f32>,
}

impl Iterator for Signal {
    type Item = f32;
    fn next(&mut self) -> Option<Self::Item> {
        self.sample_rx.try_recv().ok()
    }
}
