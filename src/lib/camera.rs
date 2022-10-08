use crate::metres::Metres;
use serde::{Deserialize, Serialize};
type Scalar = f64;

/// A 2D camera location in exhibition space.
pub type Point = nannou::glam::DVec2;

/// Items related to the camera that provides a view of the floorplan.
#[derive(Debug, Serialize, Deserialize)]
pub struct Camera {
    /// The position of the camera over the floorplan.
    ///
    /// [0.0, 0.0] - the centre of the floorplan.
    #[serde(default = "default_position")]
    pub position: Point,
    /// The higher the zoom, the closer the floorplan appears.
    ///
    /// The zoom can be multiplied by a distance in metres to get the equivalent distance as a GUI
    /// scalar value.
    ///
    /// 1.0 - Original resolution.
    /// 0.5 - 50% view.
    #[serde(default = "default_zoom")]
    pub zoom: f64,
    /// The number of floorplan pixels per metre loaded from the project config file.
    ///
    /// Note: Although this parameter is saved and loaded to the project's state JSON, the
    /// parameter is always updated via the "floorplan_pixels_per_metre" loaded via the config
    /// TOML.
    #[serde(default = "default_floorplan_pixels_per_metre")]
    pub floorplan_pixels_per_metre: f64,
}

impl Camera {
    /// Convert from metres to the GUI scalar value.
    pub fn metres_to_scalar(&self, metres: Metres) -> Scalar {
        self.zoom * metres * self.floorplan_pixels_per_metre
    }

    /// Convert from the GUI scalar value to metres.
    pub fn scalar_to_metres(&self, scalar: Scalar) -> Metres {
        ((scalar / self.zoom) / self.floorplan_pixels_per_metre) as Metres
    }
}

impl Default for Camera {
    fn default() -> Self {
        let position = default_position();
        let zoom = default_zoom();
        let floorplan_pixels_per_metre = default_floorplan_pixels_per_metre();
        Camera {
            position,
            zoom,
            floorplan_pixels_per_metre,
        }
    }
}

fn default_position() -> Point {
    [(0.0), (0.0)].into()
}

fn default_zoom() -> f64 {
    0.0
}

fn default_floorplan_pixels_per_metre() -> f64 {
    94.0
}
