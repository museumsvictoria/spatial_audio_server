//! Items related to the realtime audio input sound source kind.

use crossbeam::sync::SegQueue;
use std::ops;
use std::sync::Arc;
use time_calc::{Ms, Samples};

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
    pub sample_rx: Arc<SegQueue<f32>>,
    pub channels: usize,
    pub remaining_samples: Option<usize>,
}

impl Signal {
    /// The number of channels in the source.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The number of frames remaining in the signal.
    ///
    /// Returns `None` if the signal is "continuous" or has no duration.
    pub fn remaining_frames(&self) -> Option<Samples> {
        self.remaining_samples.map(|s| Samples((s / self.channels) as _))
    }
}

impl Iterator for Signal {
    type Item = f32;
    fn next(&mut self) -> Option<Self::Item> {
        self.sample_rx
            .try_pop()
            .map(|sample| {
                self.remaining_samples = self.remaining_samples.map(|n| n.saturating_sub(1));
                sample
            })
    }
}
