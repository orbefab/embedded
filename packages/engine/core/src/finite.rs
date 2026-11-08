use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

/// An f64 that is neither NaN nor infinite, with -0.0 folded into 0.0.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Finite(f64);

impl Finite {
    pub fn new(x: f64) -> Option<Finite> {
        // -0.0 + 0.0 == +0.0 under round-to-nearest.
        x.is_finite().then_some(Finite(x + 0.0))
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for Finite {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(deserializer)?;
        Finite::new(value).ok_or_else(|| de::Error::custom("number must be finite"))
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[test]
    fn rejects_non_finite() {
        assert!(Finite::new(f64::NAN).is_none());
        assert!(Finite::new(f64::INFINITY).is_none());
        assert!(Finite::new(f64::NEG_INFINITY).is_none());
    }

    #[test]
    fn folds_negative_zero() {
        let zero = Finite::new(-0.0).unwrap();
        assert_eq!(zero.get().to_bits(), 0.0f64.to_bits());
        assert_eq!(serde_json::to_string(&zero).unwrap(), "0.0");
    }

    #[test]
    fn deserialize_folds_negative_zero_and_rejects_non_finite() {
        let zero: Finite = serde_json::from_str("-0.0").unwrap();
        assert_eq!(zero.get().to_bits(), 0.0f64.to_bits());
        assert!(serde_json::from_str::<Finite>("1e309").is_err());
        let infinite =
            serde::de::value::F64Deserializer::<serde::de::value::Error>::new(f64::INFINITY);
        let nan = serde::de::value::F64Deserializer::<serde::de::value::Error>::new(f64::NAN);
        assert!(Finite::deserialize(infinite).is_err());
        assert!(Finite::deserialize(nan).is_err());
    }
}
