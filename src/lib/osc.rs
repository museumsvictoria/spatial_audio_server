pub mod input {
    use interaction::{self, Interaction};
    use nannou::osc;
    use std;
    use std::net::SocketAddr;
    use std::sync::mpsc;

    /// Spawn the OSC receiver thread.
    pub fn spawn(
        osc_receiver: osc::Receiver,
    ) -> (std::thread::JoinHandle<()>,
              mpsc::Receiver<(SocketAddr, osc::Message)>,
              mpsc::Receiver<Interaction>) {
        let (msg_tx, msg_rx) = mpsc::channel();
        let (interaction_gui_tx, interaction_gui_rx) = mpsc::channel();

        let handle = std::thread::Builder::new()
            .name("osc_in".into())
            .spawn(move || {
                loop {
                    let (packet, addr) = match osc_receiver.recv() {
                        Ok(ok) => ok,
                        Err(e) => {
                            println!("Error while receiving OSC: {}", e);
                            break;
                        }
                    };

                    // Unfold the packet into its messages.
                    for message in packet.into_msgs() {
                        // Forward messages to GUI thread for displaying in the log.
                        msg_tx.send((addr.clone(), message.clone())).ok();

                        // OSC -> Interaction
                        let interaction = match interaction::from_osc(&message) {
                            Some(interaction) => interaction,
                            None => continue,
                        };

                        // Forward interactions to the GUI thread for displaying in the log.
                        interaction_gui_tx.send(interaction).ok();
                    }
                }
            })
            .unwrap();

        (handle, msg_rx, interaction_gui_rx)
    }
}

pub mod output {}
