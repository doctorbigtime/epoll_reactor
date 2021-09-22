extern crate libc;
use std::os::unix::io::{AsRawFd, RawFd};
use std::io;
// use std::io::prelude::*;
use std::collections::{HashMap};
use std::net::{TcpListener, TcpStream, SocketAddr};
use std::{cell::RefCell, rc::Rc};
// use std::num::NonZeroU8;

pub enum EventType {
  Readable,
  Writable,
  // Disconnect,
  Timeout,
}

#[derive(Debug)]
pub struct Interest(i32);
const READ : i32 = libc::EPOLLIN;
const WRITE : i32 = libc::EPOLLOUT;
impl Interest {
  pub const READ: Interest = Interest(READ);
  pub const WRITE: Interest = Interest(WRITE);
  pub const READ_WRITE: Interest = Interest(WRITE|READ);
}

pub trait ReactorListener {
  fn on_event(&mut self, ctrl: &mut EpollReactor, event: EventType);
  fn get_fd(&self) -> RawFd;
  fn get_interest(&self) -> Interest;
}

pub trait IdleCallback {
  fn on_idle(&mut self);
}

pub trait TimerCallback {
  fn on_timer(&mut self, timer: u32);
}

struct ReactorBlock {
  pub fd: RawFd,
  pub listener: Rc<RefCell<dyn ReactorListener>>,
}
impl ReactorBlock {
  pub fn new(listener: Rc<RefCell<dyn ReactorListener>>, fd: RawFd) -> Self {
    Self{
      fd: fd,
      listener: listener,
    }
  }
}

pub struct ReactorState {
  control_blocks: HashMap<u64, ReactorBlock>,
  idle_callbacks: Vec<Box<dyn IdleCallback>>,
}

pub struct EpollReactor {
  pub efd: RawFd,
  state: Option<ReactorState>,
  // control_blocks: HashMap<u64, ReactorBlock>,
}

impl EpollReactor {
  pub fn new() -> io::Result<Self> {
    unsafe {
      let epoll_fd = libc::epoll_create(1);
      if epoll_fd < 0 {
        return Err(std::io::Error::last_os_error());
      }
      return Ok(EpollReactor{
        efd: epoll_fd,
        state: Some(ReactorState{control_blocks:HashMap::new(), idle_callbacks: Vec::new()}),
        // control_blocks: HashMap::new(),
      });
    }
  }

  /// adds a listener to dispatch map.
  /// * `listener`: the listener trait, the Rc gets moved into an internal hash map.
  pub fn register_listener(&mut self, listener: Rc<RefCell<dyn ReactorListener>>) -> io::Result<()> {
    let fd = listener.borrow().get_fd();
    let interest = listener.borrow().get_interest();
    // self.control_blocks.insert(fd as u64, ReactorBlock::new(listener, fd));
    let state = self.state.as_mut().unwrap();
    println!("adding listener for fd {}", fd);
    state.control_blocks.insert(fd as u64, ReactorBlock::new(listener, fd));
    add_interest(self.efd, fd, interest).unwrap();
    Ok(())
  }

  pub fn register_idle_callback(&mut self, callback: Box<dyn IdleCallback>) {
    let state = self.state.as_mut().unwrap();
    state.idle_callbacks.push(callback);
  }

  /// removes a listener.
  // pub fn remove_listener(&mut self, listener: Rc<dyn ReactorListener>) -> io::Result<()> {
  //   let fd = listener.get_fd();
  //   return self.remove_listener_by_fd(fd as u64);
  // }

  /// removes a listener by file descriptor.
  pub fn remove_listener_by_fd(&mut self, fd: u64) -> io::Result<()> {
    remove_interest(self.efd, fd as i32)?;
    let state = self.state.as_mut().unwrap();
    state.control_blocks.remove(&fd);
    Ok(())
  }

  pub fn change_interest_by_fd(&mut self, fd: u64, interest: Interest) -> io::Result<()> {
    change_interest(self.efd, fd as i32, interest)?;
    Ok(())
  }

  pub fn listen(&mut self, addr: &str, handler: Box<ConnectionHandler>) -> io::Result<()> {
    let listener = AcceptListener::new(addr, handler);
    self.register_listener(Rc::new(RefCell::new(listener)))?;
    Ok(())
  }

  pub fn connect(&mut self, addr: &str, handler: Box<ConnectionHandler>) -> io::Result<()> {
    let connector = ConnectListener::new(addr, handler);
    self.register_listener(Rc::new(RefCell::new(connector)))?;
    Ok(())
  }

  /// runs epoll_wait, triggers callbacks for registered listeners.
  /// * `timeout_ms`: number of milliseconds to wait for before timeout.
  pub fn poll(&mut self, timeout_ms: i32) -> io::Result<()> {
    let timeout : libc::c_int = timeout_ms;
    let mut events : Vec<libc::epoll_event> = Vec::with_capacity(1024);
    let nfds = do_epoll_wait(self.efd, &mut events, timeout)?;
    if nfds == 0 {
      println!("timeout");
      let state = self.state.as_mut().unwrap();
      for callback in &mut state.idle_callbacks {
        callback.on_idle();
      }
      return Ok(());
    }
    unsafe {
      events.set_len(nfds as usize);
      for (_i, ee) in events.iter().enumerate() {
        // println!("ee[{}]: events={} u64={}", i, ee.events, ee.u64);
        // let mut state = self.state.as_mut().unwrap();
        let state = self.state.as_mut().unwrap();
        if let Some(block) = state.control_blocks.get(&ee.u64) {
          let listener = Rc::clone(&block.listener);
          let events = ee.events as i32;
          if events & libc::EPOLLIN == libc::EPOLLIN {
            println!("firing listener on_readable");
            // let mut listener = &mut *block.listener;
            listener.borrow_mut().on_event(self, EventType::Readable);
          }
        }
      }
    }
    Ok(())
  }

