// use epoll_reactor::{EpollReactor, ReactorListener, Interest, EventType, ConnectionResult};
use epoll_reactor::*;
use std::io::prelude::*;
use std::os::unix::io::{AsRawFd, RawFd};
use std::net::{TcpStream, SocketAddr};
use std::{cell::RefCell, rc::Rc};

struct EchoConn {
  sock: TcpStream,
  addr: SocketAddr,
}
impl EchoConn {
  fn new(sock: TcpStream, addr: SocketAddr) -> Self { Self{sock:sock, addr:addr} }
}
impl ReactorListener for EchoConn {
  fn get_interest(&self) -> Interest { println!("interest: {:?}", Interest::READ_WRITE); Interest::READ_WRITE }
  fn get_fd(&self) -> RawFd {
    self.sock.as_raw_fd()
  }
  fn on_event(&mut self, reactor: &mut EpollReactor, event: EventType) {
    match event {
      EventType::Readable => {
        let mut buf : [u8; 1<<13] = [0; 1<<13];
        let bytes = self.sock.read(&mut buf).unwrap();
        if 0 == bytes {
          println!("{} disconnected", self.addr);
          reactor.remove_listener_by_fd(self.get_fd() as u64).unwrap();
        }
        let nl = '\n' as u8;
        // if let Some(offset) = buf.into_iter().position(|&c| c == nl) {
        if let Some(offset) = buf.iter().position(|&c| c == nl) {
          println!("found new line at {}", offset);
          self.sock.write(&buf[0..offset]).unwrap();
        }
      }
      _ => { panic!("unexpected call"); }
    }
  }
}

fn main() {
  // let listener = AcceptListener::new("127.0.0.1:8086",
  //   Box::new(|res| {
  //     match res {
  //       ConnectionResult::Connected(sock, addr) => {
  //         println!("Connection request from {}", addr);
  //         Some(Rc::new(RefCell::new(EchoConn::new(sock, addr))))
  //         // None
  //       }
  //       _ => panic!("unexpected result"),
  //     }
  //   }));
  let mut reactor = EpollReactor::new().unwrap();
  // reactor.register_listener(Rc::new(RefCell::new(listener))).unwrap();
  reactor.listen("127.0.0.1:8086",
    Box::new(|res| {
      match res {
        ConnectionResult::Connected(sock, addr) => {
          println!("Connection request from {}", addr);
          Some(Rc::new(RefCell::new(EchoConn::new(sock, addr))))
        }
        _ => panic!("unexpected result"),
      }
    })).unwrap();
  // reactor.connect("127.0.0.1:8086",
  //   Box::new(|res| {
  //   })).unwrap();
  loop {
    reactor.poll(1000).unwrap();
  }
}
