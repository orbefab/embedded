/// A point on the simulation timeline, in integer picoseconds from t = 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize), serde(transparent))]
pub struct Time(u64);

impl Time {
    pub const ZERO: Time = Time(0);

    pub const fn from_ps(ps: u64) -> Time {
        Time(ps)
    }

    pub const fn as_ps(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_before_a_later_picosecond() {
        assert_eq!(Time::ZERO.as_ps(), 0);
        assert!(Time::ZERO < Time::from_ps(1));
        assert_eq!(Time::from_ps(42).as_ps(), 42);
    }
}
