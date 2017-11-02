use nannou::osc;
use nannou::osc::Type::{Float, Int};

/// Describes the various interactions that are listened for within the installation.
///
/// `Interaction`s are received on the audio_server via OSC. Each interaction-related OSC message
/// address should be prefixed with "bp" (for Beyond Perception) and then the abbreviation of the
/// relevant installation name. E.g.
///
/// - "/bp/cw" for Cosmic Wave interactions.
/// - "/bp/te" for Turbulent Encounters interactions.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Interaction {
    CosmicWave(CosmicWave),
    TurbulentEncounters(TurbulentEncounters),
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum CosmicWave {
    /// Interactions with the touch surface from the perspective of looking directly at the
    /// projection surface.
    ///
    /// Each of the scalar values are represented in the range of `0.0` to `1.0` where `0.0` is the
    /// minimum of the range and `1.0` is the maximum of the range.
    Touch {
        /// The ID of the material surface that has been touched.
        screen_id: i32,
        /// The ID associated with this continuous touch.
        id: i32,
        /// The horizontal plane.
        x: f32,
        /// The vertical plane.
        y: f32,
        /// The depth plane.
        z: f32,
    },
    /// Occasionally interactions may cause a reaction of some sort - e.g. `Merger`/`Ringdown`.
    TriggerReaction(Reaction),
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Reaction {}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum TurbulentEncounters {
    /// Each of the rotary encoders for the 
    RotaryEncoder {
        /// The unique identifier associated with the encoder.
        id: i32,
        /// The "amount" the encoder spun.
        ///
        /// `0.0` represents the minimum, `1.0` represents the maximum.
        value: f32,
    },
}


impl From<CosmicWave> for Interaction {
    fn from(cw: CosmicWave) -> Self {
        Interaction::CosmicWave(cw)
    }
}

impl From<TurbulentEncounters> for Interaction {
    fn from(te: TurbulentEncounters) -> Self {
        Interaction::TurbulentEncounters(te)
    }
}


/// Parse the given OSC message as an `Interaction`.
///
/// Returns `None` if the message cannot be parsed.
pub fn from_osc(msg: &osc::Message) -> Option<Interaction> {
    const BP_PREFIX: &'static str = "/bp/";

    // If the address doesn't begin with the bp prefix, ignore it.
    if msg.addr.len() <= BP_PREFIX.len() || BP_PREFIX != &msg.addr[..BP_PREFIX.len()] {
        return None;
    }

    // If there are no arguments there is nothing to be parsed.
    let args = match msg.args {
        Some(ref args) => args,
        None => return None,
    };

    // Match on the remainder of the address and the number of arguments in the message.
    let interaction = match (&msg.addr[BP_PREFIX.len()..], args.len()) {
        // Cosmic Wave Interactions.
        ("cw", 5) => match (&args[0], &args[1], &args[2], &args[3], &args[4]) {
            (&Int(screen_id), &Int(id), &Float(x), &Float(y), &Float(z)) =>
                CosmicWave::Touch { screen_id, id, x, y, z }.into(),
            _ => return None,
        },
        ("cw", 1) => match &args[0] {
            &Int(_reaction_index) => CosmicWave::TriggerReaction(unimplemented!()).into(),
            _ => return None,
        },
        // Turbulent Encounters interactions.
        ("te", 2) => match (&args[0], &args[1]) {
            (&Int(id), &Float(value)) => TurbulentEncounters::RotaryEncoder { id, value }.into(),
            _ => return None,
        },
        _ => return None,
    };

    Some(interaction)
}

#[test]
fn test_from_osc() {
    // Test Cosmic Wave touch.
    let a = osc::Message {
        addr: "/bp/cw".into(),
        args: Some(vec![Int(2), Int(4), Float(0.1), Float(0.2), Float(0.3)]),
    };
    let a_expected = Interaction::CosmicWave(CosmicWave::Touch { screen_id: 2, id: 4, x: 0.1, y: 0.2, z: 0.3 });
    assert_eq!(from_osc(&a), Some(a_expected));

    // Test invalid message == None.
    let b = osc::Message {
        addr: "/bp/cw".into(),
        args: None,
    };
    assert_eq!(from_osc(&b), None);

    // Test Turbulent Encounters Rotary Dial.
    let c = osc::Message {
        addr: "/bp/te".into(),
        args: Some(vec![Int(0), Float(0.66)]),
    };
    let c_expected = Interaction::TurbulentEncounters(TurbulentEncounters::RotaryEncoder { id: 0, value: 0.66 });
    assert_eq!(from_osc(&c), Some(c_expected));
}
