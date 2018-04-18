//! Declarations and definitions related to each unique installation.
//!
//! In a future generic version of the audio server, these installations should not be
//! hard-coded and rather identified via dynamically generated unique IDs. Otherwise, most
//! of the logic should remain the same.

use std::fmt;
use utils::Range;

/// The ID of all installations in the beyond perception exhibition.
pub const ALL: &'static [Id] = &[
    Id::WavesAtWork,
    Id::RipplesInSpacetime,
    Id::EnergeticVibrationsAudioVisualiser,
    Id::EnergeticVibrationsProjectionMapping,
    Id::TurbulentEncounters,
    Id::Cacophony,
    Id::WrappedInSpectrum,
    Id::Turret1,
    Id::Turret2,
];

/// A unique identifier for referring to an installation.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub enum Id {
    WavesAtWork = 0,
    RipplesInSpacetime = 1,
    EnergeticVibrationsAudioVisualiser = 2,
    EnergeticVibrationsProjectionMapping = 3,
    TurbulentEncounters = 4,
    Cacophony = 5,
    WrappedInSpectrum = 6,
    Turret1 = 7,
    Turret2 = 8,
}

/// An installation's computers.
pub type Computers = computer::Addresses;

/// A single installation within the exhibition.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Installation {
    /// All computers within the exhibition.
    pub computers: Computers,
    /// Constraints related to the soundscape.
    pub soundscape: Soundscape,
}

/// Constraints related to the soundscape.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Soundscape {
    #[serde(default = "default::simultaneous_sounds")]
    pub simultaneous_sounds: Range<usize>,
}

impl Default for Soundscape {
    fn default() -> Self {
        let simultaneous_sounds = default::SIMULTANEOUS_SOUNDS;
        Soundscape { simultaneous_sounds }
    }
}

impl Id {
    pub fn display_str(&self) -> &str {
        match *self {
            Id::WavesAtWork => "Waves At Work",
            Id::RipplesInSpacetime => "Ripples In Spacetime",
            Id::EnergeticVibrationsAudioVisualiser => {
                "Energetic Vibrations - Audio Visualiser"
            }
            Id::EnergeticVibrationsProjectionMapping => {
                "Energetic Vibrations - Projection Mapping"
            }
            Id::TurbulentEncounters => "Turbulent Encounters",
            Id::Cacophony => "Cacophony",
            Id::WrappedInSpectrum => "Wrapped In Spectrum",
            Id::Turret1 => "Turret 1",
            Id::Turret2 => "Turret 2",
        }
    }

    pub fn default_osc_addr_str(&self) -> &str {
        match *self {
            Id::WavesAtWork => "wave",
            Id::RipplesInSpacetime => "ripp",
            Id::EnergeticVibrationsAudioVisualiser => "enav",
            Id::EnergeticVibrationsProjectionMapping => "enpm",
            Id::TurbulentEncounters => "turb",
            Id::Cacophony => "caco",
            Id::WrappedInSpectrum => "wrap",
            Id::Turret1 => "tur1",
            Id::Turret2 => "tur2",
        }
    }

    pub fn from_usize(i: usize) -> Option<Self> {
        ALL.get(i).map(|&i| i)
    }

    pub fn to_u32(&self) -> u32 {
        match *self {
            Id::WavesAtWork => 0,
            Id::RipplesInSpacetime => 1,
            Id::EnergeticVibrationsAudioVisualiser => 2,
            Id::EnergeticVibrationsProjectionMapping => 3,
            Id::TurbulentEncounters => 4,
            Id::Cacophony => 5,
            Id::WrappedInSpectrum => 6,
            Id::Turret1 => 7,
            Id::Turret2 => 8,
        }
    }

    pub fn default_num_computers(&self) -> usize {
        match *self {
            Id::WavesAtWork => 1,
            Id::RipplesInSpacetime => 4,
            Id::EnergeticVibrationsAudioVisualiser => 1,
            Id::EnergeticVibrationsProjectionMapping => 3,
            Id::TurbulentEncounters => 1,
            Id::Cacophony => 1,
            Id::WrappedInSpectrum => 2,
            Id::Turret1 => 0,
            Id::Turret2 => 0,
        }
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.display_str())
    }
}

/// Default soundscape constraints.
pub mod default {
    use fxhash::FxHashMap;
    use utils::Range;

    pub const SIMULTANEOUS_SOUNDS: Range<usize> = Range { min: 1, max: 8 };

    pub fn simultaneous_sounds() -> Range<usize> {
        SIMULTANEOUS_SOUNDS
    }

    /// The default map of installations used by the Beyond Perception project.
    pub fn map() -> FxHashMap<super::Id, super::Installation> {
        super::ALL
            .iter()
            .map(|&id| {
                let computers = (0..id.default_num_computers())
                    .map(|i| {
                        let computer = super::computer::Id(i);
                        let socket = "127.0.0.1:9002".parse().unwrap();
                        let osc_addr_base = id.default_osc_addr_str().to_string();
                        let osc_addr = format!("/{}/{}", osc_addr_base, i);
                        let addr = super::computer::Address { socket, osc_addr };
                        (computer, addr)
                    })
                    .collect();
                let soundscape = Default::default();
                let installation = super::Installation { computers, soundscape };
                (id, installation)
            })
            .collect()
    }
}

/// State related to the computers available to an installation.
pub mod computer {
    use fxhash::FxHashMap;
    use std::net;

    /// A unique identifier for a single computer within an installation.
    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
    pub struct Id(pub usize);

    /// The address of a single 
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct Address {
        // The IP address of the target installation computer.
        pub socket: net::SocketAddrV4,
        // The OSC address string.
        pub osc_addr: String,
    }

    /// A map from all computer Ids to their addresses.
    pub type Addresses = FxHashMap<Id, Address>;
}
