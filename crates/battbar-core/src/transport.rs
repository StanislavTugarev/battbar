// SPDX-License-Identifier: GPL-3.0-or-later
//! Frame transport abstraction between protocol code and physical links, plus the
//! recording and replay transports used for diagnostics and hardware-free tests.
//!
//! A transport moves whole frames. Protocol framing that differs per link (e.g. HID++
//! over GATT has no report ID and no device index) is handled inside the transport, so
//! protocol code always sees the canonical form.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum TransportError {
    #[error("I/O error: {0}")]
    Io(String),
    #[error("device disconnected")]
    Disconnected,
    #[error("refused: {0}")]
    Refused(String),
}

/// Static description of a link, written as the header of a trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkInfo {
    /// Unique ID within a trace, e.g. `hid:046d:c547:2` or `gatt:<peripheral uuid hash>`.
    pub link: String,
    /// `hidapi`, `gatt`, …
    pub kind: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
    /// `usb` or `ble`.
    pub bus: String,
    /// Top-level collections `(usage_page, usage)` of the HID interface (empty for GATT).
    #[serde(default)]
    pub collections: Vec<(u16, u16)>,
}

pub trait FrameTransport: Send {
    fn info(&self) -> &LinkInfo;
    fn write(&mut self, frame: &[u8]) -> Result<(), TransportError>;
    /// Next incoming frame, or `None` after `timeout`.
    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, TransportError>;
}

pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    s.split_whitespace().map(|h| u8::from_str_radix(h, 16).ok()).collect()
}

// ------------------------------------------------------------------ trace format

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    Out,
    In,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TraceLine {
    Header {
        battbar: String,
        #[serde(flatten)]
        info: LinkInfo,
    },
    Frame {
        t_ms: u64,
        link: String,
        dir: Dir,
        hex: String,
    },
    /// Free-form annotation (e.g. reference values from vendor software).
    Note {
        text: String,
    },
}

/// Shared JSONL trace writer; several recording transports may write into one file.
pub struct TraceWriter {
    out: Box<dyn Write + Send>,
    start: Instant,
    headers: Vec<String>,
}

impl TraceWriter {
    pub fn new(out: Box<dyn Write + Send>) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(TraceWriter {
            out,
            start: Instant::now(),
            headers: vec![],
        }))
    }

    fn line(&mut self, l: &TraceLine) {
        if let Ok(s) = serde_json::to_string(l) {
            let _ = writeln!(self.out, "{s}");
            let _ = self.out.flush();
        }
    }

    pub fn header(&mut self, info: &LinkInfo) {
        if self.headers.contains(&info.link) {
            return;
        }
        self.headers.push(info.link.clone());
        self.line(&TraceLine::Header {
            battbar: env!("CARGO_PKG_VERSION").into(),
            info: info.clone(),
        });
    }

    pub fn frame(&mut self, link: &str, dir: Dir, data: &[u8]) {
        let t_ms = self.start.elapsed().as_millis() as u64;
        self.line(&TraceLine::Frame {
            t_ms,
            link: link.into(),
            dir,
            hex: to_hex(data),
        });
    }

    pub fn note(&mut self, text: &str) {
        self.line(&TraceLine::Note { text: text.into() });
    }
}

/// Proxies another transport and logs every frame.
pub struct RecordingTransport<T: FrameTransport> {
    inner: T,
    trace: Arc<Mutex<TraceWriter>>,
}

impl<T: FrameTransport> RecordingTransport<T> {
    pub fn new(inner: T, trace: Arc<Mutex<TraceWriter>>) -> Self {
        trace.lock().unwrap().header(inner.info());
        RecordingTransport { inner, trace }
    }
}

impl<T: FrameTransport> FrameTransport for RecordingTransport<T> {
    fn info(&self) -> &LinkInfo {
        self.inner.info()
    }
    fn write(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        self.trace.lock().unwrap().frame(&self.inner.info().link, Dir::Out, frame);
        self.inner.write(frame)
    }
    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, TransportError> {
        let r = self.inner.read(timeout)?;
        if let Some(f) = &r {
            self.trace.lock().unwrap().frame(&self.inner.info().link, Dir::In, f);
        }
        Ok(r)
    }
}

impl FrameTransport for Box<dyn FrameTransport> {
    fn info(&self) -> &LinkInfo {
        (**self).info()
    }
    fn write(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        (**self).write(frame)
    }
    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, TransportError> {
        (**self).read(timeout)
    }
}

// ------------------------------------------------------------------ replay

#[derive(Debug, Clone, Default)]
pub struct Trace {
    pub links: Vec<LinkInfo>,
    pub lines: Vec<TraceLine>,
}

