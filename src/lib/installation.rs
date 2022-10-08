//! Declarations and definitions related to each unique installation.
//!
//! In a future generic version of the audio server, these installations should not be
//! hard-coded and rather identified via dynamically generated unique IDs. Otherwise, most
//! of the logic should remain the same.

use crate::utils::Range;
use serde::{Deserialize, Deserializer, Serialize};
use slug::slugify;

/// All known beyond perception installations (used by default).
pub const BEYOND_PERCEPTION_NAMES: &'static [&'static str] = &[
    "Waves At Work",
    "Ripples In Spacetime",
    "Energetic Vibrations - Audio Visualiser",
    "Energetic Vibrations - Projection Mapping",
    "Turbulent Encounters",
    "Cacophony",
    "Wrapped In Spectrum",
    "Turret 1",
    "Turret 2",
];

/// A memory efficient unique identifier for an installation.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize)]
pub struct Id(pub usize);

// Support loading installations from the old enum format.
impl<'de> Deserialize<'de> for Id {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match serde_json::Value::deserialize(d)? {
            // Deserialize the Id as a string rather than an int.
            serde_json::Value::String(string) => {
                // Check for old enum variant names.
                let id = match &string[..] {
                    "WavesAtWork" => Id(0),
                    "RipplesInSpacetime" => Id(1),
                    "EnergeticVibrationsAudioVisualiser" => Id(2),
                    "EnergeticVibrationsProjectionMapping" => Id(3),
                    "TurbulentEncounters" => Id(4),
                    "Cacophony" => Id(5),
                    "WrappedInSpectrum" => Id(6),
                    "Turret1" => Id(7),
                    "Turret2" => Id(8),
                    s => Id(s.parse().expect("could not parse installation id as usize")),
                };

                Ok(id)
            }
            serde_json::Value::Number(n) => {
                let u = n.as_u64().expect("could not deserialize Id number to u64") as usize;
                Ok(Id(u))
            }
            err => panic!(
                "failed to deserialize `Id`: expected String or Int, found {:?}",
                err
            ),
        }
    }
}

/// An installation's computers.
pub type Computers = computer::Addresses;

/// A single installation within the exhibition.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Installation {
    /// The name of the Installation.
    #[serde(default = "default::name_string")]
    pub name: String,
    /// All computers within the exhibition.
    #[serde(default)]
    pub computers: Computers,
    /// Constraints related to the soundscape.
    #[serde(default)]
    pub soundscape: Soundscape,
}

impl Default for Installation {
    fn default() -> Self {
        let name = default::name().into();
        let computers = Default::default();
        let soundscape = Default::default();
        Installation {
            name,
            computers,
            soundscape,
        }
    }
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
        Soundscape {
            simultaneous_sounds,
        }
    }
}

/// Produces the OSC address string - a slugified version of the installation's name.
pub fn osc_addr_string(name: &str) -> String {
    format!("/{}", slugify(name))
}

/// The default number of computers for the beyond perception installation with the given name.
pub fn beyond_perception_default_num_computers(name: &str) -> Option<usize> {
    let n = match name {
        "Waves At Work" => 0,
        "Ripples In Spacetime" => 0,
        "Energetic Vibrations - Audio Visualiser" => 0,
        "Energetic Vibrations - Projection Mapping" => 3,
        "Turbulent Encounters" => 0,
        "Cacophony" => 0,
        "Wrapped In Spectrum" => 0,
        "Turret 1" => 0,
        "Turret 2" => 0,
        _ => return None,
    };
    Some(n)
}

/// Default soundscape constraints.
pub mod default {
    use crate::utils::Range;

    pub const SIMULTANEOUS_SOUNDS: Range<usize> = Range { min: 1, max: 8 };

    pub fn name() -> &'static str {
        "<unnamed>"
    }

    pub fn name_string() -> String {
        name().into()
    }

    pub fn simultaneous_sounds() -> Range<usize> {
        SIMULTANEOUS_SOUNDS
    }
}

/// State related to the computers available to an installation.
pub mod computer {
    use fxhash::FxHashMap;
    use serde::{Deserialize, Serialize};
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
