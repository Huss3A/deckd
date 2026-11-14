//! Wire protocol for the Ulanzi D200H (USB 2207:0019). See NOTES.md for what is verified.

use hidapi::{HidApi, HidDevice};
use std::time::{Duration, Instant};

const VID: u16 = 0x2207;
const PID: u16 = 0x0019;
const DATA_INTERFACE: i32 = 0;
pub const PACKET: usize = 1024;
const MAGIC: [u8; 2] = [0x7c, 0x7c];
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(5); // what Ulanzi Studio does

/// Command IDs (verified against Ulanzi Studio traffic captured with Frida).
pub mod cmd {
    pub const FULL_LAYOUT: u16 = 0x0001;
    pub const GETBASE: u16 = 0x0003; // reply: 0x0303 JSON (serial, firmware, error)
    pub const SMALL_WINDOW: u16 = 0x0006;
    pub const BRIGHTNESS: u16 = 0x000a;
    pub const LABEL_STYLE: u16 = 0x000b;
    pub const PARTIAL_LAYOUT: u16 = 0x000d;
    pub const UNKNOWN_1A: u16 = 0x001a; // app sends "0" after brightness; meaning unknown
    pub const DEVICE_INFO: u16 = 0x0303;
    pub const CONFIG_REQUESTED: u16 = 0x010b; // [doc] firmware wants the layout re-sent
}

const LABEL_STYLE_JSON: &str = "{\n    \"Align\": \"bottom\",\n    \"Color\": 16777215,\n    \"FontName\": \"Source Han Sans SC\",\n    \"ShowTitle\": true,\n    \"Size\": 10,\n    \"Weight\": 80\n}\n";

/// Incoming reports.
#[derive(Debug)]
pub enum Event {
    Button { index: u8, state: u8, pressed: bool },
    Other { cmd: u16, payload: Vec<u8> },
}

/// Build a 1025-byte HID write buffer: report ID 0x00 + 1024-byte packet.
/// `total_len` is the value of the length field (for ZIPs, the whole archive size).
fn build_packet(command: u16, total_len: usize, payload: &[u8]) -> Vec<u8> {
    let mut buf = vec![0u8; 1 + PACKET];
    buf[1..3].copy_from_slice(&MAGIC);
    buf[3..5].copy_from_slice(&command.to_be_bytes()); // command is big-endian
    buf[5..9].copy_from_slice(&(total_len as u32).to_le_bytes()); // length is little-endian
    let n = payload.len().min(PACKET - 8);
    buf[9..9 + n].copy_from_slice(&payload[..n]);
    buf
}

pub fn send(dev: &HidDevice, command: u16, payload: &[u8]) -> Result<(), String> {
    dev.write(&build_packet(command, payload.len(), payload))
        .map(|_| ())
        .map_err(|e| format!("write {command:#06x} failed: {e}"))
}

pub fn parse(report: &[u8]) -> Option<Event> {
    if report.len() < 12 || report[..2] != MAGIC {
        return None;
    }
    let command = u16::from_be_bytes([report[2], report[3]]);
    match command {
        0x0101 | 0x0102 => Some(Event::Button { state: report[8], index: report[9], pressed: report[11] == 1 }),
        _ => Some(Event::Other { cmd: command, payload: report[8..].to_vec() }),
    }
}

pub fn open(api: &HidApi) -> Result<HidDevice, String> {
    let info = api
        .device_list()
        .find(|d| d.vendor_id() == VID && d.product_id() == PID && d.interface_number() == DATA_INTERFACE)
        .ok_or("deck not found (is it plugged in, and is Ulanzi Studio closed?)")?;
    let dev = info.open_device(api).map_err(|e| format!("open failed: {e}"))?;
    dev.set_blocking_mode(false).map_err(|e| e.to_string())?;
    Ok(dev)
}

/// Small-window data `mode|cpu|mem|time|gpu|12H|` (mode 1 = clock); also the keep-alive.
pub fn heartbeat(dev: &HidDevice) -> Result<(), String> {
    heartbeat_mode(dev, 1, 0, 0)
}

/// `mode` is the wide slot's display mode: 1 = firmware clock, 2 = our own picture (works, verified), 0 = firmware stats
/// (from community docs, unverified on this deck). `cpu` / `mem` are percent values the firmware's stats readout shows.
pub fn heartbeat_mode(dev: &HidDevice, mode: u8, cpu: u8, mem: u8) -> Result<(), String> {
    let now = chrono::Local::now().format("%H:%M:%S");
    send(dev, cmd::SMALL_WINDOW, format!("{mode}|{cpu}|{mem}|{now}|0|12H|").as_bytes())
}

pub fn drain(dev: &HidDevice) {
    let mut buf = [0u8; PACKET + 1];
    for _ in 0..256 {
        if !matches!(dev.read(&mut buf), Ok(n) if n > 0) {
            break;
        }
    }
}

/// Wait for a specific incoming command; other reports (button events) are skipped.
fn wait_reply(dev: &HidDevice, want: u16, timeout: Duration) -> Option<String> {
    let end = Instant::now() + timeout;
    let mut buf = [0u8; PACKET + 1];
    while Instant::now() < end {
        match dev.read(&mut buf) {
            Ok(n) if n > 0 => {
                if let Some(Event::Other { cmd, payload }) = parse(&buf[..n]) {
                    if cmd == want {
                        let text: Vec<u8> = payload.into_iter().take_while(|b| *b != 0).collect();
                        return Some(String::from_utf8_lossy(&text).into_owned());
                    }
                }
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    None
}

/// GETBASE barrier: deck answers with a JSON blob whose "error" must be 0.
pub fn getbase(dev: &HidDevice) -> Result<String, String> {
    send(dev, cmd::GETBASE, &[])?;
    let json = wait_reply(dev, cmd::DEVICE_INFO, Duration::from_secs(5)).ok_or("no GETBASE reply within 5s")?;
    if !json.contains("\"error\": \"0\"") && !json.contains("\"error\":\"0\"") {
        return Err(format!("deck reported an error: {json}"));
    }
    Ok(json)
}

/// Send a ZIP: first packet carries the total length and the first 1016 bytes,
/// the rest goes as raw 1024-byte chunks (zero-padded), each with the report-ID byte.
pub fn upload_zip(dev: &HidDevice, command: u16, zip: &[u8], barriers: bool) -> Result<(), String> {
    if barriers {
        drain(dev);
        getbase(dev)?;
    }
    let first = zip.len().min(PACKET - 8);
    dev.write(&build_packet(command, zip.len(), &zip[..first])).map_err(|e| e.to_string())?;
    for chunk in zip[first..].chunks(PACKET) {
        let mut buf = vec![0u8; 1 + PACKET];
        buf[1..1 + chunk.len()].copy_from_slice(chunk);
        dev.write(&buf).map_err(|e| format!("chunk write failed: {e}"))?;
    }
    if barriers {
        getbase(dev)?;
    }
    Ok(())
}

/// Same order the stock app uses when it connects. Returns the deck's identity JSON.
pub fn handshake(dev: &HidDevice, brightness: u8) -> Result<String, String> {
    heartbeat(dev)?;
    let info = getbase(dev)?;
    send(dev, cmd::LABEL_STYLE, LABEL_STYLE_JSON.as_bytes())?;
    getbase(dev)?;
    send(dev, cmd::BRIGHTNESS, brightness.min(100).to_string().as_bytes())?;
    send(dev, cmd::UNKNOWN_1A, b"0")?;
    Ok(info.split_whitespace().collect::<Vec<_>>().join(" "))
}
