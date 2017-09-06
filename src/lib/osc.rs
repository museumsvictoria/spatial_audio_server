
pub mod input {
    use interaction::Interaction;
    use rosc::{self, OscMessage, OscPacket};
    use std;
    use std::net::{UdpSocket, SocketAddr, SocketAddrV4};
    use std::sync::mpsc;

    /// Spawn the OSC receiver thread.
    pub fn spawn(addr: SocketAddrV4)
        -> (mpsc::Receiver<(SocketAddr, OscMessage)>, mpsc::Receiver<Interaction>)
    {
        let (msg_tx, msg_rx) = mpsc::channel();
        let (interaction_tx, interaction_rx) = mpsc::channel();

        std::thread::Builder::new()
            .name("osc_in".into())
            .spawn(move || {

                let socket = UdpSocket::bind(addr).unwrap();
                println!("Listening to {}", addr);

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
                    //
                    // No worries if the GUI channel is closed, the interaction channel is the one
                    // we care about most.
                    msg_tx.send((addr, message)).ok();

                    // TODO: OSC -> Interaction
                }

            })
            .unwrap();

        (msg_rx, interaction_rx)
    }
}

pub mod output {
}
