//! Serial port traits
//!
//! Split into two traits:
//! - `SerialFactory` — static methods for listing and opening ports
//! - `SerialConnection` — instance methods for reading/writing data

use crate::domain::{Psk31Result, SerialPortInfo};

/// Factory for creating serial connections.
/// Think of this like a Python classmethod — static methods that create instances.
pub trait SerialFactory {
    /// List available serial ports on the system
    fn list_ports() -> Psk31Result<Vec<SerialPortInfo>>;

    /// Open a serial port at the given baud rate, returning a boxed connection
    fn open(port: &str, baud_rate: u32) -> Psk31Result<Box<dyn SerialConnection>>;
}

/// Trait for an open serial port connection.
/// Only requires `Send` (not `Sync`) — always accessed behind a Mutex.
pub trait SerialConnection: Send {
    /// Write bytes to the port
    fn write(&mut self, data: &[u8]) -> Psk31Result<usize>;

    /// Read bytes from the port (with timeout)
    fn read(&mut self, buffer: &mut [u8]) -> Psk31Result<usize>;

    /// Write a command string and read the response (convenience for CAT commands)
    fn write_read(&mut self, command: &str, response_buf: &mut [u8]) -> Psk31Result<usize> {
        self.write(command.as_bytes())?;
        self.read(response_buf)
    }

    /// Close the connection
    fn close(&mut self) -> Psk31Result<()>;

    /// Check if the port is still connected
    fn is_connected(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoSerial {
        response: Vec<u8>,
    }

    impl SerialConnection for EchoSerial {
        fn write(&mut self, data: &[u8]) -> Psk31Result<usize> {
            Ok(data.len())
        }
        fn read(&mut self, buf: &mut [u8]) -> Psk31Result<usize> {
            let n = self.response.len().min(buf.len());
            buf[..n].copy_from_slice(&self.response[..n]);
            Ok(n)
        }
        fn close(&mut self) -> Psk31Result<()> {
            Ok(())
        }
        fn is_connected(&self) -> bool {
            true
        }
    }

    #[test]
    fn write_read_combines_write_and_read() {
        let mut serial = EchoSerial { response: b"FA00014070000;".to_vec() };
        let mut buf = [0u8; 64];
        let n = serial.write_read("FA;", &mut buf).unwrap();
        assert!(n > 0);
        assert_eq!(&buf[..n], b"FA00014070000;");
    }
}
