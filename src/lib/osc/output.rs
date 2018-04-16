use fxhash::FxHashMap;
use installation;
use nannou::osc;
use nannou::osc::Type::{Float, Int};
use std;
use std::iter::once;
use std::sync::mpsc;

/// Messages that can be received by the `osc::output` thread.
pub enum Message {
    Audio(installation::Id, AudioFrameData),
    Osc(OscTarget),
}

/// Add or remove an OSC target for a given installation.
pub enum OscTarget {
    Add(
        installation::Id,
        installation::computer::Id,
        osc::Sender<osc::Connected>,
        String,
    ),
    Remove(installation::Id, installation::computer::Id),
}

/// Data related to a single frame of audio.
#[derive(Debug, Default)]
pub struct AudioFrameData {
    pub avg_peak: f32,
    pub avg_rms: f32,
    pub avg_fft: FftData,
    pub speakers: Vec<Speaker>,
}

/// Basic FFT audio analysis results.
#[derive(Debug, Default)]
pub struct FftData {
    /// Low, mid and high bands.
    pub lmh: [f32; 3],
    /// More detailed 8-bin data.
    pub bins: [f32; 8],
}

/// Data related to a single audio channel.
#[derive(Debug)]
pub struct Speaker {
    pub peak: f32,
    pub rms: f32,
}

/// The log of a sent message.
#[derive(Debug)]
pub struct Log {
    pub installation: installation::Id,
    pub computer: installation::computer::Id,
    pub addr: std::net::SocketAddr,
    pub msg: osc::Message,
    pub error: Option<osc::CommunicationError>,
}

/// Spawn the osc sender thread.
pub fn spawn() -> (
    std::thread::JoinHandle<()>,
    mpsc::Sender<Message>,
    mpsc::Receiver<Log>,
) {
    let (msg_tx, msg_rx) = mpsc::channel();
    let (log_tx, log_rx) = mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("osc_out".into())
        .spawn(move || {
            run(msg_rx, log_tx);
        })
        .unwrap();
    (handle, msg_tx, log_rx)
}

fn run(msg_rx: mpsc::Receiver<Message>, log_tx: mpsc::Sender<Log>) {
    struct Target {
        osc_tx: osc::Sender<osc::Connected>,
        osc_addr: String,
    }

    enum Update {
        Msg(Message),
        SendOsc,
    }

    // Each installation gets its own map of installation::computer::Id -> Target.
    type TargetMap = FxHashMap<installation::computer::Id, Target>;
    let mut osc_txs: FxHashMap<installation::Id, TargetMap> = Default::default();

    // Update channel.
    let (update_tx, update_rx) = mpsc::channel();

    // Start a timer thread for triggering OSC output 60 times per second.
    let update_tx_2 = update_tx.clone();
    std::thread::Builder::new()
        .name("osc_output_timer".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(16));
            if update_tx_2.send(Update::SendOsc).is_err() {
                break;
            }
        })
        .unwrap();

    // Start a thread for converting `Message`s to `Update`s.
    std::thread::Builder::new()
        .name("osc_output_msg_to_update".into())
        .spawn(move || {
            for msg in msg_rx {
                if update_tx.send(Update::Msg(msg)).is_err() {
                    break;
                }
            }
        })
        .unwrap();

    // A map containing the latest data received in terms of messages.
    let mut last_received = FxHashMap::default();
    let mut last_sent = FxHashMap::default();
    for update in update_rx {
        match update {
            Update::Msg(msg) => match msg {
                // Audio data received that is to be delivered to the given installation.
                Message::Audio(installation, data) => {
                    last_received.insert(installation, data);
                }
                // Some OSC target should be added or removed.
                Message::Osc(osc) => match osc {
                    OscTarget::Add(installation, computer, osc_tx, osc_addr) => {
                        osc_txs
                            .entry(installation)
                            .or_insert_with(FxHashMap::default)
                            .insert(computer, Target { osc_tx, osc_addr });
                    }
                    OscTarget::Remove(installation, computer) => {
                        if let Some(txs) = osc_txs.get_mut(&installation) {
                            txs.remove(&computer);
                        }
                    }
                },
            },
            Update::SendOsc => for (installation, data) in last_received.drain() {
                let AudioFrameData {
                    avg_peak,
                    avg_rms,
                    avg_fft,
                    speakers,
                } = data;

                let targets = match osc_txs.get(&installation) {
                    Some(targets) => targets,
                    None => continue,
                };

                // The buffer used to collect arguments.
                let mut args = Vec::new();

                // Push the analysis of the averaged channels.
                args.push(Float(avg_peak));
                args.push(Float(avg_rms));
                let lmh = avg_fft.lmh.iter().map(|&f| Float(f));
                args.extend(lmh);
                let bins = avg_fft.bins.iter().map(|&f| Float(f));
                args.extend(bins);

                // Push the Peak and RMS per speaker.
                let speakers = speakers.into_iter().enumerate().flat_map(|(i, s)| {
                    once(Int(i as _))
                        .chain(once(Float(s.peak)))
                        .chain(once(Float(s.rms)))
                });
                args.extend(speakers);

                // Retrieve the OSC sender for each computer in the installation.
                for target in targets.iter() {
                    let (
                        &computer,
                        &Target {
                            ref osc_tx,
                            ref osc_addr,
                        },
                    ) = target;
                    let addr = &osc_addr[..];

                    // Send the message!
                    let msg = osc::Message {
                        addr: addr.into(),
                        args: Some(args.clone()),
                    };

                    // If the message is the same as the last one we sent for this computer, don't
                    // bother sending it again.
                    if last_sent.get(&(installation, computer)) == Some(&msg) {
                        continue;
                    }

                    // Send the OSC.
                    let error = osc_tx.send(msg.clone()).err();

                    // Update the `last_sent` map if there were no errors.
                    if error.is_none() {
                        last_sent.insert((installation, computer), msg.clone());
                    }

                    // Log the message for displaying in the GUI.
                    let addr = osc_tx.remote_addr();
                    let mut log = Log {
                        installation,
                        computer,
                        addr,
                        msg,
                        error,
                    };
                    log_tx.send(log).ok();
                }
            },
        }
    }
}
