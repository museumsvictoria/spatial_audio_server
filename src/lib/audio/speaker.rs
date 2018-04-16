use audio;
use fxhash::FxHashSet;
use installation;
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
    // Installations assigned to this speaker.
    #[serde(default)]
    pub installations: FxHashSet<installation::Id>,
}

/// Calculate a speaker's DBAP weight taking into consideration its assigned installations.
pub fn dbap_weight(
    sound_installations: &audio::sound::Installations,
    speaker_installations: &FxHashSet<installation::Id>,
) -> f64
{
    match *sound_installations {
        audio::sound::Installations::All => 1.0,
        audio::sound::Installations::Set(ref set) => {
            match set.intersection(&speaker_installations).next() {
                Some(_) => 1.0,
                None => 0.0,
            }
        },
    }
}
