//! Declarations and definitions related to each unique installation.
//!
//! In a future generic version of the audio server, these installations should not be
//! hard-coded and rather identified via dynamically generated unique IDs. Otherwise, most
//! of the logic should remain the same.

use std::fmt;
use utils::Range;

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub enum Installation {
    WavesAtWork = 0,
    RipplesInSpacetime = 1,
    EnergeticVibrationsAudioVisualiser = 2,
    EnergeticVibrationsProjectionMapping = 3,
    TurbulentEncounters = 4,
    Cacophony = 5,
    WrappedInSpectrum = 6,
}

/// A unique identifier for a single computer within an installation.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct ComputerId(pub usize);

/// Constraints related to the soundscape.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Soundscape {
    #[serde(default = "default::simultaneous_sounds")]
    pub simultaneous_sounds: Range<usize>,
}

impl Default for Soundscape {
    fn default() -> Self {
        let simultaneous_sounds = default::SIMULTANEOUS_SOUNDS;
        Soundscape {
            simultaneous_sounds,
        }
    }
}

mod default {
    use utils::Range;

    pub const SIMULTANEOUS_SOUNDS: Range<usize> = Range { min: 1, max: 8 };

    pub fn simultaneous_sounds() -> Range<usize> {
        SIMULTANEOUS_SOUNDS
    }
}

impl Installation {
    pub fn display_str(&self) -> &str {
        match *self {
            Installation::WavesAtWork => "Waves At Work",
            Installation::RipplesInSpacetime => "Ripples In Spacetime",
            Installation::EnergeticVibrationsAudioVisualiser => {
                "Energetic Vibrations - Audio Visualiser"
            }
            Installation::EnergeticVibrationsProjectionMapping => {
                "Energetic Vibrations - Projection Mapping"
            }
            Installation::TurbulentEncounters => "Turbulent Encounters",
            Installation::Cacophony => "Cacophony",
            Installation::WrappedInSpectrum => "Wrapped In Spectrum",
        }
    }

    pub fn default_osc_addr_str(&self) -> &str {
        match *self {
            Installation::WavesAtWork => "wave",
            Installation::RipplesInSpacetime => "ripp",
            Installation::EnergeticVibrationsAudioVisualiser => "enav",
            Installation::EnergeticVibrationsProjectionMapping => "enpm",
            Installation::TurbulentEncounters => "turb",
            Installation::Cacophony => "caco",
            Installation::WrappedInSpectrum => "wrap",
        }
    }

    pub fn from_usize(i: usize) -> Option<Self> {
        ALL.get(i).map(|&i| i)
    }

    pub fn to_u32(&self) -> u32 {
        match *self {
            Installation::WavesAtWork => 0,
            Installation::RipplesInSpacetime => 1,
            Installation::EnergeticVibrationsAudioVisualiser => 2,
            Installation::EnergeticVibrationsProjectionMapping => 3,
            Installation::TurbulentEncounters => 4,
            Installation::Cacophony => 5,
            Installation::WrappedInSpectrum => 6,
        }
    }

    pub fn default_num_computers(&self) -> usize {
        match *self {
            Installation::WavesAtWork => 1,
            Installation::RipplesInSpacetime => 4,
            Installation::EnergeticVibrationsAudioVisualiser => 1,
            Installation::EnergeticVibrationsProjectionMapping => 3,
            Installation::TurbulentEncounters => 1,
            Installation::Cacophony => 1,
            Installation::WrappedInSpectrum => 2,
        }
    }
}

impl fmt::Display for Installation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.display_str())
    }
}

pub const ALL: &'static [Installation] = &[
    Installation::WavesAtWork,
    Installation::RipplesInSpacetime,
    Installation::EnergeticVibrationsAudioVisualiser,
    Installation::EnergeticVibrationsProjectionMapping,
    Installation::TurbulentEncounters,
    Installation::Cacophony,
    Installation::WrappedInSpectrum,
];
