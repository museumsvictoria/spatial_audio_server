pub mod input {
    use interaction::{self, Interaction};
    use rosc::{self, OscMessage, OscPacket};
    use std;
    use std::net::{UdpSocket, SocketAddr, SocketAddrV4};
    use std::sync::mpsc;

    /// Spawn the OSC receiver thread.
    pub fn spawn(
        addr: SocketAddrV4,
    ) -> (std::thread::JoinHandle<()>,
              mpsc::Receiver<(SocketAddr, OscMessage)>,
              mpsc::Receiver<Interaction>) {
        let (msg_tx, msg_rx) = mpsc::channel();
        let (interaction_gui_tx, interaction_gui_rx) = mpsc::channel();

        let handle = std::thread::Builder::new()
            .name("osc_in".into())
            .spawn(move || {
                let socket = UdpSocket::bind(addr).unwrap();
                let mut buffer = [0u8; rosc::decoder::MTU];
                loop {
                    let (size, addr) = match socket.recv_from(&mut buffer) {
                        Ok(ok) => ok,
                        Err(e) => {
                            println!("Error receiving from socket: {}", e);
                            break;
                        }
                    };

                    // Decode the OSC packet.
                    let packet = rosc::decoder::decode(&buffer[..size]).unwrap();

                    // We're expecting a single `OscMessage` per packet.
                    let message = match packet {
                        OscPacket::Message(msg) => msg,
                        OscPacket::Bundle(mut bundle) => {
                            // Just check for the first message if it's a bundle.
                            match bundle.content.swap_remove(0) {
                                OscPacket::Message(msg) => msg,
                                packet => panic!("unexpected OscPacket: {:?}", packet),
                            }
                        }
                    };

                    // Forward messages to GUI thread for displaying in the log.
                    msg_tx.send((addr, message.clone())).ok();

                    // OSC -> Interaction
                    let interaction = match interaction::from_osc(&message) {
                        Some(interaction) => interaction,
                        None => continue,
                    };

                    // Forward interactions to the GUI thread for displaying in the log.
                    interaction_gui_tx.send(interaction).ok();
                }
            })
            .unwrap();

        (handle, msg_rx, interaction_gui_rx)
    }
}

pub mod output {}
