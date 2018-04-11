use installation::Installation;
use metres::Metres;
use nannou::math::map_range;
use nannou::rand::Rng;
use soundscape;
use std::collections::HashSet;
use std::ops;
use time_calc::{Ms, Samples};
use utils::{self, Range};

pub use self::realtime::Realtime;
pub use self::wav::Wav;

pub mod realtime;
pub mod wav;

pub const MAX_PLAYBACK_DURATION: Ms = Ms(utils::DAY_MS);

pub const MAX_ATTACK_DURATION: Ms = Ms(utils::MIN_MS);

pub const MAX_RELEASE_DURATION: Ms = Ms(utils::MIN_MS);

/// Items related to audio sources.
///
/// Audio sources come in two kinds:
///
/// 1. WAV - pre-rendered n-channel .wav files and
/// 2. Realtime - input from some other currently running program (e.g. MSP, Live, etc).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Source {
    /// The kind of source (WAV or Realtime).
    pub kind: Kind,
    /// The role of the source within the exhibition.
    #[serde(default)]
    pub role: Option<Role>,
    /// The distance with which the channels should be spread from the source position.
    ///
    /// If the source only has one channel, `spread` is ignored.
    #[serde(default = "default::spread")]
    pub spread: Metres,
    /// The rotation of the channels around the source position in radians.
    ///
    /// This is a constant offset that is added to a sound's orientation when determining channel
    /// locations during playback.
    ///
    /// If the source only has one channel, `radians` is ignored.
    #[serde(default = "default::channel_radians")]
    pub channel_radians: f32,
    /// An amplitude modulator specified by the user via the GUI.
    #[serde(default = "default::volume")]
    pub volume: f32,
    /// Whether or not the source has been muted.
    #[serde(default)]
    pub muted: bool,
}

/// A **Signal** yielding interleaved samples.
///
/// **Signal**s are produced by **Source**s and played back on the output thread via **Sound**s.
pub struct Signal {
    pub kind: SignalKind,
    attack: Attack,
    release: Release,
    // The duration of the signal if one was specified.
    //
    // If `None`, the signal will just play out until the `SignalKind` samples return `None`.
    duration: Option<Duration>,
}

/// The kind of the **Signal**.
///
/// Indicates whether the signal is sourced from a `Wav` or `Realtime` source.
pub enum SignalKind {
    Wav {
        samples: wav::Signal,
        playback: wav::Playback,
    },
    Realtime {
        samples: realtime::Signal,
    },
}

/// An iterator yielding `Some` until the `current_frame` reaches `duration_frames`.
#[derive(Clone, Debug)]
pub struct Duration {
    duration_frames: Samples,
    current_frame: Samples,
}

/// An iterator producing the volume modifier for an attack envelope.
#[derive(Clone, Debug)]
pub struct Attack {
    duration_frames: Samples,
    current_frame: Samples,
}

/// An iterator producing the volume modifier for a release envelope.
#[derive(Clone, Debug)]
pub struct Release {
    duration_frames: Samples,
    frame_countdown: Samples,
}

/// The samples produced by a source signal with attack and release applied.
pub struct SignalSamples<'a> {
    channels: usize,
    /// The number of samples until the release should kick in based on the duration of the sound.
    frames_until_release_begins: Samples,
    gain_per_channel: GainPerChannel,
    attack: &'a mut Attack,
    release: &'a mut Release,
    duration: &'a mut Option<Duration>,
    samples: &'a mut Iterator<Item = f32>,
}

