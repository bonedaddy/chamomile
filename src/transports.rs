use chamomile_types::types::{PeerId, TransportType, PEER_ID_LENGTH};
use smol::{
    channel::{self, Receiver, Sender},
    io::Result,
};
use std::net::SocketAddr;

mod rtp;
mod tcp;
//mod udp;
mod udt;

use crate::hole_punching::{Hole, DHT};
use crate::keys::Keypair;
use crate::peer::{Peer, PEER_LENGTH};

/// new a channel for send TransportSendMessage.
pub fn new_transport_send_channel() -> (Sender<TransportSendMessage>, Receiver<TransportSendMessage>)
{
    channel::unbounded()
}

/// new a channel for receive EndpointIncomingMessage.
pub fn new_transport_recv_channel() -> (Sender<TransportRecvMessage>, Receiver<TransportRecvMessage>)
{
    channel::unbounded()
}

/// new a channel for EndpointSendMessage between in session's and transport stream.
pub fn new_endpoint_channel() -> (Sender<EndpointMessage>, Receiver<EndpointMessage>) {
    channel::unbounded()
}

/// Endpoint can receied this message channel.
pub enum TransportSendMessage {
    /// connect to a socket address.
    /// params is `socket_addr`, `remote_pk bytes`.
    Connect(SocketAddr, RemotePublic),
    /// params is `delivery_id`, `socket_addr`, `remote_pk bytes`.
    StableConnect(
        Sender<EndpointMessage>,
        Receiver<EndpointMessage>,
        SocketAddr,
        RemotePublic,
    ),
}

/// when endpoint get a incoming connection, will send to outside.
/// params: `socket_addr`, `endpoint_stream_receiver`,
/// `endpoint_stream_sender` and `is_stable`, `remote_pk bytes`.
pub struct TransportRecvMessage(
    pub SocketAddr,                // remote addr.
    pub RemotePublic,              // remote public info.
    pub bool,                      // is send by self.
    pub Sender<EndpointMessage>,   // session's endpoint sender.
    pub Receiver<EndpointMessage>, // session's endpoint receiver.
    pub Sender<EndpointMessage>,   // transport's receiver.
);

/// Session Endpoint Message.
/// bytes[0] is type, bytes[1..] is data.
pub enum EndpointMessage {
    /// type is 0u8.
    Close,
    /// type is 1u8.
    Handshake(RemotePublic),
    /// type is 2u8.
    DHT(DHT),
    /// type is 3u8.
    Hole(Hole),
    /// type is 4u8.
    HoleConnect,
    /// type is 5u8. encrypted's CoreData.
    Data(Vec<u8>),
    /// type is 6u8. encrypted's CoreData.
    RelayData(PeerId, PeerId, Vec<u8>),
    /// type is 7u8. encrypted's ConnectData.
    RelayConnect(RemotePublic, PeerId),
}

/// main function. start the endpoint listening.
pub async fn start(
    transport: &TransportType,
    addr: SocketAddr,
) -> Result<(Sender<TransportSendMessage>, Receiver<TransportRecvMessage>)> {
    let (send_send, send_recv) = new_transport_send_channel();
    let (recv_send, recv_recv) = new_transport_recv_channel();

    match transport {
        //&TransportType::UDP => udp::UdpEndpoint::start(addr, recv_send, send_recv).await?,
        &TransportType::TCP => tcp::start(addr, recv_send, send_recv).await?,
        _ => panic!("Not suppert, waiting"),
    }

    Ok((send_send, recv_recv))
}

/// Rtemote Public Info, include local transport and public key bytes.
#[derive(Clone)]
pub struct RemotePublic(pub Keypair, pub Peer);

impl RemotePublic {
    pub fn id(&self) -> &PeerId {
        self.1.id()
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < PEER_LENGTH {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "serialize remote public error",
            ));
        }
        let peer = Peer::from_bytes(&bytes[..PEER_LENGTH]).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::Other, "serialize remote public error")
        })?;
        let keypair = Keypair::from_bytes(&bytes[PEER_LENGTH..]).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::Other, "serialize remote public error")
        })?;

        Ok(Self(keypair, peer))
    }

    pub fn ref_to_bytes(key: &Keypair, peer: &Peer) -> Vec<u8> {
        let mut bytes = vec![];
        bytes.append(&mut peer.to_bytes());
        bytes.append(&mut key.to_bytes());
        bytes
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = vec![];
        bytes.append(&mut self.1.to_bytes());
        bytes.append(&mut self.0.to_bytes());
        bytes
    }
}

