use audio;
use installation::Installation;
use metres::Metres;
use nannou::prelude::*;
use nannou::rand::Rng;
use std::collections::HashMap;
use std::{cmp, time};
use utils::{self, duration_to_secs, pt2, vt2};

// The minimum distance that the point may be from the target before it may switch to the next.
const TARGET_DISTANCE_THRESHOLD: Metres = Metres(1.0);

// The point and vector types in exhibition space.
type Point = Point2<Metres>;
type Vector = Vector2<Metres>;

/// An automous-agent style movement kind.
///
/// The agent must consider:
///
/// - A desired target location within one of the assigned installations.
/// - A user-defined movement weight affecting the max velocity and rotation speeds.
#[derive(Debug)]
pub struct Agent {
    /// The current location of the agent.
    location: Point,
    /// The desired, "target" location that the agent wants to reach.
    target_location: Point,
    /// The current velocity whose magnitude describes speed in metres per second.
    velocity: Vector,
    /// Used to limit the magnitude of the desired velocity in metres per second.
    pub max_speed: f64,
    /// Used to limit the magnitude of the steering force.
    pub max_force: f64,
}

/// Information about an installation required by the Agent.
#[derive(Debug)]
pub struct InstallationData {
    /// The installation area.
    pub area: super::Area,
    /// The number of sounds that the installation needs before its current target is satisfied.
    pub num_sounds_needed_to_reach_target: i32,
    /// The number of sounds that the installation needs before it satisfies the constraints.
    pub num_sounds_needed: usize,
    /// The number of sounds that can be added to the installation before the constraints would be
    /// exceeded.
    pub num_available_sounds: usize,
}

/// A map of installation data relevant to the agent.
pub type InstallationDataMap = HashMap<Installation, InstallationData>;

impl Agent {
    /// Generate a new agent starting in the given installation area.
    pub fn generate<R>(
        mut rng: R,
        start_installation: Installation,
        installations: &InstallationDataMap,
        max_speed: f64,
        max_force: f64,
    ) -> Self
    where
        R: Rng,
    {
        let location =
            generate_installation_target(&mut rng, &installations[&start_installation].area);
        let target_location = generate_target(&mut rng, installations);
        // TODO: Generate these based on "weight" or whatever user params are decided upon.
        let start_magnitude = rng.gen::<f64>() * max_speed;
        let desired_velocity = desired_velocity(location, target_location);
        let desired_radians = desired_velocity.y.0.atan2(desired_velocity.x.0);
        // TODO: Generate initial angle and create velocity from this.
        let initial_radians = desired_radians + rng.gen::<f64>() * 2.0 - 1.0;
        let velocity = {
            let (vx, vy) = utils::rad_mag_to_x_y(initial_radians, start_magnitude);
            [Metres(vx), Metres(vy)].into()
        };
        let agent = Agent {
            location,
            target_location,
            velocity,
            max_speed,
            max_force,
        };
        agent
    }

    /// The current location and orientation of the **Agent** for use within the audio engine's
    /// DBAP calculations..
    pub fn position(&self) -> audio::sound::Position {
        let point = self.location;
        let vel = vt2::to_f64(self.velocity);
        let radians = vel.y.atan2(vel.x) as f32;
        audio::sound::Position { point, radians }
    }

    /// Produce the agent's target seeking force for its current state.
    ///
    /// The force is in `Metres` per second and should be applied accordingly.
    pub fn seek_force(&self) -> Vector {
        seek_force(
            self.location,
            self.target_location,
            self.velocity,
            self.max_speed,
            self.max_force,
        )
    }

    /// Applies the given force to the agent, updating its internal state appropriately.
    pub fn apply_force(&mut self, force: Vector, delta_time: &time::Duration) {
        let new_velocity = vt2::to_f64(self.velocity) + vt2::to_f64(force);
        self.velocity = vt2::to_metres(new_velocity);
        let delta_secs = duration_to_secs(delta_time);
        self.location = pt2::to_metres(pt2::to_f64(self.location) + new_velocity * delta_secs);
    }

    /// Update the agent for the given past amount of time.
    pub fn update<R>(
        &mut self,
        mut rng: R,
        delta_time: &time::Duration,
        installations: &InstallationDataMap,
    ) where
        R: Rng,
    {
        // We can't know where to go if there are no assigned installations.
        if !installations.is_empty() {
            if should_pick_new_target(self.location, self.target_location, &installations) {
                self.target_location = generate_target(&mut rng, &installations);
            }
        }

        // Determine the steering force to apply based on how much time has passed.
        let force = self.seek_force();
        self.apply_force(force, delta_time);

        // If we've reached the target, pick a new one.
        if reached_target(self.location, self.target_location) {
            if !installations.is_empty() {
                self.target_location = generate_target(rng, installations);
            }
        }
    }
}

/// Decide whether or not the target_location should be regenerated due to a lack of available
/// sounds at the installation closest to that target.
///
/// **Panics** if the given installation map is empty.
fn should_pick_new_target(
    current_location: Point,
    target_location: Point,
    installations: &InstallationDataMap,
) -> bool {
    // Find the installation closest to the target area.
    let (target_installation, target_installation_data) =
        closest_installation(target_location, &installations)
            .expect("the given installations map was empty");

    // If there are no available sounds and we're not already within the installation pick a new
    // target.
    if target_installation_data.num_available_sounds == 0 {
        let (closest_installation, _data) =
            closest_installation(current_location, &installations).unwrap();
        if target_installation != closest_installation {
            return true;
        }
    }

    false
}

