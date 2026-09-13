use super::RandomImpl;

/// A trait extending `RandomImpl` with Gaussian (normal) distribution generation capabilities.
pub trait GaussianGenerator: RandomImpl {
    /// Returns the stored Gaussian value from a previous calculation, if available.
    ///
    /// # Returns
    /// The previously stored Gaussian value, or `None` if no value is stored.
    fn stored_next_gaussian(&self) -> Option<f64>;

    /// Sets the stored Gaussian value for the next call.
    ///
    /// # Arguments
    /// - `value` – The Gaussian value to store, or `None` to clear the storage.
    fn set_stored_next_gaussian(&mut self, value: Option<f64>);

    /// Generates the next Gaussian-distributed random value.
    ///
    /// # Returns
    /// A random value from a standard Gaussian (normal) distribution.
    fn calculate_gaussian(&mut self) -> f64 {
        if let Some(gaussian) = self.stored_next_gaussian() {
            self.set_stored_next_gaussian(None);
            gaussian
        } else {
            loop {
                let d = self.next_f64().mul_add(2.0, -1.0);
                let e = self.next_f64().mul_add(2.0, -1.0);
                let f = d * d + e * e;

                if f < 1f64 && f != 0f64 {
                    let g = (-2f64 * f.ln() / f).sqrt();
                    self.set_stored_next_gaussian(Some(e * g));
                    return d * g;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockRandom {
        values: Vec<f64>,
        index: usize,
        stored: Option<f64>,
    }

    impl MockRandom {
        fn new(values: Vec<f64>) -> Self {
            Self {
                values,
                index: 0,
                stored: None,
            }
        }
    }

    impl RandomImpl for MockRandom {
        fn split(&mut self) -> Self {
            Self::new(self.values.clone())
        }

        fn next_splitter(&mut self) -> crate::random::RandomDeriver {
            crate::random::RandomDeriver::Xoroshiro(
                crate::random::xoroshiro128::Xoroshiro::from_seed(0).next_splitter(),
            )
        }

        fn next_i32(&mut self) -> i32 {
            self.next_f64() as i32
        }

        fn next_bounded_i32(&mut self, _bound: i32) -> i32 {
            0
        }

        fn next_i64(&mut self) -> i64 {
            0
        }

        fn next_bool(&mut self) -> bool {
            false
        }

        fn next_f32(&mut self) -> f32 {
            self.next_f64() as f32
        }

        fn next_f64(&mut self) -> f64 {
            let val = self.values[self.index % self.values.len()];
            self.index += 1;
            val
        }

        fn next_gaussian(&mut self) -> f64 {
            self.calculate_gaussian()
        }
    }

    impl GaussianGenerator for MockRandom {
        fn stored_next_gaussian(&self) -> Option<f64> {
            self.stored
        }

        fn set_stored_next_gaussian(&mut self, value: Option<f64>) {
            self.stored = value;
        }
    }

    #[test]
    fn test_gaussian_storage_and_pair_generation() {
        let mut mock = MockRandom::new(vec![0.5, 0.75]);
        assert!(mock.stored_next_gaussian().is_none());

        let g1 = mock.calculate_gaussian();
        assert_eq!(g1, 0.0);
        assert!(mock.stored_next_gaussian().is_some());

        let g2 = mock.calculate_gaussian();
        assert!(g2 != 0.0);
        assert!(mock.stored_next_gaussian().is_none());
    }

    #[test]
    fn test_stored_gaussian_override() {
        let mut mock = MockRandom::new(vec![0.5, 0.75]);
        mock.set_stored_next_gaussian(Some(42.5));
        assert_eq!(mock.stored_next_gaussian(), Some(42.5));
        assert_eq!(mock.calculate_gaussian(), 42.5);
        assert_eq!(mock.stored_next_gaussian(), None);
    }
}