#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("line {line}: {err}")]
    Parse { line: usize, err: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl Trace {
    pub fn parse(r: impl BufRead) -> Result<Self, TraceError> {
        let mut t = Trace::default();
        for (i, line) in r.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let l: TraceLine = serde_json::from_str(&line).map_err(|e| TraceError::Parse {
                line: i + 1,
                err: e.to_string(),
            })?;
            if let TraceLine::Header { info, .. } = &l {
                t.links.push(info.clone());
            }
            t.lines.push(l);
        }
        Ok(t)
    }

    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, TraceError> {
        Self::parse(std::io::BufReader::new(std::fs::File::open(path)?))
    }

    /// Replay transports for every link in the trace.
    pub fn transports(&self) -> Vec<ReplayTransport> {
        self.links.iter().map(|l| ReplayTransport::new(self, &l.link)).collect()
    }
}

/// Answers requests from a recorded trace. Responses are looked up by the exact
/// request bytes, so protocol code may change request order without breaking tests.
/// A request recorded several times replays its responses in recorded order and then
/// repeats the last one.
pub struct ReplayTransport {
    info: LinkInfo,
    responses: HashMap<Vec<u8>, VecDeque<Vec<Vec<u8>>>>,
    pending: VecDeque<Vec<u8>>,
    /// Requests that had no recorded answer (useful when a test fails).
    pub unmatched: Vec<Vec<u8>>,
}

impl ReplayTransport {
    pub fn new(trace: &Trace, link: &str) -> Self {
        let info = trace.links.iter().find(|l| l.link == link).cloned().expect("link in trace");
        let mut responses: HashMap<Vec<u8>, VecDeque<Vec<Vec<u8>>>> = HashMap::new();
        let mut pending = VecDeque::new();
        let mut current: Option<(Vec<u8>, Vec<Vec<u8>>)> = None;
        for l in &trace.lines {
            let TraceLine::Frame { link: fl, dir, hex, .. } = l else { continue };
            if fl != link {
                continue;
            }
            let Some(bytes) = from_hex(hex) else { continue };
            match dir {
                Dir::Out => {
                    if let Some((req, ins)) = current.take() {
                        responses.entry(req).or_default().push_back(ins);
                    }
                    current = Some((bytes, vec![]));
                }
                Dir::In => match &mut current {
                    Some((_, ins)) => ins.push(bytes),
                    None => pending.push_back(bytes),
                },
            }
        }
        if let Some((req, ins)) = current.take() {
            responses.entry(req).or_default().push_back(ins);
        }
        ReplayTransport {
            info,
            responses,
            pending,
            unmatched: vec![],
        }
    }
}

impl FrameTransport for ReplayTransport {
    fn info(&self) -> &LinkInfo {
        &self.info
    }
    fn write(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        match self.responses.get_mut(frame) {
            Some(q) => {
                let batch = if q.len() > 1 {
                    q.pop_front().unwrap()
                } else {
                    q.front().cloned().unwrap_or_default()
                };
                self.pending.extend(batch);
            }
            None => self.unmatched.push(frame.to_vec()),
        }
        Ok(())
    }
    fn read(&mut self, _timeout: Duration) -> Result<Option<Vec<u8>>, TransportError> {
        Ok(self.pending.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo(LinkInfo);
    impl FrameTransport for Echo {
        fn info(&self) -> &LinkInfo {
            &self.0
        }
        fn write(&mut self, _f: &[u8]) -> Result<(), TransportError> {
            Ok(())
        }
        fn read(&mut self, _t: Duration) -> Result<Option<Vec<u8>>, TransportError> {
            Ok(Some(vec![0xaa, 0xbb]))
        }
    }

    #[derive(Clone)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn info() -> LinkInfo {
        LinkInfo {
            link: "l1".into(),
            kind: "test".into(),
            vendor_id: 1,
            product_id: 2,
            product: "p".into(),
            bus: "usb".into(),
            collections: vec![(0xff00, 1)],
        }
    }

    #[test]
    fn record_then_replay_roundtrip() {
        let buf = Buf(Arc::new(Mutex::new(vec![])));
        let w = TraceWriter::new(Box::new(buf.clone()));
        let mut rec = RecordingTransport::new(Echo(info()), w);
        rec.write(&[1, 2, 3]).unwrap();
        assert_eq!(rec.read(Duration::ZERO).unwrap(), Some(vec![0xaa, 0xbb]));
        drop(rec);

        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        let trace = Trace::parse(text.as_bytes()).unwrap();
        assert_eq!(trace.links.len(), 1);
        let mut rp = trace.transports().pop().unwrap();
        rp.write(&[9, 9]).unwrap();
        assert_eq!(rp.read(Duration::ZERO).unwrap(), None);
        assert_eq!(rp.unmatched, vec![vec![9, 9]]);
        rp.write(&[1, 2, 3]).unwrap();
        assert_eq!(rp.read(Duration::ZERO).unwrap(), Some(vec![0xaa, 0xbb]));
        // Repeated request repeats the last answer.
        rp.write(&[1, 2, 3]).unwrap();
        assert_eq!(rp.read(Duration::ZERO).unwrap(), Some(vec![0xaa, 0xbb]));
    }
}
