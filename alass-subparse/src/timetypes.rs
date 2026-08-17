// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Time points, durations and time spans, all in whole milliseconds.

use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// The internal timing of `TimePoint` and `TimeDelta`, in milliseconds.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Timing(i64 /* number of milliseconds */);

impl Timing {
    fn from_components(hours: i64, mins: i64, secs: i64, ms: i64) -> Timing {
        Timing(ms + 1000 * (secs + 60 * (mins + 60 * hours)))
    }

    fn from_msecs(ms: i64) -> Timing {
        Timing(ms)
    }

    fn from_csecs(cs: i64) -> Timing {
        Timing(cs * 10)
    }

    fn from_secs(s: i64) -> Timing {
        Timing(s * 1000)
    }

    fn from_mins(mins: i64) -> Timing {
        Timing(mins * 1000 * 60)
    }

    fn from_hours(h: i64) -> Timing {
        Timing(h * 1000 * 60 * 60)
    }

    fn msecs(self) -> i64 {
        self.0
    }

    fn csecs(self) -> i64 {
        self.0 / 10
    }

    fn secs(self) -> i64 {
        self.0 / 1000
    }

    fn secs_f64(self) -> f64 {
        self.0 as f64 / 1000.0
    }

    fn mins(self) -> i64 {
        self.0 / (60 * 1000)
    }

    fn hours(self) -> i64 {
        self.0 / (60 * 60 * 1000)
    }

    fn mins_comp(self) -> i64 {
        self.mins() % 60
    }

    fn secs_comp(self) -> i64 {
        self.secs() % 60
    }

    fn csecs_comp(self) -> i64 {
        self.csecs() % 100
    }

    fn msecs_comp(self) -> i64 {
        self.msecs() % 1000
    }

    fn is_negative(self) -> bool {
        self.0 < 0
    }
}

impl Debug for Timing {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "Timing({self})")
    }
}

impl Display for Timing {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let t = if self.0 < 0 { -*self } else { *self };
        write!(
            f,
            "{}{}:{:02}:{:02}.{:03}",
            if self.0 < 0 { "-" } else { "" },
            t.hours(),
            t.mins_comp(),
            t.secs_comp(),
            t.msecs_comp()
        )
    }
}

impl Add for Timing {
    type Output = Timing;
    fn add(self, rhs: Timing) -> Timing {
        Timing(self.0 + rhs.0)
    }
}

impl Sub for Timing {
    type Output = Timing;
    fn sub(self, rhs: Timing) -> Timing {
        Timing(self.0 - rhs.0)
    }
}

impl AddAssign for Timing {
    fn add_assign(&mut self, r: Timing) {
        self.0 += r.0;
    }
}

impl SubAssign for Timing {
    fn sub_assign(&mut self, r: Timing) {
        self.0 -= r.0;
    }
}

