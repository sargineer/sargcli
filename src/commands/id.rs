//! `sarg id`: what is this USB device, and what does sarg know about it?
//! The 09-02 flash went wrong partly because nobody asked what enumerates
//! as 303a:1001. Reads sysfs (no lsusb, no root), names the chip from a
//! small table, and asks sarg for lessons that mention the id.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::Ctx;
use crate::client::urlencode;
use crate::error::{Result, SargError};
use crate::output;
use crate::render::{self, s, status_badge, truncate};

#[derive(Debug, Clone, serde::Serialize)]
pub struct UsbDevice {
    pub vid: String,
    pub pid: String,
    pub manufacturer: String,
    pub product: String,
    pub serial: String,
    pub tty: Vec<String>,
    pub sysfs: String,
}

/// vid[:pid] → (who, what)
fn known(vid: &str, pid: &str) -> (&'static str, &'static str) {
    match (vid, pid) {
        ("303a", "1001") => ("Espressif", "USB JTAG/serial debug unit — native USB of an ESP32-S3/C3/C6/H2"),
        ("303a", "0002") => ("Espressif", "ESP32-S2 USB bootloader"),
        ("303a", "1002") => ("Espressif", "ESP32-S3 USB bootloader (download mode)"),
        ("303a", _) => ("Espressif", "ESP32 family native USB (TinyUSB or custom PID)"),
        ("10c4", "ea60") => ("Silicon Labs", "CP2102/CP2104 USB-UART bridge — common on ESP32 devkits"),
        ("1a86", "7523") => ("WCH", "CH340 USB-UART bridge — common on clone devkits and camera boards"),
        ("1a86", "55d4") => ("WCH", "CH9102 USB-UART bridge"),
        ("1a86", _) => ("WCH", "CH34x/CH9xx USB-UART bridge"),
        ("0403", _) => ("FTDI", "FT232/FT2232 USB-UART"),
        ("2e8a", "0003") => ("Raspberry Pi", "RP2040 BOOTSEL (UF2 mass storage)"),
        ("2e8a", "000f") => ("Raspberry Pi", "RP2350 BOOTSEL (UF2 mass storage)"),
        ("2e8a", "0005") => ("Raspberry Pi", "RP2 running MicroPython (CDC)"),
        ("2e8a", "000c") => ("Raspberry Pi", "Debug Probe"),
        ("2e8a", _) => ("Raspberry Pi", "RP2040/RP2350 device"),
        ("239a", _) => ("Adafruit", "Adafruit board (CircuitPython/Arduino/UF2 bootloader; PID names the board)"),
        ("0483", "df11") => ("STMicro", "STM32 DFU bootloader"),
        ("0483", _) => ("STMicro", "STM32 USB device (CDC/composite)"),
        ("1915", _) => ("Nordic", "nRF52/nRF53 USB (DFU or CDC)"),
        ("1209", "abd1") => ("OpenMV", "OpenMV Cam (CDC + MSC)"),
        ("1209", _) => ("pid.codes", "open-source hardware (PID names the project)"),
        ("2207", _) => ("Rockchip", "Rockchip SoC (MaskROM/ADB/RNDIS) — e.g. Luckfox Pico"),
        ("2341", _) => ("Arduino", "Arduino board"),
        ("1b4f", _) => ("SparkFun", "SparkFun board"),
        ("16c0", "0483") => ("PJRC", "Teensy (serial)"),
        ("0bda", "2838") => ("Realtek", "RTL2832U — RTL-SDR dongle"),
        ("1d50", "60a1") => ("Airspy", "Airspy R2/Mini SDR"),
        ("0525", _) => ("Netchip/Linux", "USB gadget (RNDIS/ECM/serial) — a Linux board pretending to be a device"),
        ("1d6b", _) => ("Linux", "root hub"),
        ("05e3", _) => ("Genesys Logic", "USB hub"),
        ("1a40", _) => ("Terminus", "USB hub"),
        ("0451", _) => ("Texas Instruments", "USB hub or TI device"),
        ("04d8", _) => ("Microchip", "Microchip device (PIC/ATSAM or MCP2221)"),
        _ => ("unknown vendor", "not in sarg's table"),
    }
}

