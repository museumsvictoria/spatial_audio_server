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
}

impl Default for Config {
    fn default() -> Self {
        let project_default = Default::default();
        let selected_project_slug = default::project_slug();
        Config { project_default, selected_project_slug }
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
}
