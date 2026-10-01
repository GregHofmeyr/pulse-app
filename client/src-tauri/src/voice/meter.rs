//! RMS meter, read-and-reset.

/// Running sum of squares; `take` returns (rms, samples) and resets.
#[derive(Default, Debug)]
pub struct Meter {
    sq: f64,
    n: u64,
}

impl Meter {
    pub fn add(&mut self, v: f32) {
        self.sq += (v as f64) * (v as f64);
        self.n += 1;
    }

    pub fn take(&mut self) -> (f32, u64) {
        let rms = if self.n == 0 {
            0.0
        } else {
            (self.sq / self.n as f64).sqrt() as f32
        };
        let n = self.n;
        *self = Meter::default();
        (rms, n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_constant() {
        let mut m = Meter::default();
        for _ in 0..100 {
            m.add(0.5);
        }
        let (rms, n) = m.take();
        assert!((rms - 0.5).abs() < 1e-6);
        assert_eq!(n, 100);
        assert_eq!(m.take(), (0.0, 0));
    }
}
