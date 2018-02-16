use metres::Metres;

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
    #[serde(default = "default_spread")]
    pub spread: Metres,
    /// The rotation of the channels around the source position in radians.
    ///
    /// If the source only has one channel, `radians` is ignored.
    #[serde(default)]
    pub radians: f32,
}

impl Source {
    pub fn channel_count(&self) -> usize {
        match self.kind {
            Kind::Wav(ref wav) => wav.channels,
            Kind::Realtime(ref rt) => rt.channels.len(),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct Id(pub u64);

impl Id {
    pub const INITIAL: Self = Id(0);
}

#[derive(Copy, Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum Role {
    Soundscape,
    Installation,
    Scribbles,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Kind {
    Wav(Wav),
    Realtime(Realtime),
}

pub fn default_spread() -> Metres {
    Metres(2.5)
}
