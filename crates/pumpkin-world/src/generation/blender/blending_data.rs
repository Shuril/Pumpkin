use pumpkin_data::chunk::Biome;

#[derive(Clone)]
pub struct BlendingData {
    /// Block-space minimum and exclusive maximum of the old-generation area.
    pub min_y: i32,
    pub max_y: i32,
    /// Heights at the 16 vanilla border/interior quart columns. Vanilla's
    /// packed representation is not a 16x16 grid: it stores the seven
    /// interior-edge columns plus the nine outside-edge columns.
    pub heights: Vec<f64>,
    /// Density columns in the same 16-column order. Empty columns are valid
    /// for packed data loaded from disk; they are populated by the blender
    /// when an old chunk is sampled.
    pub densities: Vec<Vec<f64>>,
    /// Biome columns in vanilla quart-Y order. Packed blending data does not
    /// persist these, so a loaded chunk starts with empty columns.
    pub biomes: Vec<Vec<&'static Biome>>,
}

impl BlendingData {
    const INSIDE_MAX: i32 = 3;
    const OUTSIDE_MAX: i32 = 4;
    const INSIDE_COUNT: usize = 7;
    const COLUMN_COUNT: usize = 16;
    const NO_VALUE: f64 = f64::MAX;

    #[must_use]
    pub fn from_packed(min_section: i32, max_section: i32, heights: Option<Vec<f64>>) -> Self {
        let mut normalized = vec![Self::NO_VALUE; Self::COLUMN_COUNT];
        if let Some(values) = heights {
            for (dst, value) in normalized.iter_mut().zip(values) {
                *dst = value;
            }
        }
        Self {
            min_y: min_section * 16,
            max_y: max_section * 16,
            heights: normalized,
            densities: vec![Vec::new(); Self::COLUMN_COUNT],
            biomes: vec![Vec::new(); Self::COLUMN_COUNT],
        }
    }

    /// Encodes the subset represented by Mojang's `BlendingData.Packed` codec.
    #[must_use]
    pub fn to_packed_nbt(&self) -> pumpkin_nbt::compound::NbtCompound {
        let mut data = pumpkin_nbt::compound::NbtCompound::new();
        data.put_int("min_section", self.min_y.div_euclid(16));
        data.put_int("max_section", self.max_y.div_euclid(16));
        if self.heights.iter().any(|value| *value != Self::NO_VALUE) {
            data.put_list(
                "heights",
                self.heights.iter().copied().map(Into::into).collect(),
            );
        }
        data
    }

    fn index(cell_x: i32, cell_z: i32) -> Option<usize> {
        if cell_x == Self::OUTSIDE_MAX || cell_z == Self::OUTSIDE_MAX {
            let offset = Self::INSIDE_COUNT as i32 + cell_x + Self::OUTSIDE_MAX - cell_z;
            return usize::try_from(offset)
                .ok()
                .filter(|i| *i < Self::COLUMN_COUNT);
        }
        if cell_x == 0 || cell_z == 0 {
            let offset = Self::INSIDE_MAX - cell_x + cell_z;
            return usize::try_from(offset)
                .ok()
                .filter(|i| *i < Self::INSIDE_COUNT);
        }
        None
    }

    fn column_coords(index: usize) -> (i32, i32) {
        if index < Self::INSIDE_COUNT {
            let index = index as i32;
            (
                (Self::INSIDE_MAX - index).max(0),
                (index - Self::INSIDE_MAX).max(0),
            )
        } else {
            let offset = index as i32 - Self::INSIDE_COUNT as i32;
            (
                Self::OUTSIDE_MAX - (Self::OUTSIDE_MAX - offset).max(0),
                Self::OUTSIDE_MAX - (offset - Self::OUTSIDE_MAX).max(0),
            )
        }
    }

    #[must_use]
    pub fn get_height(&self, cell_x: i32, _cell_y: i32, cell_z: i32) -> f64 {
        Self::index(cell_x, cell_z)
            .and_then(|index| self.heights.get(index).copied())
            .unwrap_or(Self::NO_VALUE)
    }

    #[must_use]
    pub fn get_density(&self, cell_x: i32, cell_y: i32, cell_z: i32) -> f64 {
        let Some(index) = Self::index(cell_x, cell_z) else {
            return Self::NO_VALUE;
        };
        let cell_count_y = (self.max_y - self.min_y) / 8;
        if !(0..cell_count_y).contains(&cell_y) {
            return Self::NO_VALUE;
        }
        self.densities
            .get(index)
            .and_then(|column| column.get(cell_y as usize))
            .copied()
            .map_or(Self::NO_VALUE, |density| density * 0.1)
    }

    pub fn iterate_heights<F>(&self, quart_x: i32, quart_z: i32, mut consumer: F)
    where
        F: FnMut(i32, i32, f64),
    {
        for (index, h) in self.heights.iter().copied().enumerate() {
            if h != Self::NO_VALUE {
                let (x, z) = Self::column_coords(index);
                consumer(quart_x + x, quart_z + z, h);
            }
        }
    }

    pub fn iterate_densities<F>(
        &self,
        quart_x: i32,
        quart_z: i32,
        min_cell_y: i32,
        max_cell_y: i32,
        mut consumer: F,
    ) where
        F: FnMut(i32, i32, i32, f64),
    {
        let cell_count_y = (self.max_y - self.min_y) / 8;
        for cell_y in min_cell_y..=max_cell_y {
            if (0..cell_count_y).contains(&cell_y) {
                for (index, column) in self.densities.iter().enumerate() {
                    if let Some(&density) = column.get(cell_y as usize) {
                        if density != Self::NO_VALUE {
                            let (x, z) = Self::column_coords(index);
                            consumer(quart_x + x, cell_y, quart_z + z, density * 0.1);
                        }
                    }
                }
            }
        }
    }

    pub fn iterate_biomes<F>(&self, quart_x: i32, _quart_y: i32, quart_z: i32, mut consumer: F)
    where
        F: FnMut(i32, i32, &'static Biome),
    {
        for (index, column) in self.biomes.iter().enumerate() {
            let (x, z) = Self::column_coords(index);
            if let Some(biome) = column.get(0).copied() {
                consumer(quart_x + x, quart_z + z, biome);
            }
        }
    }
}
