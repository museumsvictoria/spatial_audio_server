use crate::audio;
use crate::metres::Metres;
use serde::{Deserialize, Serialize};
use time_calc::Ms;

/// Master state of the project.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Master {
    /// The master volume for the exhibition, between 0.0 (silence) and 1.0 (full).
    #[serde(default = "default_master_volume")]
    pub volume: f32,
    /// The latency applied to real-time input sources.
    #[serde(default = "default_realtime_source_latency")]
    pub realtime_source_latency: Ms,
    /// The rolloff decibel amount, used to attenuate speaker gains over distances.
    #[serde(default = "default_dbap_rolloff_db")]
    pub dbap_rolloff_db: f64,
    /// The current value of proximity limit. The limit in meters
    /// for a speaker to be considered in the dbap calculations
    /// It is stored as a square for faster calculations
    #[serde(default = "default_proximity_limit")]
    pub proximity_limit_2: Metres,
}

impl Default for Master {
    fn default() -> Self {
        let volume = default_master_volume();
        let realtime_source_latency = default_realtime_source_latency();
        let dbap_rolloff_db = default_dbap_rolloff_db();
        let proximity_limit_2 = default_proximity_limit();
        Master {
            volume,
            realtime_source_latency,
            dbap_rolloff_db,
            proximity_limit_2,
        }
    }
}

fn default_master_volume() -> f32 {
    audio::DEFAULT_MASTER_VOLUME
}

fn default_realtime_source_latency() -> Ms {
    audio::DEFAULT_REALTIME_SOURCE_LATENCY
}

fn default_dbap_rolloff_db() -> f64 {
    audio::DEFAULT_DBAP_ROLLOFF_DB
}

fn default_proximity_limit() -> Metres {
    audio::DEFAULT_PROXIMITY_LIMIT_2
}
