use project;
use toml;

/// Various configuration parameters for a single project.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    /// The default configuration for a project.
    #[serde(default)]
    pub project_default: project::Config,
    /// The directory stem of the selected project.
    #[serde(default = "default_project_slug")]
    pub selected_project_slug: String,
}
