//! A bounded private runner-to-preview channel. Blocking reads do no idle work.
use rand::RngCore;
use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

pub const ADDRESS: &str = "CRANPOSE_LIVE_ADDRESS";
pub const TOKEN: &str = "CRANPOSE_LIVE_TOKEN";
const LIMIT: usize = 1024 * 1024;

pub struct Bridge {
    listener: TcpListener,
    token: String,
    stream: Option<TcpStream>,
    disconnected: bool,
}
impl Bridge {
    pub fn new() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let mut random = [0u8; 24];
        rand::rng().fill_bytes(&mut random);
        Ok(Self {
            listener,
            token: random.iter().map(|v| format!("{v:02x}")).collect(),
            stream: None,
            disconnected: false,
        })
    }
    pub fn environment(&self) -> io::Result<[(String, String); 2]> {
        Ok([
            (ADDRESS.into(), self.listener.local_addr()?.to_string()),
            (TOKEN.into(), self.token.clone()),
        ])
    }
    pub fn exchange(&mut self, payload: &str, timeout: Duration) -> io::Result<String> {
        if self.disconnected {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Restart preview to reconnect live values",
            ));
        }
        let deadline = Instant::now() + timeout;
        if self.stream.is_none() {
            loop {
                match self.listener.accept() {
                    Ok((mut stream, _)) => {
                        // macOS inherits the listener's nonblocking flag.
                        stream.set_nonblocking(false)?;
                        let remaining = deadline
                            .saturating_duration_since(Instant::now())
                            .max(Duration::from_millis(1));
                        stream.set_read_timeout(Some(remaining))?;
                        stream.set_write_timeout(Some(remaining))?;
                        stream.set_nodelay(true)?;
                        if read(&mut stream)? != self.token {
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "Live preview authentication failed",
                            ));
                        }
                        self.stream = Some(stream);
                        break;
                    }
                    Err(e)
                        if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        let stream = self.stream.as_mut().expect("connected");
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        stream.set_read_timeout(Some(remaining))?;
        stream.set_write_timeout(Some(remaining))?;
        let result = write(stream, payload).and_then(|()| read(stream));
        if result.is_err() {
            self.stream.take();
            // The preview receiver exits on disconnect. Avoid imposing another
            // timeout on every subsequent edit while the compiler takes over.
            self.disconnected = true;
        }
        result
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        if let Some(stream) = &self.stream {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}
/// Called once by the preview's shared runtime. The operating system tears down
/// the stream with the process; disconnect ends this worker without retry loops.
pub fn receive(apply: impl Fn(String) -> String + Send + 'static) {
    let (Ok(address), Ok(token)) = (std::env::var(ADDRESS), std::env::var(TOKEN)) else {
        return;
    };
    std::thread::spawn(move || {
        let run = || -> io::Result<()> {
            let address = address
                .parse()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Live address"))?;
            let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
            stream.set_nodelay(true)?;
            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
            write(&mut stream, &token)?;
            loop {
                let payload = read(&mut stream)?;
                write(&mut stream, &apply(payload))?;
            }
        };
        let _ = run();
    });
}
fn write(stream: &mut impl Write, text: &str) -> io::Result<()> {
    if text.len() > LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Live message too large",
        ));
    }
    stream.write_all(&(text.len() as u32).to_be_bytes())?;
    stream.write_all(text.as_bytes())
}
fn read(stream: &mut impl Read) -> io::Result<String> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Live message too large",
        ));
    }
    let mut bytes = vec![0u8; length];
    stream.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_frames_and_disconnect() {
        assert!(read(&mut &(u32::MAX.to_be_bytes())[..]).is_err());
        assert!(write(&mut Vec::new(), &"x".repeat(LIMIT + 1)).is_err());
        let mut bytes = vec![];
        write(&mut bytes, "é 🦀").expect("write");
        assert_eq!(read(&mut bytes.as_slice()).expect("read"), "é 🦀");
        let mut bridge = Bridge::new().expect("bridge");
        let address = bridge.listener.local_addr().expect("address");
        let token = bridge.token.clone();
        let worker = std::thread::spawn(move || {
            let mut socket = TcpStream::connect(address).expect("connect");
            write(&mut socket, &token).expect("auth");
            assert_eq!(read(&mut socket).expect("payload"), "update");
            write(&mut socket, "applied").expect("reply");
            assert!(read(&mut socket).is_err());
        });
        assert_eq!(
            bridge
                .exchange("update", Duration::from_secs(1))
                .expect("exchange"),
            "applied"
        );
        drop(bridge);
        worker.join().expect("exit");
    }

    #[test]
    fn failed_connected_channel_does_not_retry_each_edit() {
        let mut bridge = Bridge::new().expect("bridge");
        let address = bridge.listener.local_addr().expect("address");
        let token = bridge.token.clone();
        let worker = std::thread::spawn(move || {
            let mut socket = TcpStream::connect(address).expect("connect");
            write(&mut socket, &token).expect("auth");
            assert_eq!(read(&mut socket).expect("payload"), "first");
            // The receiver exits instead of acknowledging this request.
        });
        assert!(bridge.exchange("first", Duration::from_secs(1)).is_err());
        worker.join().expect("exit");
        assert!(bridge.disconnected);
        assert!(bridge.stream.is_none());
        assert_eq!(
            bridge
                .exchange("next", Duration::ZERO)
                .expect_err("closed")
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