/// An iterator yielding the same gain for each channel in a frame.
#[derive(Clone, Debug)]
pub struct GainPerChannel {
    channels: ops::Range<usize>,
    gain: f32,
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
    #[serde(default)]
    pub installations: HashSet<Installation>,
    #[serde(default)]
    pub groups: HashSet<soundscape::group::Id>,
    #[serde(default = "default::occurrence_rate")]
    pub occurrence_rate: Range<Ms>,
    #[serde(default = "default::simultaneous_sounds")]
    pub simultaneous_sounds: Range<usize>,
    #[serde(default = "default::playback_duration")]
    pub playback_duration: Range<Ms>,
    #[serde(default = "default::attack_duration")]
    pub attack_duration: Range<Ms>,
    #[serde(default = "default::release_duration")]
    pub release_duration: Range<Ms>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Kind {
    Wav(Wav),
    Realtime(Realtime),
}

impl Kind {
    /// The value used to skew the playback duration to a suitable linear range for a slider.
    ///
    /// This is dependent upon whether or not the source is potentially infinite.
    pub fn playback_duration_skew(&self) -> f32 {
        match *self {
            Kind::Realtime(_) => skew::PLAYBACK_DURATION_MAX,
            Kind::Wav(ref wav) => match wav.should_loop {
                true => skew::PLAYBACK_DURATION_MAX,
                false => playback_duration_skew(wav.duration.to_ms(super::SAMPLE_RATE)),
            },
        }
    }
}

/// Skew some given playback duration into a "perceived" linear range.
///
/// This is useful for GUI sliders and for generating durations based on some given range.
pub fn playback_duration_skew(duration: Ms) -> f32 {
    let no_skew = 1.0;
    if duration < Ms(utils::SEC_MS) {
        return no_skew;
    }
    if duration > MAX_PLAYBACK_DURATION {
        return skew::PLAYBACK_DURATION_MAX;
    }
    map_range(
        duration.0,
        utils::SEC_MS,
        MAX_PLAYBACK_DURATION.0,
        no_skew,
        skew::PLAYBACK_DURATION_MAX,
    )
}

/// Generate a random playback duration within the given range.
pub fn random_playback_duration<R>(mut rng: R, range: Range<Ms>) -> Ms
where
    R: Rng,
{
    let range_duration = range.max - range.min;
    let skew = playback_duration_skew(range_duration);
    let skewed_normalised_value = rng.gen::<f64>().powf(skew as f64);
    Ms(utils::unskew_and_unnormalise(skewed_normalised_value, range.min.0, range.max.0, skew))
}

impl Source {
    pub fn channel_count(&self) -> usize {
        match self.kind {
            Kind::Wav(ref wav) => wav.channels,
            Kind::Realtime(ref rt) => rt.channels.len(),
        }
    }
}

impl Attack {
    /// Construct an `Attack` from its duration in frames.
    pub fn from_duration_frames(duration_frames: Samples) -> Self {
        let current_frame = Samples(0);
        Attack { duration_frames, current_frame }
    }
}

impl Release {
    /// Construct a `Release` from its duration in frames.
    pub fn from_duration_frames(duration_frames: Samples) -> Self {
        let frame_countdown = duration_frames;
        Release { duration_frames, frame_countdown }
    }
}

impl Duration {
    /// Construct a `Duration` from its frames.
    pub fn from_frames(duration_frames: Samples) -> Self {
        let current_frame = Samples(0);
        Duration { duration_frames, current_frame }
    }
}

impl SignalKind {
    /// The number of frames remaining in the signal.
    fn remaining_frames(&self) -> Option<Samples> {
        match *self {
            SignalKind::Wav { ref samples, .. } => samples.remaining_frames(),
            SignalKind::Realtime { .. } => None,
        }
    }

    /// The number of channels in the signal.
    pub fn channels(&self) -> usize {
        match *self {
            SignalKind::Wav { ref samples, .. } => samples.channels(),
            SignalKind::Realtime { ref samples } => samples.channels(),
        }
    }

    /// Borrow the inner iterator yielding samples.
    pub fn samples(&mut self) -> &mut Iterator<Item = f32> {
        match *self {
            SignalKind::Wav { ref mut samples, .. } => samples.samples(),
            SignalKind::Realtime { ref mut samples } => samples as _,
        }
    }
}

impl Signal {
    /// Construct a new `Signal` from the given source kind, attack and release frames.
    pub fn new(kind: SignalKind, attack_frames: Samples, release_frames: Samples) -> Self {
        let attack = Attack::from_duration_frames(attack_frames);
        let release = Release::from_duration_frames(release_frames);
        let duration = None;
        Signal { kind, attack, release, duration }
    }

    /// Specify the duration of the signal in frames in the `Signal`.
    pub fn with_duration_frames(mut self, frames: Samples) -> Self {
        self.duration = Some(Duration::from_frames(frames));
        self
    }

    // The minimum number of frames between `self.remaining_frames` and
    // `self.kind.remaining_frames()` if any.
    //
    // This returns `None` if the `Signal` has know end.
    fn remaining_frames(&self) -> Option<Samples> {
        let remaining_frames = self.duration.as_ref().map(Duration::remaining_frames);
        let kind_remaining_frames = self.kind.remaining_frames();
        match (remaining_frames, kind_remaining_frames) {
            (Some(a), Some(b)) => Some(::std::cmp::min(a, b)),
            (Some(a), _) => Some(a),
            (_, Some(b)) => Some(b),
            _ => None,
        }
    }

