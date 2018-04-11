use audio;
use metres::Metres;
use nannou::prelude::*;

pub use self::agent::Agent;

pub mod agent;

/// The movement kind applied to an active sound.
#[derive(Debug)]
pub enum Movement {
    /// Uses steering behaviour and basic forces to move an agent toward its desired
    /// location.
    Agent(Agent),
}

/// The bounding box for an iterator yielding points.
#[derive(Copy, Clone, Debug)]
pub struct BoundingRect {
    pub left: Metres,
    pub right: Metres,
    pub top: Metres,
    pub bottom: Metres,
}

/// Includes the bounding box and
#[derive(Copy, Clone, Debug)]
pub struct Area {
    pub bounding_rect: BoundingRect,
    pub centroid: Point2<Metres>,
}

impl Movement {
    /// Determine the position and orientation of the sound.
    pub fn position(&self) -> audio::sound::Position {
        match *self {
            Movement::Agent(ref agent) => agent.position(),
        }
    }
}

impl BoundingRect {
    /// Initialise a bounding box at a single point in space.
    pub fn from_point(p: Point2<Metres>) -> Self {
        BoundingRect {
            left: p.x,
            right: p.x,
            top: p.y,
            bottom: p.y,
        }
    }

    /// Determine the movement area bounds on the given set of points.
    pub fn from_points<I>(points: I) -> Option<Self>
    where
        I: IntoIterator<Item = Point2<Metres>>,
    {
        let mut points = points.into_iter();
        points.next().map(|p| {
            let init = BoundingRect::from_point(p);
            points.fold(init, BoundingRect::with_point)
        })
    }

    /// Extend the bounding box to include the given point.
    pub fn with_point(self, p: Point2<Metres>) -> Self {
        BoundingRect {
            left: p.x.min(self.left),
            right: p.x.max(self.right),
            bottom: p.y.min(self.bottom),
            top: p.y.max(self.top),
        }
    }

    /// The middle of the bounding box.
    pub fn middle(&self) -> Point2<Metres> {
        Point2 {
            x: (self.left + self.right) * 0.5,
            y: (self.bottom + self.top) * 0.5,
        }
    }
}
