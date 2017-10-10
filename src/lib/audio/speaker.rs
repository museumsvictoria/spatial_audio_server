use atomic::Atomic;
use cgmath::Point2;
use metres::Metres;
use std::sync::atomic::AtomicUsize;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct Id(pub u64);

/// Represents a virtual output at some location within the space.
///
/// These parameters are atomics in order to safely share them with the GUI thread.
#[derive(Deserialize, Serialize)]
pub struct Speaker {
    // The location of the speaker within the space.
    #[serde(with = "::serde_extra::atomic")]
    pub point: Atomic<Point2<Metres>>,
    // The channel on which the output is rendered.
    #[serde(with = "::serde_extra::atomic_usize")]
    pub channel: AtomicUsize,
}
