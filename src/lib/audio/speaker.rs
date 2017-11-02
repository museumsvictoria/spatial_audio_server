use metres::Metres;
use nannou::math::Point2;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct Id(pub u64);

/// Represents a virtual output at some location within the space.
///
/// These parameters are atomics in order to safely share them with the GUI thread.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Speaker {
    // The location of the speaker within the space.
    pub point: Point2<Metres>,
    // The channel on which the output is rendered.
    pub channel: usize,
}