  pub fn register_timer(&mut self, _callback: Box<dyn TimerCallback>) {
    // let state = self.state.as_mut().unwrap();
    // state.timer_queue.register(next_time, period, end_time, callback);
  }
}

pub enum ConnectionResult {
  Connected(TcpStream, SocketAddr),
  Failed(std::io::Error),
}

pub type Listener = Rc<RefCell<dyn ReactorListener>>;
pub type ConnectionHandler = dyn FnMut(ConnectionResult) -> Option<Rc<RefCell<dyn ReactorListener>>>;

pub struct AcceptListener {
  listener: TcpListener,
  handler: Box<ConnectionHandler>,
}
impl AcceptListener {
  pub fn new(addr: &str, handler: Box<ConnectionHandler>) -> Self {
    let listener = TcpListener::bind(addr).unwrap();
    listener.set_nonblocking(true).unwrap();
    Self {
      listener: listener,
      handler: handler,
    }
  }
}
impl ReactorListener for AcceptListener {
  fn get_fd(&self) -> RawFd {
    self.listener.as_raw_fd()
  }
  fn get_interest(&self) -> Interest { return Interest::READ; }
  fn on_event(&mut self, reactor: &mut EpollReactor, event: EventType) {
    match event {
      EventType::Readable => {
        match self.listener.accept() {
          Ok((stream, addr)) => {
            println!("Acceptor new client {}", addr);
            if let Some(sock) = (self.handler)(ConnectionResult::Connected(stream, addr)) {
              println!("It returned a new socket");
              reactor.register_listener(sock).unwrap();
            } else {
              println!("nothing to see here");
            }
          }
          Err(e) => { eprintln!("Could not accept: {}", e); }
        }
      }
      _ => panic!("Unexpected event type"),
    }
  }
}

pub struct ConnectListener {
  sock: TcpStream,
  addr: SocketAddr,
  handler: Box<ConnectionHandler>,
}
impl ConnectListener {
  pub fn new(addr: &str, handler: Box<ConnectionHandler>) -> Self {
    let addr = addr.parse().unwrap();
    let sock = TcpStream::connect(&addr).unwrap();
    sock.set_nonblocking(true).unwrap();
    Self {
      sock: sock,
      addr: addr,
      handler: handler,
    }
  }
}
impl ReactorListener for ConnectListener {
  fn get_fd(&self) -> RawFd {
    self.sock.as_raw_fd()
  }
  fn get_interest(&self) -> Interest { return Interest::WRITE; }
  fn on_event(&mut self, reactor: &mut EpollReactor, event: EventType) {
    match event {
      EventType::Writable => {
        println!("Connector connected to {}", self.addr); 
        let stream = self.sock.try_clone().unwrap();
        if let Some(sock) = (self.handler)(ConnectionResult::Connected(stream, self.addr)) {
          println!("new socket returned");
          reactor.register_listener(sock).unwrap();
          reactor.remove_listener_by_fd(self.get_fd() as u64).unwrap();
        } else {
          println!("nothing returned");
        }
      }
      _ => panic!("unexpected event"),
    }
  }
}

fn add_interest(epoll_fd: RawFd, fd: RawFd, interest: Interest) -> io::Result<()> {
  // let mut ee = libc::epoll_event{events: libc::EPOLLIN as u32, u64: fd as u64};
  let mut ee = libc::epoll_event{events: interest.0 as u32, u64: fd as u64};
  unsafe {
    let ret = libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut ee);
    if ret < 0 {
      return Err(std::io::Error::last_os_error());
    }
    return Ok(());
  }
}

fn remove_interest(epoll_fd: RawFd, fd: RawFd) -> io::Result<()> {
  unsafe {
    let ret = libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut());
    if ret < 0 {
      return Err(std::io::Error::last_os_error());
    }
    return Ok(());
  }
}

fn change_interest(epoll_fd: RawFd, fd: RawFd, interest: Interest) -> io::Result<()> {
  let mut ee = libc::epoll_event{events: interest.0 as u32, u64: fd as u64};
  unsafe {
    let ret = libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_MOD, fd, &mut ee);
    if ret < 9 {
      return Err(std::io::Error::last_os_error());
    }
    return Ok(());
  }
}

fn do_epoll_wait(efd: RawFd, events: &mut Vec <libc::epoll_event>, timeout: libc::c_int) -> io::Result<libc::c_int> {
  unsafe {
    let ret = libc::epoll_wait(efd, events.as_mut_ptr() as *mut libc::epoll_event, events.capacity() as i32, timeout);
    if ret < 0 {
      return Err(std::io::Error::last_os_error());
    }
    return Ok(ret)
  }
}
