use std;
use std::path::Path;
use toml;

/// Various configuration parameters for the audio_server loaded on startup.
#[derive(Copy, Clone, Debug, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default = "default::window_width")]
    pub window_width: u32,
    #[serde(default = "default::window_height")]
    pub window_height: u32,
    #[serde(default = "default::osc_input_port")]
    pub osc_input_port: u16,
    #[serde(default = "default::osc_log_limit")]
    pub osc_log_limit: usize,
    #[serde(default = "default::interaction_log_limit")]
    pub interaction_log_limit: usize,
    #[serde(default = "default::floorplan_pixels_per_metre")]
    pub floorplan_pixels_per_metre: f64,
}

// Fallback parameters in the case that they are missing from the file or invalid.
pub mod default {
    pub fn window_width() -> u32 { 1280 }
    pub fn window_height() -> u32 { 720 }
    pub fn osc_input_port() -> u16 { 9001 }
    pub fn osc_log_limit() -> usize { 50 }
    pub fn interaction_log_limit() -> usize { 50 }
    pub fn floorplan_pixels_per_metre() -> f64 { 148.0 }
}

/// Load the `Config` from the toml file at the given path.
pub fn load(path: &Path) -> Result<Config, std::io::Error> {
    // Load the `toml` string from the given file.
    let mut file = std::fs::File::open(&path)?;
    let mut contents = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut contents)?;
    let toml_str = std::str::from_utf8(&contents[..]).unwrap();

    // Parse the `String` into a `Toml` type.
    Ok(toml::from_str(toml_str).unwrap())
}