impl EndpointMessage {
    pub fn to_bytes(self) -> Vec<u8> {
        let mut bytes = vec![0u8];
        match self {
            EndpointMessage::Close => {
                bytes[0] = 0u8;
            }
            EndpointMessage::Handshake(peer) => {
                bytes[0] = 1u8;
                let mut peer_bytes = peer.to_bytes();
                bytes.extend(&(peer_bytes.len() as u32).to_be_bytes()[..]);
                bytes.append(&mut peer_bytes);
            }
            EndpointMessage::DHT(dht) => {
                bytes[0] = 2u8;
                bytes.append(&mut dht.to_bytes());
            }
            EndpointMessage::Hole(hole) => {
                bytes[0] = 3u8;
                bytes.push(hole.to_byte());
            }
            EndpointMessage::HoleConnect => {
                bytes[0] = 4u8;
            }
            EndpointMessage::Data(mut data) => {
                bytes[0] = 5u8;
                bytes.append(&mut data);
            }
            EndpointMessage::RelayData(p1_id, p2_id, mut data) => {
                bytes[0] = 6u8;
                bytes.append(&mut p1_id.to_bytes());
                bytes.append(&mut p2_id.to_bytes());
                bytes.append(&mut data);
            }
            EndpointMessage::RelayConnect(p1_peer, p2_id) => {
                bytes[0] = 7u8;
                let mut peer_bytes = p1_peer.to_bytes();
                bytes.extend(&(peer_bytes.len() as u32).to_be_bytes()[..]);
                bytes.append(&mut peer_bytes);
                bytes.append(&mut p2_id.to_bytes());
            }
        }

        bytes
    }

    fn from_bytes(mut bytes: Vec<u8>) -> std::result::Result<Self, ()> {
        if bytes.len() < 1 {
            return Err(());
        }

        let t: Vec<u8> = bytes.drain(0..1).collect();
        match t[0] {
            0u8 => Ok(EndpointMessage::Close),
            1u8 => {
                if bytes.len() < 4 {
                    return Err(());
                }
                let mut peer_len_bytes = [0u8; 4];
                peer_len_bytes.copy_from_slice(bytes.drain(0..4).as_slice());
                let peer_len = u32::from_be_bytes(peer_len_bytes) as usize;
                if bytes.len() < peer_len {
                    return Err(());
                }
                let peer =
                    RemotePublic::from_bytes(bytes.drain(0..peer_len).collect()).map_err(|_| ())?;
                Ok(EndpointMessage::Handshake(peer))
            }
            2u8 => {
                let dht = DHT::from_bytes(&bytes)?;
                Ok(EndpointMessage::DHT(dht))
            }
            3u8 => {
                if bytes.len() != 1 {
                    return Err(());
                }
                let hole = Hole::from_byte(bytes[0])?;
                Ok(EndpointMessage::Hole(hole))
            }
            4u8 => Ok(EndpointMessage::HoleConnect),
            5u8 => Ok(EndpointMessage::Data(bytes)),
            6u8 => {
                if bytes.len() < PEER_ID_LENGTH * 2 {
                    return Err(());
                }
                let p1 = PeerId::from_bytes(&bytes.drain(0..PEER_ID_LENGTH).as_slice())?;
                let p2 = PeerId::from_bytes(&bytes.drain(0..PEER_ID_LENGTH).as_slice())?;
                Ok(EndpointMessage::RelayData(p1, p2, bytes))
            }
            7u8 => {
                if bytes.len() < 4 {
                    return Err(());
                }
                let mut peer_len_bytes = [0u8; 4];
                peer_len_bytes.copy_from_slice(bytes.drain(0..4).as_slice());
                let peer_len = u32::from_be_bytes(peer_len_bytes) as usize;
                if bytes.len() < peer_len + PEER_ID_LENGTH {
                    return Err(());
                }
                let peer =
                    RemotePublic::from_bytes(bytes.drain(0..peer_len).collect()).map_err(|_| ())?;
                let p2 = PeerId::from_bytes(&bytes.drain(0..PEER_ID_LENGTH).as_slice())?;
                Ok(EndpointMessage::RelayConnect(peer, p2))
            }
            _ => Err(()),
        }
    }
}
