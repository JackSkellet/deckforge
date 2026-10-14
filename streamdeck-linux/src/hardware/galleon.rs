//! Galleon-specific HID metadata and heartbeat contract recovered from Windows7.6.
//! Addresses and evidence boundaries are recorded in docs/GALLEON_REFERENCE.md.
use super::*;
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

pub(super) const WARMUP: Duration = Duration::from_millis(200);
pub(super) const INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, PartialEq)]
pub(super) enum Ping {
    Feature([u8; 32]),
    Output(Box<[u8; REPORT_SIZE]>),
}
impl Ping {
    pub fn for_firmware(firmware: Option<&str>) -> Result<Self> {
        // Native starts with the feature-report variant before querying firmware.
        let output = if let Some(firmware) = firmware {
            let parts = firmware
                .split('.')
                .map(str::parse::<u64>)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            ensure!(parts.len() >= 3, "invalid Galleon firmware version");
            // This is intentionally the native minor-version gate, not a general
            // semver comparison: ESDHWGRETSCHImp::adjustCapabilities,0x14061b110.
            parts[1] >= 6
        } else {
            false
        };
        if output {
            let mut report = Box::new([0; REPORT_SIZE]);
            report[..2].copy_from_slice(&[2, 0x25]);
            Ok(Self::Output(report))
        } else {
            let mut report = [0; 32];
            report[..2].copy_from_slice(&[3, 0x27]);
            Ok(Self::Feature(report))
        }
    }
    pub fn send(&self, device: &HidDevice) -> Result<()> {
        match self {
            Self::Feature(report) => device
                .send_feature_report(report)
                .context("Galleon keepalive"),
            Self::Output(report) => {
                let size = device
                    .write(report.as_slice())
                    .context("Galleon keepalive")?;
                ensure!(size == REPORT_SIZE, "short Galleon keepalive write");
                Ok(())
            }
        }
    }
}

pub(super) struct Heartbeat {
    ping: Ping,
    last: Cell<Instant>,
}
impl Heartbeat {
    pub fn new(firmware: &str, now: Instant) -> Result<Self> {
        Ok(Self {
            ping: Ping::for_firmware(Some(firmware))?,
            last: Cell::new(now),
        })
    }
    pub fn poll(&self, now: Instant, send: impl FnOnce(&Ping) -> Result<()>) -> Result<()> {
        if now.saturating_duration_since(self.last.get()) >= INTERVAL {
            send(&self.ping)?;
            // A failed write does not suppress retries. The owning session
            // handles reconnect; this helper never resets the device.
            self.last.set(now);
        }
        Ok(())
    }
}

pub(super) fn serial(report: &[u8]) -> Result<String> {
    ensure!(
        report.len() == 32 && report[0] == 6,
        "invalid Galleon serial report"
    );
    let length = usize::from(report[1]).min(14);
    ensure!(length > 0, "empty Galleon serial");
    ascii_string(&report[2..2 + length])
}
pub(super) fn firmware(report: &[u8]) -> Result<String> {
    ensure!(
        report.len() == 32 && report[0] == 5 && report[1] > 4,
        "invalid Galleon firmware report"
    );
    let length = usize::from(report[1] - 4).min(8);
    let version = ascii_string(&report[6..6 + length])?;
    Ping::for_firmware(Some(&version))?;
    Ok(version)
}
pub(crate) fn brightness(percent: u8) -> Result<u8> {
    ensure!(percent <= 100, "brightness must be between 0 and 100");
    Ok((4 + u16::from(percent) * 96 / 100) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn firmware_gate_and_metadata_follow_native_bounds() {
        for (version, output) in [
            ("1.5.0", false),
            ("1.6.0", true),
            ("2.0.0", false),
            ("0.9.0.1", true),
        ] {
            let mut report = [0; 32];
            report[0] = 5;
            report[1] = (version.len() + 4) as u8;
            report[6..6 + version.len()].copy_from_slice(version.as_bytes());
            assert_eq!(firmware(&report).unwrap(), version);
            let ping = Ping::for_firmware(Some(version)).unwrap();
            assert_eq!(matches!(ping, Ping::Output(_)), output);
            match ping {
                Ping::Feature(r) => {
                    assert_eq!(&r[..2], &[3, 0x27]);
                    assert!(r[2..].iter().all(|x| *x == 0));
                }
                Ping::Output(r) => {
                    assert_eq!(&r[..2], &[2, 0x25]);
                    assert!(r[2..].iter().all(|x| *x == 0));
                }
            }
        }
        for version in ["", "1.6", "1.x.0", "-1.6.0"] {
            assert!(Ping::for_firmware(Some(version)).is_err());
        }
        let mut report = [b'X'; 32];
        report[0] = 6;
        report[1] = 30;
        report[2..16].copy_from_slice(b"EL123456789012");
        assert_eq!(serial(&report).unwrap(), "EL123456789012");
        assert!(serial(&report[..16]).is_err());
        report[0] = 5;
        report[1] = 3;
        assert!(firmware(&report).is_err());
        assert_eq!(
            (
                brightness(0).unwrap(),
                brightness(50).unwrap(),
                brightness(100).unwrap()
            ),
            (4, 52, 100)
        );
        assert!(brightness(101).is_err());
    }
    #[test]
    fn heartbeat_is_periodic_and_does_not_hide_failed_writes() {
        let now = Instant::now();
        let heartbeat = Heartbeat::new("1.6.0", now).unwrap();
        let mut count = 0;
        for ms in [0, 499, 500, 501, 999, 1000] {
            heartbeat
                .poll(now + Duration::from_millis(ms), |_| {
                    count += 1;
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(count, 2);
        assert!(
            heartbeat
                .poll(now + Duration::from_millis(1500), |_| bail!("disconnected"))
                .is_err()
        );
        heartbeat
            .poll(now + Duration::from_millis(1501), |_| {
                count += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(count, 3);
    }
}
