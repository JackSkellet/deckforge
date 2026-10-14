//! A separate uinput pointer keeps existing keyboard device capabilities unchanged.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    mem::{offset_of, size_of},
    os::fd::AsRawFd,
    time::Duration,
};
pub struct Mouse(File);
fn checked(value: i32) -> Result<()> {
    if value < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
impl Mouse {
    pub fn new() -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .context("Open /dev/uinput for scrolling")?;
        let fd = file.as_raw_fd();
        unsafe {
            checked(libc::ioctl(fd, libc::_IOW::<i32>(b'U' as u32, 100), 1))?;
            checked(libc::ioctl(fd, libc::_IOW::<i32>(b'U' as u32, 100), 2))?;
            checked(libc::ioctl(fd, libc::_IOW::<i32>(b'U' as u32, 101), 0x110))?;
            for axis in [0, 1, 6, 8] {
                checked(libc::ioctl(fd, libc::_IOW::<i32>(b'U' as u32, 102), axis))?;
            }
            let mut setup: libc::uinput_setup = std::mem::zeroed();
            setup.id.bustype = 6;
            setup.id.version = 1;
            for (out, ch) in setup.name.iter_mut().zip(b"Deckforge scroll wheel") {
                *out = *ch as libc::c_char;
            }
            checked(libc::ioctl(
                fd,
                libc::_IOW::<libc::uinput_setup>(b'U' as u32, 3),
                &setup,
            ))?;
            checked(libc::ioctl(fd, libc::_IO(b'U' as u32, 1)))?;
        }
        let mouse = Self(file);
        std::thread::sleep(Duration::from_secs(1));
        Ok(mouse)
    }
    pub fn scroll(&mut self, ticks: i32, horizontal: bool) -> Result<()> {
        self.0
            .write_all(&frame(ticks, horizontal)?)
            .context("Send scroll event")
    }
}
impl Drop for Mouse {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.0.as_raw_fd(), libc::_IO(b'U' as u32, 2));
        }
    }
}
fn frame(ticks: i32, horizontal: bool) -> Result<Vec<u8>> {
    ensure!(ticks.unsigned_abs() <= 12700, "Scroll event too large");
    let size = size_of::<libc::input_event>();
    let mut bytes = vec![0; size * 2];
    let ty = offset_of!(libc::input_event, type_);
    bytes[ty..ty + 2].copy_from_slice(&2u16.to_ne_bytes());
    let code = offset_of!(libc::input_event, code);
    bytes[code..code + 2].copy_from_slice(&(if horizontal { 6u16 } else { 8u16 }).to_ne_bytes());
    let value = offset_of!(libc::input_event, value);
    bytes[value..value + 4].copy_from_slice(&ticks.to_ne_bytes());
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_wheel_axes_and_sync_are_encoded() {
        for (ticks, h, code) in [(-3, false, 8u16), (5, true, 6)] {
            let f = frame(ticks, h).unwrap();
            let c = offset_of!(libc::input_event, code);
            let v = offset_of!(libc::input_event, value);
            assert_eq!(&f[c..c + 2], &code.to_ne_bytes());
            assert_eq!(&f[v..v + 4], &ticks.to_ne_bytes());
            assert!(f[size_of::<libc::input_event>()..].iter().all(|b| *b == 0));
        }
    }
}