/// Find and return the installation cloest to the given point.
///
/// Returns `None` if there are no installations in the given map.
fn closest_installation(
    p: Point,
    installations: &InstallationDataMap,
) -> Option<(&Installation, &InstallationData)> {
    let mut iter = installations.iter();
    iter.next()
        .map(|first| {
            let first_dist = Metres(pt2::to_f64(p).distance2(pt2::to_f64(first.1.area.centroid)));
            iter.fold((first.0, first_dist), |(closest, dist), inst| {
                let inst_dist = Metres(pt2::to_f64(p).distance2(pt2::to_f64(inst.1.area.centroid)));
                if inst_dist < dist {
                    (inst.0, inst_dist)
                } else {
                    (closest, dist)
                }
            })
        })
        .map(|(id, _)| (id, &installations[&id]))
}

/// Sort installations by their suitability for use as a target.
///
/// Suitability is based on the following states in order:
///
/// 1. The number of remaining available sounds before the max is reached.
/// 2. The number of sounds needed to reach the minimum required sounds for an installation.
/// 3. The number of sounds needed to reach the target.
fn installation_suitability_order(a: &InstallationData, b: &InstallationData) -> cmp::Ordering {
    // Check that there are sounds available.
    match (a.num_available_sounds, b.num_available_sounds) {
        (_, 0) => return cmp::Ordering::Less,
        (0, _) => return cmp::Ordering::Greater,
        _ => (),
    }

    // The more sounds are needed, the higher the priority.
    match b.num_sounds_needed.cmp(&a.num_sounds_needed) {
        cmp::Ordering::Equal => (),
        cmp => return cmp,
    }

    // The more sounds needed to reach the target, the higher the priority.
    b.num_sounds_needed_to_reach_target
        .cmp(&a.num_sounds_needed_to_reach_target)
}

/// Generate a new target within one of the given installations.
fn generate_target<R>(mut rng: R, installations: &InstallationDataMap) -> Point
where
    R: Rng,
{
    // Collect references to installation data into a `Vec` that we can sort by target suitability.
    let mut vec: Vec<_> = installations.values().collect();
    vec.sort_by(|a, b| installation_suitability_order(a, b));

    // Randomly select one of the installations from the front of the vec.
    let index = (rng.gen::<f32>().powi(4) * vec.len() as f32) as usize;
    let data = &vec[index];
    generate_installation_target(rng, &data.area)
}

/// Generate a target location within the given installation.
fn generate_installation_target<R>(mut rng: R, installation_area: &super::Area) -> Point
where
    R: Rng,
{
    let x_len = installation_area.bounding_rect.right - installation_area.bounding_rect.left;
    let y_len = installation_area.bounding_rect.top - installation_area.bounding_rect.bottom;
    let x = installation_area.bounding_rect.left + x_len * rng.gen::<f64>();
    let y = installation_area.bounding_rect.bottom + y_len * rng.gen::<f64>();
    Point2 { x, y }
}

/// Whether or not the current point has reached the target.
fn reached_target(current: Point, target: Point) -> bool {
    let distance = Metres(pt2::to_f64(current).distance(pt2::to_f64(target)));
    distance <= TARGET_DISTANCE_THRESHOLD
}

/// The desired velocity is the velocity that would take the agent from its `current` position
/// directly to the `target` position.
fn desired_velocity(current: Point, target: Point) -> Vector {
    vt2::to_metres(pt2::to_f64(target) - pt2::to_f64(current))
}

/// The steering vector is the target velocity minus the current velocity.
///
/// - `current_velocity` is the rate at which the agent is currently moving.
/// - `target_velocity` is the vector that would take the agent from its current location directly
///   to the target location.
///
/// The resulting vector is a force that may be applied to the current velocity to steer it
/// directly towards the target location.
fn steering_force(current_velocity: Vector, target_velocity: Vector) -> Vector {
    vt2::to_metres(vt2::to_f64(target_velocity) - vt2::to_f64(current_velocity))
}

/// Limit the magnitude of the given vector.
fn limit_magnitude(v: Vector, limit: f64) -> Vector {
    let vf = vt2::to_f64(v);
    let magnitude = vf.magnitude();
    let f = if magnitude > limit {
        vf.normalize() * limit
    } else {
        vf
    };
    vt2::to_metres(f)
}

/// Produces a force that steers an agent toward its desired target.
fn seek_force(
    current_position: Point,
    target_position: Point,
    current_velocity: Vector,
    max_speed: f64,
    max_force: f64,
) -> Vector {
    let desired_velocity = desired_velocity(current_position, target_position);
    let desired_normalised = vt2::to_f64(desired_velocity).normalize();
    let desired = desired_normalised * max_speed;
    let steering_force = steering_force(current_velocity, vt2::to_metres(desired));
    let steering_limited = limit_magnitude(steering_force, max_force);
    steering_limited
}