fn read(p: &Path, name: &str) -> String {
    fs::read_to_string(p.join(name))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn find_ttys(dev: &Path, depth: u8) -> Vec<String> {
    let mut out = Vec::new();
    if depth == 0 {
        return out;
    }
    if let Ok(rd) = fs::read_dir(dev) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let path = e.path();
            if name.starts_with("ttyACM") || name.starts_with("ttyUSB") {
                out.push(format!("/dev/{name}"));
            } else if name == "tty" {
                if let Ok(rd2) = fs::read_dir(&path) {
                    for t in rd2.flatten() {
                        out.push(format!("/dev/{}", t.file_name().to_string_lossy()));
                    }
                }
            } else if path.is_dir() && (name.contains(':') || name.starts_with("ttyUSB")) {
                out.extend(find_ttys(&path, depth - 1));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn device_at(p: &Path) -> Option<UsbDevice> {
    let vid = read(p, "idVendor");
    if vid.is_empty() {
        return None;
    }
    Some(UsbDevice {
        vid,
        pid: read(p, "idProduct"),
        manufacturer: read(p, "manufacturer"),
        product: read(p, "product"),
        serial: read(p, "serial"),
        tty: find_ttys(p, 3),
        sysfs: p.display().to_string(),
    })
}

/// Every USB device that is not a hub.
pub fn scan() -> Vec<UsbDevice> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/bus/usb/devices") {
        for e in rd.flatten() {
            let p = e.path();
            if read(&p, "bDeviceClass") == "09" {
                continue;
            }
            if let Some(d) = device_at(&p) {
                if d.vid != "1d6b" {
                    out.push(d);
                }
            }
        }
    }
    out.sort_by(|a, b| a.sysfs.cmp(&b.sysfs));
    out
}

/// /dev/ttyACM0 → the USB device that owns it.
pub fn from_tty(tty: &str) -> Option<UsbDevice> {
    let name = tty.trim_start_matches("/dev/");
    let mut p: PathBuf = fs::canonicalize(format!("/sys/class/tty/{name}/device")).ok()?;
    for _ in 0..6 {
        if let Some(d) = device_at(&p) {
            return Some(d);
        }
        p = p.parent()?.to_path_buf();
    }
    None
}

fn parse_vidpid(sp: &str) -> Option<(String, String)> {
    let (v, p) = sp.split_once(':')?;
    let ok = |x: &str| x.len() == 4 && x.chars().all(|c| c.is_ascii_hexdigit());
    (ok(v) && ok(p)).then(|| (v.to_lowercase(), p.to_lowercase()))
}

pub fn run(ctx: &mut Ctx, what: Option<&str>) -> Result<()> {
    let devices: Vec<UsbDevice> = match what {
        None => scan(),
        Some(w) if w.starts_with("/dev/") || w.starts_with("tty") => {
            vec![from_tty(w).ok_or_else(|| {
                SargError::usage(format!("{w}: not a USB serial device on this machine"))
            })?]
        }
        Some(w) => {
            let (vid, pid) = parse_vidpid(w).ok_or_else(|| {
                SargError::usage("give vid:pid (303a:1001), /dev/ttyACM0, or nothing to scan")
            })?;
            // Prefer a live device with that id so we get its strings and tty.
            let mut live: Vec<UsbDevice> = scan()
                .into_iter()
                .filter(|d| d.vid == vid && d.pid == pid)
                .collect();
            if live.is_empty() {
                live.push(UsbDevice {
                    vid,
                    pid,
                    manufacturer: String::new(),
                    product: String::new(),
                    serial: String::new(),
                    tty: vec![],
                    sysfs: String::new(),
                });
            }
            live
        }
    };
    if devices.is_empty() {
        println!("no USB devices other than hubs are visible in /sys/bus/usb/devices");
        return Ok(());
    }

    let w = render::width();
    let mut report = Vec::new();

    for d in &devices {
        let (who, what_is) = known(&d.vid, &d.pid);
        // Lessons that mention the id verbatim.
        let q = format!("{}:{}", d.vid, d.pid);
        let notes = ctx
            .client
            .get_json(&format!("/notes?q={}&n=5", urlencode(&q)))
            .ok()
            .and_then(|v| v.get("results").and_then(|r| r.as_array()).cloned())
            .unwrap_or_default();

        if ctx.json() {
            report.push(json!({
                "device": d,
                "vendor": who,
                "what": what_is,
                "lessons": notes,
            }));
            continue;
        }

        let mut head = vec![render::bold(&format!("{}:{}", d.vid, d.pid))];
        if !d.manufacturer.is_empty() || !d.product.is_empty() {
            head.push(format!("{} {}", d.manufacturer, d.product).trim().to_string());
        }
        if !d.tty.is_empty() {
            head.push(render::good(&d.tty.join(" ")));
        }
        if !d.serial.is_empty() {
            head.push(render::dim(&format!("serial {}", d.serial)));
        }
        println!("{}", head.join(" · "));
        println!("  {} — {}", who, what_is);
        if notes.is_empty() {
            println!("  {}", render::dim(&format!("no lessons mention {q}")));
        }
        for n in notes.iter().take(3) {
            println!(
                "  {} {} {}",
                status_badge(s(n, "status")),
                render::bold(&format!("{}/{}", s(n, "handle"), s(n, "id"))),
                render::dim(&truncate(s(n, "title"), w.saturating_sub(36)))
            );
        }
        if notes.len() > 3 {
            println!(
                "{}",
                render::dim(&format!("  … sarg ask '{q}' for {} lessons", notes.len()))
            );
        }
        println!();
    }
    if ctx.json() {
        output::json(&report);
    }
    Ok(())
}
