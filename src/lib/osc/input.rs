use nannou::osc;
use nannou::osc::Type::Float;
use std;
use std::net::SocketAddr;
use std::sync::mpsc;

const BEYOND_PERCEPTION_ADDR: &'static str = "/bp";
const SOURCE_VOLUME_ADDR: &'static str = "/source_volume";
const MASTER_VOLUME_ADDR: &'static str = "/master_volume";
const PLAY_SOUNDSCAPE: &'static str = "/play_soundscape";
const PAUSE_SOUNDSCAPE: &'static str = "/pause_soundscape";

/// A record of a received message.
#[derive(Debug)]
pub struct Log {
    /// The address from which the message was received.
    pub addr: SocketAddr,
    /// The recevied OSC message.
    pub msg: osc::Message,
}

/// A control message parsed from an OSC input message.
#[derive(Clone, Debug)]
pub enum Control {
    SourceVolume(SourceVolume),
    MasterVolume(MasterVolume),
    PauseSoundscape,
    PlaySoundscape,
}

/// An OSC input message that was parsed as the master volume for the exhibition.
#[derive(Clone, Debug)]
pub struct MasterVolume(pub f32);

/// An OSC input message that was parsed as volume for a source.
///
/// Expects the following OSC message:
///
/// - Address: "/bp/source_volume/<source_name>"
/// - Arguments: `Float` where `String` is the source name and `Float` is the volume.
#[derive(Clone, Debug)]
pub struct SourceVolume {
    /// The name of the source to which this will be applied.
    ///
    /// Note that the volume will be applied to the first source whose name matches this, so best
    /// to give a unique name to each source using the source editor.
    pub name: String,
    /// The value that will be assigned to the `audio::Source`'s `volume` field.
    pub volume: f32,
}

impl From<MasterVolume> for Control {
    fn from(mv: MasterVolume) -> Self {
        Control::MasterVolume(mv)
    }
}

impl From<SourceVolume> for Control {
    fn from(sv: SourceVolume) -> Self {
        Control::SourceVolume(sv)
    }
}

// Finds the "/bp" string and returns the remainder if any.
fn parse_bp(s: &str) -> Option<&str> {
    if s.starts_with(BEYOND_PERCEPTION_ADDR) {
        Some(&s[BEYOND_PERCEPTION_ADDR.len()..])
    } else {
        None
    }
}

// Finds the "/source_volume" string and returns
fn parse_source_volume(s: &str) -> Option<&str> {
    if s.starts_with(SOURCE_VOLUME_ADDR) {
        Some(&s[SOURCE_VOLUME_ADDR.len()..])
    } else {
        None
    }
}

// Finds the "/master_volume" string. Returns `true` if found.
fn parse_master_volume(s: &str) -> bool {
    s == MASTER_VOLUME_ADDR
}

// Finds the "/play_soundscape" string. Returns `true` if found.
fn parse_play_soundscape(s: &str) -> bool {
    s == PLAY_SOUNDSCAPE
}

// Finds the "/pause_soundscape" string. Returns `true` if found.
fn parse_pause_soundscape(s: &str) -> bool {
    s == PAUSE_SOUNDSCAPE
}

impl Control {
    fn from_osc_msg(msg: &osc::Message) -> Option<Self> {
        parse_bp(&msg.addr)
            .and_then(|s| {
                match (parse_master_volume(s), msg.args.as_ref().and_then(|args| args.get(0))) {
                    (true, Some(&Float(vol))) => {
                        let master_volume = MasterVolume(vol.min(1.0).max(0.0));
                        return Some(master_volume.into())
                    }
                    _ => (),
                }

                match (parse_source_volume(s), msg.args.as_ref().and_then(|args| args.get(0))) {
                    (Some(name), Some(&Float(volume))) => {
                        let name = name.into();
                        let source_volume = SourceVolume { name, volume };
                        return Some(source_volume.into())
                    }
                    _ => (),
                }

                if parse_play_soundscape(s) {
                    return Some(Control::PlaySoundscape);
                }

                if parse_pause_soundscape(s) {
                    return Some(Control::PauseSoundscape);
                }

                None
            })
    }
}

/// Spawn the OSC receiver thread.
pub fn spawn(
    osc_rx: osc::Receiver,
) -> (
    std::thread::JoinHandle<()>,
    mpsc::Receiver<Log>,
    mpsc::Receiver<Control>,
) {
    let (log_tx, log_rx) = mpsc::channel();
    let (control_tx, control_rx) = mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("osc_in".into())
        .spawn(move || run(osc_rx, log_tx, control_tx))
        .unwrap();
    (handle, log_rx, control_rx)
}

// The function that is run on the osc_input thread.
fn run(
    osc_rx: osc::Receiver,
    log_tx: mpsc::Sender<Log>,
    control_tx: mpsc::Sender<Control>,
) {
    loop {
        // Block until we get the next packet.
        let (packet, addr) = match osc_rx.recv() {
            Ok(ok) => ok,
            Err(e) => {
                println!("Error while receiving OSC: {}", e);
                break;
            }
        };

        // Unfold the packet into its messages.
        for message in packet.into_msgs() {
            // Forward messages to GUI thread for displaying in the log.
            let log = Log {
                addr: addr.clone(),
                msg: message.clone(),
            };
            log_tx.send(log).ok();

            // OSC -> Control
            if let Some(control) = Control::from_osc_msg(&message) {
                control_tx.send(control).ok();
            }
        }
    }
}
