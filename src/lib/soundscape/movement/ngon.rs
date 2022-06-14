use super::BoundingRect;
use crate::audio;
use crate::utils::duration_to_secs;
use nannou::glam::DVec2 as Vector2;
use nannou::prelude::*;
use std::time;

// The point and vector types in exhibition space.
type Point = Vector2;
type Vector = Vector2;

/// A 2D N-sided, symmetrical polygon path tracing movement implementation.
///
/// The bounding rectangle of the Ngon will match that of the installation to which it is assigned.
#[derive(Debug)]
pub struct Ngon {
    /// The number of sides (or vertices) in the ngon.
    pub vertices: usize,
    /// The path should travel between every "nth" vertex.
    ///
    /// For example, if this value was `2`
    pub nth: usize,
    /// Describes the radius of the **Ngon** using a normalised value.
    ///
    /// `0.0` means all points will be in the center.
    /// `1.0` means all points will extend to the bounds of the installation area.
    pub normalised_dimensions: Vector2,
    /// Some rotation that is applied to the Ngon's points around the centre.
    pub radians_offset: f64,
    /// The rate at which the path is being travelled in metres per second.
    pub speed: f64,
    /// State that is updated during a call to `Update`.
    state: State,
}

/// State that is updated during a call to `Update`.
#[derive(Debug)]
struct State {
    /// State describing the current position through the Ngon path.
    position: Position,
    /// The same as `position` but described in "metres" space over the exhibition.
    sound_position: audio::sound::Position,
}

#[derive(Debug)]
struct Position {
    /// The current line that is being travelled described by the indices of its points.
    line: Line,
    /// The normalised position along the line (0.0 is start, 1.0 is end).
    lerp: f64,
}

/// The current line that is being travelled.
#[derive(Debug)]
struct Line {
    /// The Ngon vertex index that is the starting location for this line.
    start: usize,
    /// The Ngon vertex index that is the ending location for this line.
    end: usize,
}

impl Ngon {
    /// Create a new **Ngon** movement type.
    pub fn new(
        vertices: usize,
        nth: usize,
        normalised_dimensions: Vector2,
        radians_offset: f64,
        speed: f64,
        installation_bounding_rect: &BoundingRect,
    ) -> Self {
        let start = 0;
        let end = (start + nth) % vertices;
        let line = Line { start, end };
        let lerp = 0.0;
        let position = Position { line, lerp };
        let radians = 0.0;
        let (middle, half_dim) =
            middle_and_half_dimensions(installation_bounding_rect, normalised_dimensions);
        let point = vertex_at_index(vertices, middle, half_dim, radians_offset, 0);
        let sound_position = audio::sound::Position { point, radians };
        let state = State {
            sound_position,
            position,
        };
        Ngon {
            vertices,
            nth,
            normalised_dimensions,
            radians_offset,
            speed,
            state,
        }
    }
}

// Produce the vertex for the given index.
fn vertex_at_index(
    vertices: usize,
    middle: Point,
    half_dimensions: Vector,
    radians_offset: f64,
    index: usize,
) -> Point {
    let step = index as f64 / vertices as f64;
    let radians = step * 2.0 * PI_F64 + radians_offset;
    let x = middle.x + half_dimensions.x * radians.cos();
    let y = middle.y + half_dimensions.y * radians.sin();
    Point::new(x, y)
}

// The middle of the given bounding rect and normalised dimensions halved read for use within the
// `vertex_at_index` function.
fn middle_and_half_dimensions(
    bounding_rect: &BoundingRect,
    normalised_dimensions: Vector2,
) -> (Point, Vector) {
    let middle = bounding_rect.middle();
    let width = bounding_rect.width() * normalised_dimensions.x;
    let height = bounding_rect.height() * normalised_dimensions.y;
    let half_width = width * 0.5;
    let half_height = height * 0.5;
    let half_dimensions = Point::new(half_width, half_height);
    (middle, half_dimensions)
}

impl Ngon {
    /// The current position along the Ngon path.
    pub fn position(&self) -> audio::sound::Position {
        self.state.sound_position
    }

    /// Update the `Ngon` state for the given past amount of time.
    pub fn update(&mut self, delta_time: &time::Duration, installation_area: &BoundingRect) {
        let Ngon {
            vertices,
            nth,
            normalised_dimensions,
            radians_offset,
            speed,
            ref mut state,
        } = *self;

        // Find the middle and the half width and height.
        let (middle, half_dimensions) =
            middle_and_half_dimensions(installation_area, normalised_dimensions);

        // Shorthand for finding a vertex at a specific index.
        let vertex_at_index =
            |index| vertex_at_index(vertices, middle, half_dimensions, radians_offset, index);

        // Determine the current position of the Ngon tracer.
        let mut travel_distance = speed * duration_to_secs(delta_time);
        let (point, lerp) = loop {
            let start = vertex_at_index(state.position.line.start);
            let end = vertex_at_index(state.position.line.end);
            let vec = start.lerp(end, state.position.lerp);
            let point = Point::new(vec.x, vec.y);
            let distance = point.distance(end).abs();

            // If there's no distance to travel, make sure the point is up to date with the
            // installation bounds and return.
            if travel_distance == 0.0 || distance == 0.0 {
                state.sound_position.point = point;
                return;
            }

            if travel_distance < distance {
                let start_to_end = end - start;
                let start_to_end_distance = start_to_end.length();
                let travel = if start_to_end_distance > 0.0 {
                    start_to_end.normalize() * travel_distance
                } else {
                    Vector::new(0.0, 0.0)
                };
                let new_point = point + travel;
                let new_distance = new_point.distance(end).abs();
                let new_lerp = if start_to_end_distance > 0.0 {
                    (start_to_end_distance - new_distance) / start_to_end_distance
                } else {
                    0.0
                };
                let new_point = new_point;
                break (new_point, new_lerp);
            }
            travel_distance -= distance;

            // Update the line start and end indices.
            state.position.lerp = 0.0;
            state.position.line.start = state.position.line.end;
            state.position.line.end = (state.position.line.end + nth) % vertices;
        };

        // Update the position.
        state.sound_position.point = point;
        state.position.lerp = lerp;
    }
}
