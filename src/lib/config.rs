use project;
use std::ops::Deref;

/// Various configuration parameters for a single project.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    /// The default configuration for a project.
    #[serde(default)]
    pub project_default: project::Config,
    /// The directory stem of the selected project.
    #[serde(default = "default::project_slug")]
    pub selected_project_slug: String,
    /// Whether or not CPU saving mode is enabled upon opening the server.
    #[serde(default = "default::cpu_saving_mode")]
    pub cpu_saving_mode: bool,
    /// Specify the name of the device that the audio server should use as the input audio device.
    /// The first device that contains the given string will be selected.
    ///
    /// If the device cannot be found, or if the string is empty, the default input device will be
    /// selected.
    #[serde(default)]
    pub target_input_device_name: String,
    /// Specify the name of the device that the audio server should use as the output audio device.
    ///
    /// If the device cannot be found, or if the string is empty, the default output device will be
    /// selected.
    #[serde(default)]
    pub target_output_device_name: String,
}

impl Default for Config {
    fn default() -> Self {
        let project_default = Default::default();
        let selected_project_slug = default::project_slug();
        let cpu_saving_mode = Default::default();
        let target_input_device_name = Default::default();
        let target_output_device_name = Default::default();
        Config {
            project_default,
            selected_project_slug,
            cpu_saving_mode,
            target_input_device_name,
            target_output_device_name,
        }
    }
}

impl Deref for Config {
    type Target = project::Config;
    fn deref(&self) -> &Self::Target {
        &self.project_default
    }
}

mod default {
    use project;
    use slug::slugify;
    pub fn project_slug() -> String {
        slugify(project::default_project_name())
    }

    pub fn cpu_saving_mode() -> bool {
        false
    }
}
