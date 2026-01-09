//! Hardware clock handling for linux
//!
//! The `hwclock` module provides a thin wrapper around kernel structs and
//! ioctls to retrieve the current time from the hardware clock and convert it
//! from and to a valid `chrono` data structure.
//!
//! ```rust,no_run
//! extern crate chrono;
//! extern crate hwclock;
//!
//! fn main() {
//!     use hwclock::HwClockDev;
//!
//!     let rtc = HwClockDev::open("/dev/rtc0").expect("could not open rtc clock");
//!
//!     println!("{:?}", rtc);
//!
//!     let time = rtc.get_time().expect("could not read rtc clock");
//!     println!("{:?}", time);
//!
//!     println!("Setting clock ahead 30 seconds");
//!     let mut ct: chrono::NaiveDateTime = time.try_into().expect("conversion failed");
//!     ct += chrono::Duration::seconds(30);
//!
//!     // convert back to RtcTime and set it
//!     let ntime = ct.into();
//!     rtc.set_time(&ntime).expect("could not set rtc clock");
//!
//!     println!("Rereading...");
//!     let time2 = rtc.get_time().expect("could not read rtc clock");
//!
//!     println!("{:?}", time2);
//! }
//! ```

use chrono::{Datelike, Timelike};

use libc::c_int;
use std::os::unix::io::AsRawFd;
use std::{fs, io, path};

/// Basic epoch for dates.
///
/// All dates returned by the hardware clock are in offset of the epoch year,
/// which usually is 1900 (e.g. a value of `118` on `RtcTime::tm_year`
/// marks the year 2018).
pub const YEAR_EPOCH: i32 = 1900;

mod ffi {
    use super::RtcTime;

    // ioctls, stolen from linux/rtc.h
    nix::ioctl_read!(rtc_rd_time, 'p', 0x09, RtcTime);
    nix::ioctl_write_ptr!(rtc_set_time, 'p', 0x0a, RtcTime);
}

/// Linux `struct rtc_time` wrapper
///
/// This structure is slightly shorter than other commonly used `struct tm*`.
/// It is assumed that the Rtc is kept at UTC.
///
/// Note that the resolution of the time struct is only seconds.
///
/// Conversion from and to `chrono::NaiveDateTime` is supported, any resolution
/// beyond seconds will silently be discarded without rounding.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default, Eq, PartialEq)]
pub struct RtcTime {
    /// Seconds
    pub tm_sec: c_int,
    /// Minutes
    pub tm_min: c_int,
    /// Hours
    pub tm_hour: c_int,
    /// Day of the month (1-31)
    pub tm_mday: c_int,
    /// Months since January (0-11)
    pub tm_mon: c_int,
    /// Years since `YEAR_EPOCH` (1900)
    pub tm_year: c_int,
    /// unused, should be set to 0
    pub tm_wday: c_int,
    /// unused, should be set to 0
    pub tm_yday: c_int,
    /// unused, should be set to 0
    pub tm_isdst: c_int,
}

impl TryFrom<RtcTime> for chrono::NaiveDateTime {
    type Error = ();

    fn try_from(rtc: RtcTime) -> Result<chrono::NaiveDateTime, Self::Error> {
        let d = chrono::NaiveDate::from_ymd_opt(
            rtc.tm_year + YEAR_EPOCH,
            (rtc.tm_mon + 1) as u32,
            rtc.tm_mday as u32,
        )
        .ok_or(())?;
        let t = chrono::NaiveTime::from_hms_opt(
            rtc.tm_hour as u32,
            rtc.tm_min as u32,
            rtc.tm_sec as u32,
        )
        .ok_or(())?;
        Ok(chrono::NaiveDateTime::new(d, t))
    }
}

impl From<chrono::NaiveDateTime> for RtcTime {
    fn from(ct: chrono::NaiveDateTime) -> RtcTime {
        RtcTime {
            tm_sec: ct.time().second() as i32,
            tm_min: ct.time().minute() as i32,
            tm_hour: ct.time().hour() as i32,
            tm_mday: ct.date().day() as i32,
            tm_mon: ct.date().month0() as i32,
            tm_year: ct.date().year() - YEAR_EPOCH,

            ..RtcTime::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_from_rtctime() {
        // Mon Feb 19 15:06:01 CET 2018
        // == Mon Feb 19 14:06:01 UTC 2018

        let rtc = RtcTime {
            tm_sec: 1,
            tm_min: 6,
            tm_hour: 14,
            tm_mday: 19,
            tm_mon: 1,
            tm_year: 118,
            tm_wday: 0,
            tm_yday: 0,
            tm_isdst: 0,
        };

        let ct = chrono::NaiveDateTime::new(
            chrono::NaiveDate::from_ymd_opt(2018, 2, 19)
                .expect("date creation from literals failed"),
            chrono::NaiveTime::from_hms_opt(14, 6, 1).expect("time creation from literals failed"),
        );

        let rtc_from_ct: RtcTime = ct.into();
        let ct_from_rtc: chrono::NaiveDateTime = rtc
            .try_into()
            .expect("converting RtcTime to NaiveDateTime failed");

        assert_eq!(rtc, rtc_from_ct);
        assert_eq!(ct, ct_from_rtc);
    }

    #[test]
    fn bindgen_test_layout_rtc_time() {
        assert_eq!(
            ::std::mem::size_of::<RtcTime>(),
            36usize,
            concat!("Size of: ", stringify!(RtcTime))
        );
        assert_eq!(
            ::std::mem::align_of::<RtcTime>(),
            4usize,
            concat!("Alignment of ", stringify!(RtcTime))
        );
    }
}

/// Hardware clock
///
/// Wraps an open hardware clock, usually found at `/dev/rtc` or `/dev/rtc0`.
#[derive(Debug)]
pub struct HwClockDev {
    // we store a full file instead of the raw fd, allowing us to print the
    // name of the clock using the derived debug impl
    clk: fs::File,
}

impl HwClockDev {
    /// Open clock
    ///
    /// The device node will be held open until the `HwClockDev` is dropped
    pub fn open<P: AsRef<path::Path>>(dev: P) -> io::Result<HwClockDev> {
        Ok(HwClockDev {
            clk: fs::File::open(dev)?,
        })
    }

    /// Get hardware clocks time
    pub fn get_time(&self) -> Result<RtcTime, nix::Error> {
        let mut time = RtcTime::default();

        assert_eq!(0, unsafe {
            ffi::rtc_rd_time(self.clk.as_raw_fd(), &mut time as *mut RtcTime)
        }?);

        Ok(time)
    }

    /// Set hardware clocks time
    pub fn set_time(&self, time: &RtcTime) -> Result<(), nix::Error> {
        assert_eq!(0, unsafe {
            ffi::rtc_set_time(self.clk.as_raw_fd(), time as *const RtcTime)
        }?);

        Ok(())
    }
}
