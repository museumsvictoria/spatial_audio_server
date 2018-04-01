use installation::Installation;
use metres::Metres;
use std::collections::HashSet;
use time_calc::Ms;
use utils::{self, Range};

pub use self::realtime::Realtime;
pub use self::wav::Wav;

pub mod realtime;
pub mod wav;

/// Items related to audio sources.
///
/// Audio sources come in two kinds:
///
/// 1. WAV - pre-rendered n-channel .wav files and
/// 2. Realtime - input from some other currently running program (e.g. MSP, Live, etc).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Source {
    pub kind: Kind,
    #[serde(default)]
    pub role: Option<Role>,
    /// The distance with which the channels should be spread from the source position.
    ///
    /// If the source only has one channel, `spread` is ignored.
    #[serde(default = "default::spread")]
    pub spread: Metres,
    /// The rotation of the channels around the source position in radians.
    ///
    /// If the source only has one channel, `radians` is ignored.
    #[serde(default)]
    pub radians: f32,
}

/// A **Signal** yielding interleaved samples.
///
/// **Signal**s are produced by **Source**s and played back on the output thread via **Sound**s.
pub enum Signal {
    Wav {
        samples: wav::Signal,
        playback: wav::Playback,
    },
    Realtime {
        samples: realtime::Signal,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct Id(pub u64);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum Role {
    Soundscape(Soundscape),
    Interactive,
    Scribbles,
}

/// Properties specific to sources that have been assigned the "soundscape" role.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Soundscape {
    pub installations: HashSet<Installation>,
    #[serde(default = "default::occurrence_rate")]
    pub occurrence_rate: Range<Ms>,
    #[serde(default = "default::simultaneous_sounds")]
    pub simultaneous_sounds: Range<usize>,
    #[serde(default = "default::playback_duration")]
    pub playback_duration: Range<Ms>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Kind {
    Wav(Wav),
    Realtime(Realtime),
}

impl Source {
    pub fn channel_count(&self) -> usize {
        match self.kind {
            Kind::Wav(ref wav) => wav.channels,
            Kind::Realtime(ref rt) => rt.channels.len(),
        }
    }
}

impl Signal {
    /// Borrow the inner iterator yielding samples.
    pub fn samples(&mut self) -> &mut Iterator<Item = f32> {
        match *self {
            Signal::Wav { ref mut samples, .. } => samples.samples(),
            Signal::Realtime { ref mut samples } => samples as _,
        }
    }
}

impl Role {
    /// Returns `Some(Soundscape)` if the `Role` variant is `Soundscape`.
    ///
    /// Returns `None` otherwise.
    pub fn soundscape_mut(&mut self) -> Option<&mut Soundscape> {
        match *self {
            Role::Soundscape(ref mut soundscape) => Some(soundscape),
            _ => None,
        }
    }
}

impl Id {
    pub const INITIAL: Self = Id(0);
}

pub const MAX_PLAYBACK_DURATION: Ms = Ms(utils::DAY_MS);

pub mod default {
    use metres::Metres;
    use time_calc::Ms;
    use utils::{HR_MS, Range};
    pub const SPREAD: Metres = Metres(2.5);
    pub const OCCURRENCE_RATE: Range<Ms> = Range { min: Ms(500.0), max: Ms(HR_MS as _) };
    pub const SIMULTANEOUS_SOUNDS: Range<usize> = Range { min: 1, max: 3 };
    // Assume that the user wants to play back the sound endlessly at first.
    pub const PLAYBACK_DURATION: Range<Ms> = Range {
        min: super::MAX_PLAYBACK_DURATION,
        max: super::MAX_PLAYBACK_DURATION,
    };

    pub fn spread() -> Metres {
        SPREAD
    }

    pub fn occurrence_rate() -> Range<Ms> {
        OCCURRENCE_RATE
    }

    pub fn simultaneous_sounds() -> Range<usize> {
        SIMULTANEOUS_SOUNDS
    }

    pub fn playback_duration() -> Range<Ms> {
        PLAYBACK_DURATION
    }
}

impl Default for Soundscape {
    fn default() -> Self {
        let installations = Default::default();
        let occurrence_rate = default::OCCURRENCE_RATE;
        let simultaneous_sounds = default::SIMULTANEOUS_SOUNDS;
        let playback_duration = default::PLAYBACK_DURATION;
        Soundscape {
            installations,
            occurrence_rate,
            simultaneous_sounds,
            playback_duration,
        }
    }
}