impl Neg for Timing {
    type Output = Timing;
    fn neg(self) -> Timing {
        Timing(-self.0)
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
/// Represents a time point like the start time of a subtitle entry.
pub struct TimePoint {
    /// The internal timing (with all necessary functions and nice Debug information, etc.).
    intern: Timing,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
/// Represents a duration between two `TimePoints`.
pub struct TimeDelta {
    /// The internal timing (with all necessary functions and nice Debug information, etc.).
    intern: Timing,
}

macro_rules! create_time_type {
    ($i:ident) => {
        impl $i {
            fn new(t: Timing) -> $i {
                $i { intern: t }
            }

            /// Create this time type from all time components.
            ///
            /// The components can be negative and/or exceed the its natural limits without error.
            /// For example `from_components(0, 0, 3, -2000)` is the same as `from_components(0, 0, 1, 0)`.
            #[must_use]
            pub fn from_components(hours: i64, mins: i64, secs: i64, ms: i64) -> $i {
                Self::new(Timing::from_components(hours, mins, secs, ms))
            }

            /// Create the time type from a given number of milliseconds.
            #[must_use]
            pub fn from_msecs(ms: i64) -> $i {
                Self::new(Timing::from_msecs(ms))
            }

            /// Create the time type from a given number of hundreth seconds (10 milliseconds).
            #[must_use]
            pub fn from_csecs(ms: i64) -> $i {
                Self::new(Timing::from_csecs(ms))
            }

            /// Create the time type with a given number of seconds.
            #[must_use]
            pub fn from_secs(ms: i64) -> $i {
                Self::new(Timing::from_secs(ms))
            }

            /// Create the time type with a given number of minutes.
            #[must_use]
            pub fn from_mins(mins: i64) -> $i {
                Self::new(Timing::from_mins(mins))
            }

            /// Create the time type with a given number of hours.
            #[must_use]
            pub fn from_hours(mins: i64) -> $i {
                Self::new(Timing::from_hours(mins))
            }

            /// Get the total number of milliseconds.
            #[must_use]
            pub fn msecs(self) -> i64 {
                self.intern.msecs()
            }

            /// Get the total number of hundreth seconds.
            #[must_use]
            pub fn csecs(self) -> i64 {
                self.intern.csecs()
            }

            /// Get the total number of seconds.
            #[must_use]
            pub fn secs(self) -> i64 {
                self.intern.secs()
            }

            /// Get the total number of seconds as a floating point number.
            #[must_use]
            pub fn secs_f64(self) -> f64 {
                self.intern.secs_f64()
            }

            /// Get the total number of minutes.
            #[must_use]
            pub fn mins(self) -> i64 {
                self.intern.mins()
            }

            /// Get the total number of hours.
            #[must_use]
            pub fn hours(self) -> i64 {
                self.intern.hours()
            }

            /// Get the milliseconds component in a range of [0, 999].
            #[must_use]
            pub fn msecs_comp(self) -> i64 {
                self.intern.msecs_comp()
            }

            /// Get the hundreths seconds component in a range of [0, 99].
            #[must_use]
            pub fn csecs_comp(self) -> i64 {
                self.intern.csecs_comp()
            }

            /// Get the seconds component in a range of [0, 59].
            #[must_use]
            pub fn secs_comp(self) -> i64 {
                self.intern.secs_comp()
            }

            /// Get the minute component in a range of [0, 59].
            #[must_use]
            pub fn mins_comp(self) -> i64 {
                self.intern.mins_comp()
            }

            /// Return `true` if the represented time is negative.
            #[must_use]
            pub fn is_negative(self) -> bool {
                self.intern.is_negative()
            }

            /// Return the absolute value of the current time.
            #[must_use]
            pub fn abs(self) -> $i {
                if self.is_negative() { -self } else { self }
            }
        }

        impl Neg for $i {
            type Output = $i;
            fn neg(self) -> $i {
                $i::new(-self.intern)
            }
        }

        impl Display for $i {
            fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
                write!(f, "{}", self.intern)
            }
        }
    };
}

create_time_type! {TimePoint}
create_time_type! {TimeDelta}

macro_rules! impl_add {
    ($a:ty, $b:ty, $output:ident) => {
        impl Add<$b> for $a {
            type Output = $output;
            fn add(self, rhs: $b) -> $output {
                $output::new(self.intern + rhs.intern)
            }
        }
    };
}

macro_rules! impl_sub {
    ($a:ty, $b:ty, $output:ident) => {
        impl Sub<$b> for $a {
            type Output = $output;
            fn sub(self, rhs: $b) -> $output {
                $output::new(self.intern - rhs.intern)
            }
        }
    };
}

macro_rules! impl_add_assign {
    ($a:ty, $b:ty) => {
        impl AddAssign<$b> for $a {
            fn add_assign(&mut self, r: $b) {
                self.intern += r.intern;
            }
        }
    };
}

macro_rules! impl_sub_assign {
    ($a:ty, $b:ty) => {
        impl SubAssign<$b> for $a {
            fn sub_assign(&mut self, r: $b) {
                self.intern -= r.intern;
            }
        }
    };
}

impl_add!(TimeDelta, TimeDelta, TimeDelta);
impl_add!(TimePoint, TimeDelta, TimePoint);
impl_add!(TimeDelta, TimePoint, TimePoint);

impl_sub!(TimeDelta, TimeDelta, TimeDelta);
impl_sub!(TimePoint, TimePoint, TimeDelta);
impl_sub!(TimePoint, TimeDelta, TimePoint);
impl_sub!(TimeDelta, TimePoint, TimePoint);

impl_add_assign!(TimeDelta, TimeDelta);
impl_add_assign!(TimePoint, TimeDelta);

impl_sub_assign!(TimeDelta, TimeDelta);
impl_sub_assign!(TimePoint, TimeDelta);

/// A time span (e.g. time in which a subtitle is shown).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeSpan {
    /// Start of the time span.
    pub start: TimePoint,

    /// End of the time span.
    pub end: TimePoint,
}

impl TimeSpan {
    /// Constructor of `TimeSpan`s.
    #[must_use]
    pub fn new(start: TimePoint, end: TimePoint) -> TimeSpan {
        TimeSpan { start, end }
    }

    /// Get the length of the `TimeSpan` (can be negative).
    #[must_use]
    pub fn len(self) -> TimeDelta {
        self.end - self.start
    }

