//! Items related to Soundscape Groups.
//!
//! Soundscape groups allow for describing rules/constraints for multiple sounds at once.

use crate::utils::Range;
use serde::{Deserialize, Serialize};
use time_calc::Ms;

/// A name for a soundscape group.
#[derive(Clone, Debug, Eq, PartialOrd, Ord, PartialEq, Hash, Deserialize, Serialize)]
pub struct Name(pub String);

/// A more efficient unique identifier for a group.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct Id(pub usize);

/// A soundscape group.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Group {
    pub occurrence_rate: Range<Ms>,
    pub simultaneous_sounds: Range<usize>,
}

pub mod default {
    use crate::utils::{Range, HR_MS};
    use time_calc::Ms;
    pub const OCCURRENCE_RATE: Range<Ms> = Range {
        min: Ms(0.0),
        max: Ms(HR_MS as _),
    };
    pub const SIMULTANEOUS_SOUNDS: Range<usize> = Range { min: 1, max: 10 };
}

impl Default for Group {
    fn default() -> Self {
        let occurrence_rate = default::OCCURRENCE_RATE;
        let simultaneous_sounds = default::SIMULTANEOUS_SOUNDS;
        Group {
            occurrence_rate,
            simultaneous_sounds,
        }
    }
}
