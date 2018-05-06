//! Items related to the realtime audio input sound source kind.

use crossbeam::sync::SegQueue;
use std::mem;
use std::ops;
use std::sync::{atomic, Arc};
use std::sync::atomic::AtomicBool;
use time_calc::{Ms, Samples};

pub type BufferTx = Arc<SegQueue<Vec<f32>>>;
pub type BufferRx = Arc<SegQueue<Vec<f32>>>;

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
    pub buffer_rx: BufferRx,
    pub buffer_tx: BufferTx,
    pub sample_index: usize,
    pub current_buffer: Vec<f32>,
    pub channels: usize,
    pub remaining_samples: Option<usize>,
    pub is_closed: Arc<AtomicBool>,
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
        let Signal {
            ref buffer_rx,
            ref buffer_tx,
            ref mut sample_index,
            ref mut current_buffer,
            ref mut remaining_samples,
            ..
        } = *self;

        loop {
            if *sample_index < current_buffer.len() {
                let sample = current_buffer[*sample_index];
                *remaining_samples = remaining_samples.map(|n| n.saturating_sub(1));
                *sample_index += 1;
                return Some(sample);
            }
            match buffer_rx.try_pop() {
                None => return None,
                Some(buffer) => {
                    let used_buffer = mem::replace(current_buffer, buffer);
                    buffer_tx.push(used_buffer);
                    *sample_index = 0;
                },
            }
        }
    }
}

impl Drop for Signal {
    fn drop(&mut self) {
        self.is_closed.store(true, atomic::Ordering::Relaxed);
    }
}
