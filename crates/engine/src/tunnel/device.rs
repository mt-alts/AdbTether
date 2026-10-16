use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use wintun::Session;

pub struct WintunDevice {
    session: Arc<Session>,
    rx_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
}

pub struct WintunRxToken {
    pkt: Vec<u8>,
}

pub struct WintunTxToken {
    session: Arc<Session>,
}

impl RxToken for WintunRxToken {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.pkt)
    }
}

impl TxToken for WintunTxToken {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut pkt = match self.session.allocate_send_packet(len as u16) {
            Ok(p) => p,
            Err(_) => {
                let mut tmp = vec![0u8; len];
                return f(&mut tmp);
            }
        };
        let r = f(pkt.bytes_mut());
        let _ = self.session.send_packet(pkt);
        r
    }
}

impl Device for WintunDevice {
    type RxToken<'a> = WintunRxToken;
    type TxToken<'a> = WintunTxToken;

    fn receive(&mut self, _ts: Instant) -> Option<(WintunRxToken, WintunTxToken)> {
        let pkt = self.rx_queue.lock().unwrap().pop_front();
        match pkt {
            Some(p) => Some((
                WintunRxToken { pkt: p },
                WintunTxToken {
                    session: self.session.clone(),
                },
            )),
            None => None,
        }
    }

    fn transmit(&mut self, _ts: Instant) -> Option<WintunTxToken> {
        Some(WintunTxToken {
            session: self.session.clone(),
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = 1500;
        caps
    }
}

impl WintunDevice {
    pub fn new(session: Arc<Session>) -> (Self, Arc<Mutex<VecDeque<Vec<u8>>>>) {
        let rx_queue = Arc::new(Mutex::new(VecDeque::new()));
        (
            Self {
                session: session.clone(),
                rx_queue: rx_queue.clone(),
            },
            rx_queue,
        )
    }
}
