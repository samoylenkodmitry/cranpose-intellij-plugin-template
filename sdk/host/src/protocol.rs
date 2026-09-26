//! Host side of Cranpose's version 2 embedded surface protocol.
use anyhow::{Context, Result, ensure};
use std::io::{Read, Write};

pub const VERSION: u32 = 2;
const MAX_PACKET: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Frame {
    pub surface: u32,
    pub id: u32,
    pub buffer_width: u32,
    pub buffer_height: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<i32>,
}
#[derive(Debug, Clone)]
pub struct Window {
    pub surface: u32,
    pub title: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub relative: bool,
    pub flags: u8,
}
#[derive(Debug, Clone)]
pub enum Event {
    Hello(u32, String),
    Frame(Frame),
    Cursor(u32, String),
    Message(String, String),
    Window(Window),
    Overlay(u32, String),
    Close(u32),
    Move(u32),
    Resize(u32, u8),
    Unknown,
}
pub fn read(input: &mut impl Read) -> Result<Option<Event>> {
    let mut size = [0; 4];
    match input.read(&mut size[..1]) {
        Ok(0) => return Ok(None),
        Ok(_) => input.read_exact(&mut size[1..])?,
        Err(e) => return Err(e.into()),
    };
    let size = u32::from_le_bytes(size) as usize;
    ensure!(
        (1..=MAX_PACKET).contains(&size),
        "Invalid Cranpose packet size: {size}"
    );
    let mut body = vec![0; size];
    input.read_exact(&mut body)?;
    let mut r = Reader(&body);
    Ok(Some(match r.byte()? {
        1 => Event::Hello(r.int()?, r.string()?),
        2 => {
            let surface = r.int()?;
            let id = r.int()?;
            let buffer_width = r.int()?;
            let buffer_height = r.int()?;
            let x = r.int()?;
            let y = r.int()?;
            let width = r.int()?;
            let height = r.int()?;
            ensure!(
                buffer_width > 0
                    && buffer_height > 0
                    && buffer_width as u64 * buffer_height as u64 * 4 <= MAX_PACKET as u64,
                "Invalid surface dimensions"
            );
            ensure!(
                width > 0
                    && height > 0
                    && x.checked_add(width).is_some_and(|end| end <= buffer_width)
                    && y.checked_add(height)
                        .is_some_and(|end| end <= buffer_height),
                "Frame rectangle exceeds surface"
            );
            ensure!(
                width as u64 * height as u64 * 4 == r.0.len() as u64,
                "Frame pixel count mismatch"
            );
            let pixels =
                r.0.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| i32::from_le_bytes(*b))
                    .collect();
            Event::Frame(Frame {
                surface,
                id,
                buffer_width,
                buffer_height,
                x,
                y,
                width,
                height,
                pixels,
            })
        }
        3 => Event::Cursor(r.int()?, r.string()?),
        4 => Event::Message(r.string()?, r.string()?),
        5 => Event::Window(Window {
            surface: r.int()?,
            title: r.string()?,
            x: r.float()?,
            y: r.float()?,
            width: r.float()?,
            height: r.float()?,
            relative: r.byte()? != 0,
            flags: r.byte()?,
        }),
        6 => Event::Overlay(r.int()?, r.string()?),
        7 => Event::Close(r.int()?),
        8 => Event::Move(r.int()?),
        9 => Event::Resize(r.int()?, r.byte()?),
        _ => Event::Unknown,
    }))
}
struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        ensure!(n <= self.0.len(), "Truncated Cranpose packet");
        let (value, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn int(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("u32")))
    }
    fn float(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.int()?))
    }
    fn string(&mut self) -> Result<String> {
        let length = self.int()? as usize;
        Ok(std::str::from_utf8(self.take(length)?)
            .context("Invalid UTF-8 in Cranpose protocol")?
            .to_owned())
    }
}
pub struct Packet(pub Vec<u8>);
impl Packet {
    pub fn new(kind: u8) -> Self {
        Self(vec![kind])
    }
    pub fn int(mut self, value: u32) -> Self {
        self.0.extend(value.to_le_bytes());
        self
    }
    pub fn float(mut self, value: f32) -> Self {
        self.0.extend(value.to_le_bytes());
        self
    }
    pub fn byte(mut self, value: u8) -> Self {
        self.0.push(value);
        self
    }
    pub fn text(mut self, value: &str) -> Self {
        self.0.extend((value.len() as u32).to_le_bytes());
        self.0.extend(value.as_bytes());
        self
    }
    pub fn send(&self, output: &mut impl Write) -> Result<()> {
        output.write_all(&(self.0.len() as u32).to_le_bytes())?;
        output.write_all(&self.0)?;
        output.flush()?;
        Ok(())
    }
    pub fn message(channel: &str, payload: &str) -> Self {
        Self::new(10).text(channel).text(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_overflow_and_out_of_bounds_frames() {
        let bytes = Packet::new(2)
            .int(0)
            .int(1)
            .int(2)
            .int(2)
            .int(u32::MAX)
            .int(0)
            .int(1)
            .int(1)
            .int(0);
        let mut data = vec![];
        bytes.send(&mut data).expect("encode");
        assert!(read(&mut data.as_slice()).is_err());
        let mut truncated = &[3, 0, 0, 0, 1][..];
        assert!(read(&mut truncated).is_err());
    }
    #[test]
    fn hello_unicode_and_unknown_packet() {
        let mut data = vec![];
        Packet::new(1)
            .int(2)
            .text("🦀")
            .send(&mut data)
            .expect("encode");
        assert!(
            matches!(read(&mut data.as_slice()).expect("decode"),Some(Event::Hello(2,s)) if s=="🦀")
        );
        assert!(matches!(
            read(&mut [1, 0, 0, 0, 99].as_slice()).expect("unknown"),
            Some(Event::Unknown)
        ));
    }
    #[test]
    fn clean_eof_differs_from_truncated_header() {
        assert!(read(&mut [].as_slice()).expect("eof").is_none());
        assert!(read(&mut [1, 0].as_slice()).is_err());
    }
}