    /// Returns `true` if the `TimeSpan` has zero length.
    ///
    /// Note that a `TimeSpan` whose end lies before its start is *not* empty; use
    /// [`TimeSpan::len`] if you need to tell those apart.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

impl Add<TimeDelta> for TimeSpan {
    type Output = TimeSpan;
    fn add(self, rhs: TimeDelta) -> TimeSpan {
        TimeSpan::new(self.start + rhs, self.end + rhs)
    }
}

impl Sub<TimeDelta> for TimeSpan {
    type Output = TimeSpan;
    fn sub(self, rhs: TimeDelta) -> TimeSpan {
        TimeSpan::new(self.start - rhs, self.end - rhs)
    }
}

impl AddAssign<TimeDelta> for TimeSpan {
    fn add_assign(&mut self, r: TimeDelta) {
        self.start += r;
        self.end += r;
    }
}

impl SubAssign<TimeDelta> for TimeSpan {
    fn sub_assign(&mut self, r: TimeDelta) {
        self.start -= r;
        self.end -= r;
    }
}

#[cfg(test)]
mod tests {
    use super::{TimeDelta, TimePoint, TimeSpan, Timing};

    #[test]
    fn test_timing_display() {
        let t = -Timing::from_components(12, 59, 29, 450);
        assert_eq!(t.to_string(), "-12:59:29.450");

        let t = Timing::from_msecs(0);
        assert_eq!(t.to_string(), "0:00:00.000");

        assert_eq!(format!("{t:?}"), "Timing(0:00:00.000)");
    }

    #[test]
    fn components_round_trip() {
        let t = TimePoint::from_components(1, 2, 3, 456);
        assert_eq!(t.msecs(), 3_723_456);
        assert_eq!(
            (t.hours(), t.mins_comp(), t.secs_comp(), t.msecs_comp()),
            (1, 2, 3, 456)
        );
        assert_eq!(t.csecs_comp(), 45);
        assert_eq!(TimePoint::from_csecs(150).msecs(), 1500);
        assert_eq!(TimePoint::from_secs(2).msecs(), 2000);
        assert_eq!(TimePoint::from_mins(2).msecs(), 120_000);
        assert_eq!(TimePoint::from_hours(2).msecs(), 7_200_000);
        assert_eq!(TimePoint::from_msecs(-5).secs_f64(), -0.005);
    }

    #[test]
    fn abs_and_is_negative() {
        assert!(TimePoint::from_msecs(-1).is_negative());
        assert!(!TimePoint::from_msecs(0).is_negative());
        assert_eq!(TimePoint::from_msecs(-7).abs(), TimePoint::from_msecs(7));
        assert_eq!(TimeDelta::from_msecs(7).abs(), TimeDelta::from_msecs(7));
    }

    /// `subparse 0.7.0` implemented `SubAssign` by *adding*, so `-=` moved every
    /// time type in the wrong direction.
    #[test]
    fn sub_assign_subtracts() {
        let mut d = TimeDelta::from_msecs(1000);
        d -= TimeDelta::from_msecs(400);
        assert_eq!(d, TimeDelta::from_msecs(600));

        let mut p = TimePoint::from_msecs(1000);
        p -= TimeDelta::from_msecs(400);
        assert_eq!(p, TimePoint::from_msecs(600));

        let mut s = TimeSpan::new(TimePoint::from_msecs(1000), TimePoint::from_msecs(2000));
        s -= TimeDelta::from_msecs(400);
        assert_eq!(
            s,
            TimeSpan::new(TimePoint::from_msecs(600), TimePoint::from_msecs(1600))
        );
        assert_eq!(
            s - TimeDelta::from_msecs(600),
            TimeSpan::new(TimePoint::from_msecs(0), TimePoint::from_msecs(1000))
        );
    }

    #[test]
    fn add_assign_adds() {
        let mut s = TimeSpan::new(TimePoint::from_msecs(1000), TimePoint::from_msecs(2000));
        s += TimeDelta::from_msecs(400);
        assert_eq!(
            s,
            TimeSpan::new(TimePoint::from_msecs(1400), TimePoint::from_msecs(2400))
        );
    }

    #[test]
    fn arithmetic_between_the_types() {
        let a = TimePoint::from_msecs(1000);
        let b = TimePoint::from_msecs(2500);
        assert_eq!(b - a, TimeDelta::from_msecs(1500));
        assert_eq!(a + TimeDelta::from_msecs(500), TimePoint::from_msecs(1500));
        assert_eq!(TimeDelta::from_msecs(500) + a, TimePoint::from_msecs(1500));
        assert_eq!(TimeDelta::from_msecs(500) - a, TimePoint::from_msecs(-500));
        assert!(a < b);
    }

    #[test]
    fn timespan_len_and_is_empty() {
        let s = TimeSpan::new(TimePoint::from_msecs(1000), TimePoint::from_msecs(2500));
        assert_eq!(s.len(), TimeDelta::from_msecs(1500));
        assert!(!s.is_empty());
        assert!(TimeSpan::new(TimePoint::from_msecs(1000), TimePoint::from_msecs(1000)).is_empty());
    }

    #[test]
    fn display_is_stable() {
        assert_eq!(TimeDelta::from_msecs(-1234).to_string(), "-0:00:01.234");
        assert_eq!(TimePoint::from_msecs(3_723_456).to_string(), "1:02:03.456");
    }
}
