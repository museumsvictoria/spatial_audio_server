use audio::Wav;
use time_calc::Ms;

/// Items related to audio sources.
///
/// Audio sources come in two kinds:
///
/// 1. WAV - pre-rendered n-channel .wav files and
/// 2. Realtime - input from some other currently running program (e.g. MSP, Live, etc).
#[derive(Deserialize, Serialize)]
pub struct Source {
    pub kind: Kind,
}

#[derive(Deserialize, Serialize)]
pub enum Kind {
    Wav(Wav),
    Realtime(Realtime),
}

#[derive(Deserialize, Serialize)]
pub struct Realtime {
    pub channels: usize,
    // Durationn for which the realtime input is played.
    pub duration: Ms,
    // Need some input type
}