    /// Borrow the inner iterator yielding samples and apply the attack and release.
    pub fn samples(&mut self) -> SignalSamples {
        let remaining_frames = self.remaining_frames();

        let Signal {
            ref mut kind,
            ref mut attack,
            ref mut release,
            ref mut duration,
        } = *self;

        // If the signal has no duration, this will be some max `i64` value that should never get
        // close.
        let frames_until_release_begins = match remaining_frames {
            Some(frames) => {
                let frames_until_release = frames_until_release_begins(frames, release);
                // If the release has already started, make sure the release countdown is up to date.
                if frames_until_release == Samples(0) {
                    release.frame_countdown = ::std::cmp::min(release.frame_countdown, frames);
                }
                frames_until_release
            },
            None => Samples(::std::i64::MAX),
        };

        let channels = kind.channels();
        let samples = kind.samples();
        let gain_per_channel = GainPerChannel {
            channels: 0..0,
            gain: 0.0,
        };
        SignalSamples {
            channels,
            frames_until_release_begins,
            gain_per_channel,
            attack,
            release,
            samples,
            duration,
        }
    }
}

fn frames_until_release_begins(signal_remaining_frames: Samples, release: &Release) -> Samples {
    if signal_remaining_frames < release.duration_frames {
        Samples(0)
    } else {
        signal_remaining_frames - release.duration_frames
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

impl Attack {
    fn next_gain(&mut self) -> f32 {
        if self.current_frame < self.duration_frames {
            let current = self.current_frame.samples() as f32;
            let duration = self.duration_frames.samples() as f32;
            self.current_frame += Samples(1);
            current / duration
        } else {
            1.0
        }
    }
}

impl Release {
    fn next_gain(&mut self) -> f32 {
        if self.frame_countdown > Samples(0) {
            let current = self.frame_countdown.samples() as f32;
            let duration = self.duration_frames.samples() as f32;
            self.frame_countdown -= Samples(1);
            current / duration
        } else {
            1.0
        }
    }
}

impl Duration {
    /// The number of remaining frames in the duration.
    pub fn remaining_frames(&self) -> Samples {
        self.duration_frames - self.current_frame
    }
}

impl Iterator for Duration {
    type Item = ();
    fn next(&mut self) -> Option<Self::Item> {
        if self.current_frame < self.duration_frames {
            self.current_frame += Samples(1);
            Some(())
        } else {
            None
        }
    }
}

impl Iterator for GainPerChannel {
    type Item = f32;
    fn next(&mut self) -> Option<Self::Item> {
        self.channels.next().map(|_| self.gain)
    }
}

impl<'a> Iterator for SignalSamples<'a> {
    type Item = f32;
    fn next(&mut self) -> Option<Self::Item> {
        let SignalSamples {
            channels,
            ref mut frames_until_release_begins,
            ref mut gain_per_channel,
            ref mut attack,
            ref mut release,
            ref mut duration,
            ref mut samples,
        } = *self;

        loop {
            if let Some(gain) = gain_per_channel.next() {
                return samples.next().map(|s| s * gain);
            }

            if let Some(duration) = duration.as_mut() {
                if duration.next().is_none() {
                    return None;
                }
            }

            let channels = 0..channels;
            let attack_gain = attack.next_gain();
            let release_gain = if *frames_until_release_begins == Samples(0) {
                release.next_gain()
            } else {
                *frames_until_release_begins -= Samples(1);
                1.0
            };
            let gain = attack_gain * release_gain;
            *gain_per_channel = GainPerChannel { channels, gain };
        }
    }
}

/// The values used to skew parameters in order to create a linear range across their perceptual
/// differences.
pub mod skew {
    pub const ATTACK: f32 = 0.5;
    pub const RELEASE: f32 = 0.5;
    pub const PLAYBACK_DURATION_MAX: f32 = 0.1;
}

pub mod default {
    use metres::Metres;
    use time_calc::Ms;
    use utils::{HR_MS, Range};

    pub const SPREAD: Metres = Metres(2.5);
    // Rotate the channel radians 90deg so that stereo channels are to the side by default.
    pub const CHANNEL_RADIANS: f32 = ::std::f32::consts::PI * 0.25;
    pub const VOLUME: f32 = 0.6;
    pub const OCCURRENCE_RATE: Range<Ms> = Range { min: Ms(500.0), max: Ms(HR_MS as _) };
    pub const SIMULTANEOUS_SOUNDS: Range<usize> = Range { min: 0, max: 1 };
    // Assume that the user wants to play back the sound endlessly at first.
    pub const PLAYBACK_DURATION: Range<Ms> = Range {
        min: super::MAX_PLAYBACK_DURATION,
        max: super::MAX_PLAYBACK_DURATION,
    };
    pub const ATTACK_DURATION: Range<Ms> = Range { min: Ms(0.0), max: Ms(0.0) };
    pub const RELEASE_DURATION: Range<Ms> = Range { min: Ms(0.0), max: Ms(0.0) };

    pub fn spread() -> Metres {
        SPREAD
    }

    pub fn channel_radians() -> f32 {
        CHANNEL_RADIANS
    }

    pub fn volume() -> f32 {
        VOLUME
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

    pub fn attack_duration() -> Range<Ms> {
        ATTACK_DURATION
    }

    pub fn release_duration() -> Range<Ms> {
        RELEASE_DURATION
    }
}

impl Default for Soundscape {
    fn default() -> Self {
        let installations = Default::default();
        let groups = Default::default();
        let occurrence_rate = default::OCCURRENCE_RATE;
        let simultaneous_sounds = default::SIMULTANEOUS_SOUNDS;
        let playback_duration = default::PLAYBACK_DURATION;
        let attack_duration = default::ATTACK_DURATION;
        let release_duration = default::RELEASE_DURATION;
        Soundscape {
            installations,
            groups,
            occurrence_rate,
            simultaneous_sounds,
            playback_duration,
            attack_duration,
            release_duration,
        }
    }
}
