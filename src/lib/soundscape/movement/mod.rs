use crate::audio;
use crate::metres::Metres;

pub use self::agent::Agent;
pub use self::ngon::Ngon;

pub mod agent;
pub mod ngon;

type Point2 = nannou::glam::DVec2;

/// Whether the sound has fixed movement or generative movement.
#[derive(Debug)]
pub enum Movement {
    /// The sound has a fixed location relative to the centre of the installation.
    Fixed(audio::sound::Position),
    /// The sound's movement is guided by a generative algorithm.
    Generative(Generative),
}

/// Generative movement kinds applied to an active sound.
#[derive(Debug)]
pub enum Generative {
    /// Uses steering behaviour and basic forces to move an agent toward its desired location.
    Agent(Agent),
    /// A 2D N-sided, symmetrical polygon path tracing movement implementation.
    Ngon(Ngon),
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
    pub centroid: Point2,
}

impl Generative {
    /// Determine the position and orientation of the sound.
    pub fn position(&self) -> audio::sound::Position {
        match *self {
            Generative::Agent(ref agent) => agent.position(),
            Generative::Ngon(ref ngon) => ngon.position(),
        }
    }
}

impl Movement {
    /// Determine the position and orientation of the sound.
    pub fn position(&self) -> audio::sound::Position {
        match *self {
            Movement::Fixed(position) => position,
            Movement::Generative(ref generative) => generative.position(),
        }
    }
}

impl BoundingRect {
    /// Initialise a bounding box at a single point in space.
    pub fn from_point(p: Point2) -> Self {
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
        I: IntoIterator<Item = Point2>,
    {
        let mut points = points.into_iter();
        points.next().map(|p| {
            let init = BoundingRect::from_point(p);
            points.fold(init, BoundingRect::with_point)
        })
    }

    /// Extend the bounding box to include the given point.
    pub fn with_point(self, p: Point2) -> Self {
        BoundingRect {
            left: p.x.min(self.left),
            right: p.x.max(self.right),
            bottom: p.y.min(self.bottom),
            top: p.y.max(self.top),
        }
    }

    /// The middle of the bounding box.
    pub fn middle(&self) -> Point2 {
        let width = self.width();
        let height = self.height();
        let x = self.left + width * 0.5;
        let y = self.bottom + height * 0.5;
        Point2::new(x, y)
    }

    pub fn width(&self) -> Metres {
        self.right - self.left
    }

    pub fn height(&self) -> Metres {
        self.top - self.bottom
    }
}
