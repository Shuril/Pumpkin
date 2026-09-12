pub mod perlin;
pub mod simplex;

/// A 3D gradient vector used for noise calculations.
pub struct Gradient {
    /// The X component of the gradient vector.
    x: f64,
    /// The Y component of the gradient vector.
    y: f64,
    /// The Z component of the gradient vector.
    z: f64,
}

/// A pre-computed set of 16 gradient vectors for 3D noise generation.
pub const GRADIENTS: [Gradient; 16] = [
    Gradient {
        x: 1f64,
        y: 1f64,
        z: 0f64,
    },
    Gradient {
        x: -1f64,
        y: 1f64,
        z: 0f64,
    },
    Gradient {
        x: 1f64,
        y: -1f64,
        z: 0f64,
    },
    Gradient {
        x: -1f64,
        y: -1f64,
        z: 0f64,
    },
    Gradient {
        x: 1f64,
        y: 0f64,
        z: 1f64,
    },
    Gradient {
        x: -1f64,
        y: 0f64,
        z: 1f64,
    },
    Gradient {
        x: 1f64,
        y: 0f64,
        z: -1f64,
    },
    Gradient {
        x: -1f64,
        y: 0f64,
        z: -1f64,
    },
    Gradient {
        x: 0f64,
        y: 1f64,
        z: 1f64,
    },
    Gradient {
        x: 0f64,
        y: -1f64,
        z: 1f64,
    },
    Gradient {
        x: 0f64,
        y: 1f64,
        z: -1f64,
    },
    Gradient {
        x: 0f64,
        y: -1f64,
        z: -1f64,
    },
    Gradient {
        x: 1f64,
        y: 1f64,
        z: 0f64,
    },
    Gradient {
        x: 0f64,
        y: -1f64,
        z: 1f64,
    },
    Gradient {
        x: -1f64,
        y: 1f64,
        z: 0f64,
    },
    Gradient {
        x: 0f64,
        y: -1f64,
        z: -1f64,
    },
];

impl Gradient {
    /// Computes the dot product of this gradient vector with the given coordinates.
    ///
    /// # Arguments
    /// - `x` – The X coordinate to dot with.
    /// - `y` – The Y coordinate to dot with.
    /// - `z` – The Z coordinate to dot with.
    ///
    /// # Returns
    /// The dot product `self.x * x + self.y * y + self.z * z`.
    #[inline]
    #[must_use]
    pub const fn dot(&self, x: f64, y: f64, z: f64) -> f64 {
        // When using mul_add without target-feature=+fma, you get a huge performance cost
        // because it lowers into a libm call 16x per Perlin sample.
        //
        // This improves performance by something crazy like 15%
        self.x * x + self.y * y + self.z * z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gradients_length_and_magnitudes() {
        assert_eq!(GRADIENTS.len(), 16);
        for g in &GRADIENTS {
            let mag_sq = g.x * g.x + g.y * g.y + g.z * g.z;
            assert!((mag_sq - 2.0).abs() < 1e-9);
        }
    }

    #[test]
    fn test_gradient_dot_product() {
        let g = Gradient {
            x: 1.0,
            y: -1.0,
            z: 0.0,
        };
        assert_eq!(g.dot(2.0, 3.0, 4.0), -1.0);
    }
}

