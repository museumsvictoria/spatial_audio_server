use serde::{Deserialize, Serialize};

use crate::metres::Metres;
use crate::utils::Seed;

/// Various configuration parameters for a single project.
#[derive(Copy, Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default::window_width")]
    pub window_width: u32,
    #[serde(default = "default::window_height")]
    pub window_height: u32,
    #[serde(default = "default::osc_input_port")]
    pub osc_input_port: u16,
    #[serde(default = "default::osc_input_log_limit")]
    pub osc_input_log_limit: usize,
    #[serde(default = "default::osc_output_log_limit")]
    pub osc_output_log_limit: usize,
    #[serde(default = "default::control_log_limit")]
    pub control_log_limit: usize,
    #[serde(default = "default::floorplan_pixels_per_metre")]
    pub floorplan_pixels_per_metre: f64,
    #[serde(default = "default::min_speaker_radius_metres")]
    pub min_speaker_radius_metres: Metres,
    #[serde(default = "default::max_speaker_radius_metres")]
    pub max_speaker_radius_metres: Metres,
    #[serde(default = "default::seed")]
    pub seed: Seed,
    /// The current value of proximity limit. The limit in meters
    /// for a speaker to be considered in the dbap calculations
    /// This value is squared for speed
    #[serde(default = "default::proximity_limit")]
    pub proximity_limit_2: Metres,
}

impl Default for Config {
    fn default() -> Self {
        let window_width = default::window_width();
        let window_height = default::window_height();
        let osc_input_port = default::osc_input_port();
        let osc_input_log_limit = default::osc_input_log_limit();
        let osc_output_log_limit = default::osc_output_log_limit();
        let control_log_limit = default::control_log_limit();
        let floorplan_pixels_per_metre = default::floorplan_pixels_per_metre();
        let min_speaker_radius_metres = default::min_speaker_radius_metres();
        let max_speaker_radius_metres = default::max_speaker_radius_metres();
        let seed = default::seed();
        let proximity_limit_2 = default::proximity_limit();
        Config {
            window_width,
            window_height,
            osc_input_port,
            osc_input_log_limit,
            osc_output_log_limit,
            control_log_limit,
            floorplan_pixels_per_metre,
            min_speaker_radius_metres,
            max_speaker_radius_metres,
            seed,
            proximity_limit_2,
        }
    }
}

// Fallback parameters in the case that they are missing from the file or invalid.
pub mod default {
    use crate::metres::Metres;
    use crate::utils::Seed;

    pub fn window_width() -> u32 {
        1280
    }
    pub fn window_height() -> u32 {
        720
    }
    pub fn osc_input_port() -> u16 {
        9001
    }
    pub fn osc_input_log_limit() -> usize {
        50
    }
    pub fn osc_output_log_limit() -> usize {
        10
    }
    pub fn control_log_limit() -> usize {
        50
    }
    pub fn floorplan_pixels_per_metre() -> f64 {
        148.0
    }
    pub fn min_speaker_radius_metres() -> Metres {
        0.25
    }
    pub fn max_speaker_radius_metres() -> Metres {
        1.0
    }

    pub fn seed() -> Seed {
        [0; 16]
    }

    pub fn proximity_limit() -> Metres {
        crate::audio::DEFAULT_PROXIMITY_LIMIT_2
    }
}
