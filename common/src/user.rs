use crate::util::collapse;

use super::proto::{End, Env, Rcvr, Sndr};
use super::socket::{Socket, SocketError};
use std::fmt::Display;
use std::time::{Duration, Instant};

static TICK: Duration = Duration::from_millis(100);

#[derive(Clone, Debug)]
pub struct UserError {
    msg: String,
}

impl UserError {
    fn new(message: &str) -> UserError {
        UserError {
            msg: String::from(message),
        }
    }

    fn from_socket(err: &SocketError) -> UserError {
        UserError {
            msg: format!("Underlying socket error: {}", err),
        }
    }

    fn from_sockets(err_list: &[SocketError]) -> UserError {
        let mut message = format!("{} Underlying socket error(s):", err_list.len());
        for err in err_list.iter() {
            let s = format!("\n  * {}", err);
            message.push_str(&s);
        }
        UserError { msg: message }
    }
}

impl Display for UserError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "UserError: {}", &(self.msg))
    }
}

impl std::error::Error for UserError {}

pub struct User {
    socket: Socket,
    name: String,
    id: u64,
    idstr: String,
    bytes_read: usize,
    quota_bytes: usize,
    last_data_time: Instant,
    errs: Vec<SocketError>,
    blocked_users: Vec<u64>,
}

impl User {
    pub fn new(new_socket: Socket, new_id: u64) -> User {
        let new_name = format!("user{}", &new_id);
        User {
            socket: new_socket,
            id: new_id,
            idstr: collapse(&new_name),
            name: new_name,
            bytes_read: 0,
            quota_bytes: 0,
            last_data_time: Instant::now(),
            errs: Vec::<SocketError>::new(),
            blocked_users: Vec::<u64>::new(),
        }
    }

    pub fn get_name(&self) -> &str {
        &(self.name)
    }
    pub fn get_id(&self) -> u64 {
        self.id
    }
    pub fn get_idstr(&self) -> &str {
        &(self.idstr)
    }
    pub fn get_addr(&mut self) -> Option<String> {
        match self.socket.get_addr() {
            Ok(a) => Some(a),
            Err(e) => {
                self.errs.push(e);
                None
            }
        }
    }

    pub fn set_name(&mut self, new_name: &str) {
        self.name = String::from(new_name);
        self.idstr = collapse(new_name);
    }

    pub fn get_byte_quota(&self) -> usize {
        self.quota_bytes
    }

    pub fn drain_byte_quota(&mut self, amount: usize) {
        if amount > self.quota_bytes {
            self.quota_bytes = 0;
        } else {
            self.quota_bytes -= amount;
        }
    }

    pub fn get_last_data_time(&self) -> Instant {
        self.last_data_time
    }

    pub fn has_errors(&self) -> bool {
        !self.errs.is_empty()
    }

    pub fn get_errors(&self) -> UserError {
        UserError::from_sockets(&(self.errs))
    }

    pub fn logout(&mut self, logout_message: &str) {
        let msg = Sndr::Logout(logout_message);
        self.deliver_msg(&msg);
        let _ = self.socket.send_data();
        let _ = self.socket.shutdown();
    }

    pub fn block_id(&mut self, id: u64) -> bool {
        match self.blocked_users.binary_search(&id) {
            Err(n) => {
                self.blocked_users.insert(n, id);
                true
            }
            Ok(_) => false,
        }
    }

    pub fn unblock_id(&mut self, id: u64) -> bool {
        match self.blocked_users.binary_search(&id) {
            Err(_) => false,
            Ok(n) => {
                self.blocked_users.remove(n);
                true
            }
        }
    }

    pub fn deliver(&mut self, env: &Env) {
        match env.source {
            End::User(id) => match &(self.blocked_users).binary_search(&id) {
                Ok(_) => {} // do nothing
                Err(_) => {
                    self.socket.enqueue(env.bytes());
                }
            },
            _ => {
                self.socket.enqueue(env.bytes());
            }
        }
    }

    /// Add the contents of an `Sndr` to the outgoing buffer.
    pub fn deliver_msg(&mut self, msg: &Sndr) {
        self.socket.enqueue(&(msg.bytes()));
    }

    /// Send any data that's been queued up.
    pub fn send(&mut self) {
        if self.socket.send_buff_size() > 0 {
            if let Err(e) = self.socket.send_data() {
                self.errs.push(e);
            }
        }
    }

    pub fn blocking_send(&mut self, msg: &Sndr, limit: Duration) -> Result<(), UserError> {
        self.deliver_msg(msg);
        let start_t = Instant::now();
        loop {
            match self.socket.send_data() {
                Err(e) => {
                    let err = UserError::from_socket(&e);
                    self.errs.push(e);
                    return Err(err);
                }
                Ok(0) => return Ok(()),
                _ => (),
            }
            if start_t.elapsed() > limit {
                return Err(UserError::new("Timed out on blocking send."));
            } else {
                std::thread::sleep(TICK);
            }
        }
    }

    /// Attempt to read data and decode a `Msg` from the underlying socket.
    pub fn try_get(&mut self) -> Option<Rcvr> {
        let n = match self.socket.read_data() {
            Err(e) => {
                self.errs.push(e);
                return None;
            }
            Ok(n) => n,
        };

        self.bytes_read += n;

        let buffered_bytes = self.socket.recv_buff_size();
        if buffered_bytes > 0 {
            match self.socket.try_get() {
                Err(e) => {
                    self.errs.push(e);
                    None
                }
                Ok(received_message_option) => {
                    self.last_data_time = Instant::now();
                    if let Some(ref m) = received_message_option {
                        if m.counts() {
                            self.quota_bytes += buffered_bytes - self.socket.recv_buff_size();
                        }
                    }
                    received_message_option
                }
            }
        } else {
            None
        }
    }

    pub fn blocking_get(&mut self, limit: Duration) -> Result<Rcvr, UserError> {
        let start_t = Instant::now();

        loop {
            match self.socket.try_get() {
                Err(e) => return handle_error(&mut self.errs, e),
                Ok(Some(m)) => return Ok(m),
                _ => (),
            }

            match self.socket.read_data() {
                Err(e) => return handle_error(&mut self.errs, e),
                Ok(n) if n > 0 => continue,
                _ => (),
            }

            if start_t.elapsed() > limit {
                return Err(UserError::new("Timed out on a blocking get."));
            } else {
                std::thread::sleep(TICK);
            }
        }
    }
}

fn handle_error(errs: &mut Vec<SocketError>, e: SocketError) -> Result<Rcvr, UserError> {
    let err = UserError::from_socket(&e);
    errs.push(e);
    Err(err)
}
