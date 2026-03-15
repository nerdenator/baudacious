//! Serial port adapter using the `serialport` crate
//!
//! Implements `SerialFactory` and `SerialConnection` traits.
//! Think of `SerialPortFactory` like a Python classmethod container —
//! it has no instance data, just static methods for listing/opening ports.

use std::time::Duration;

use crate::domain::{Psk31Error, Psk31Result, SerialPortInfo};
use crate::ports::{SerialConnection, SerialFactory};

/// Return a human-readable device label for a known USB VID:PID pair.
fn known_device(vid: u16, pid: u16) -> Option<&'static str> {
    match (vid, pid) {
        (0x10C4, 0xEA60) => Some("Yaesu FT-991A / CP210x"),
        (0x10C4, 0xEA70) => Some("Yaesu FT-991A / CP2105 (SLAB_USBtoUART)"),
        (0x0403, 0x6001) => Some("FTDI USB Serial"),
        (0x0403, 0x6015) => Some("FTDI USB Serial"),
        (0x067B, 0x2303) => Some("Prolific USB Serial"),
        _ => None,
    }
}

/// Zero-sized factory for creating serial port connections.
pub struct SerialPortFactory;

impl SerialFactory for SerialPortFactory {
    fn list_ports() -> Psk31Result<Vec<SerialPortInfo>> {
        let ports = serialport::available_ports()
            .map_err(|e| Psk31Error::Serial(format!("Failed to list ports: {e}")))?;

        Ok(ports
            .into_iter()
            .map(|p| {
                let (port_type, device_hint) = match &p.port_type {
                    serialport::SerialPortType::UsbPort(info) => {
                        let label = format!("USB ({:04X}:{:04X})", info.vid, info.pid);
                        let hint = known_device(info.vid, info.pid).map(|s| s.to_string());
                        (label, hint)
                    }
                    serialport::SerialPortType::PciPort => ("PCI".to_string(), None),
                    serialport::SerialPortType::BluetoothPort => ("Bluetooth".to_string(), None),
                    serialport::SerialPortType::Unknown => ("Native".to_string(), None),
                };
                SerialPortInfo {
                    name: p.port_name,
                    port_type,
                    device_hint,
                }
            })
            .collect())
    }

    fn open(port: &str, baud_rate: u32) -> Psk31Result<Box<dyn SerialConnection>> {
        let serial = serialport::new(port, baud_rate)
            .timeout(Duration::from_millis(100))
            .data_bits(serialport::DataBits::Eight)
            .stop_bits(serialport::StopBits::One)
            .parity(serialport::Parity::None)
            .flow_control(serialport::FlowControl::None)
            .open()
            .map_err(|e| Psk31Error::Serial(format!("Failed to open {port}: {e}")))?;

        Ok(Box::new(SerialPortConnection {
            port: serial,
            connected: true,
        }))
    }
}

/// An open serial port connection wrapping the `serialport` crate.
pub struct SerialPortConnection {
    port: Box<dyn serialport::SerialPort>,
    connected: bool,
}

impl SerialConnection for SerialPortConnection {
    fn write(&mut self, data: &[u8]) -> Psk31Result<usize> {
        use std::io::Write;
        self.port
            .write(data)
            .map_err(|e| Psk31Error::Serial(format!("Write failed: {e}")))
    }

    fn read(&mut self, buffer: &mut [u8]) -> Psk31Result<usize> {
        use std::io::Read;
        self.port
            .read(buffer)
            .map_err(|e| Psk31Error::Serial(format!("Read failed: {e}")))
    }

    fn close(&mut self) -> Psk31Result<()> {
        self.connected = false;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected
    }
}
