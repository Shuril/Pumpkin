use crate::WritingError;
use crate::codec::bit_set::BitSet;
use crate::{ClientPacket, VarInt, ser::NetworkWriteExt};
use pumpkin_data::packet::clientbound::PLAY_LIGHT_UPDATE;
use pumpkin_macros::java_packet;
use pumpkin_util::version::JavaMinecraftVersion;
use pumpkin_world::chunk::format::LightContainer;
use pumpkin_world::chunk::{ChunkData, ChunkLight};
use std::io::Write;

/// Builds the four masks and the list of sections which carry a 2048-byte
/// nibble array for the Java light packet.
///
/// Vanilla numbers the light sections from `minSection - 1`, therefore bit 0
/// is the synthetic section below the world, real section `i` is bit `i + 1`,
/// and bit `section_count + 1` is the synthetic section above the world.  The
/// old implementation started real sections at bit 0, which makes clients
/// apply every update one section too low (and is especially visible around
/// the world bottom).  The masks are variable-sized so custom dimensions with
/// more than 64 sections remain valid instead of overflowing a `u64` shift.
fn build_light_mask(
    sections: &[LightContainer],
    section_count: usize,
    changed_sections: Option<&[usize]>,
) -> (BitSet, BitSet, Vec<usize>) {
    let word_count = (section_count + 2).div_ceil(64).max(1);
    let mut full = vec![0_i64; word_count];
    let mut empty = vec![0_i64; word_count];
    let mut full_sections = Vec::new();

    let set_bit = |mask: &mut [i64], bit: usize| {
        mask[bit / 64] |= 1_i64 << (bit % 64);
    };

    // The section below the minimum world Y is always an empty data layer.
    set_bit(&mut empty, 0);

    for section_index in 0..section_count {
        if let Some(changed) = changed_sections
            && !changed.contains(&section_index)
        {
            continue;
        }
        let bit = section_index + 1;
        match sections.get(section_index) {
            Some(LightContainer::Full(_)) => {
                set_bit(&mut full, bit);
                full_sections.push(section_index);
            }
            // Missing sections are treated like the empty default layer. This
            // keeps malformed/partially loaded chunks serializable and avoids
            // an update packet panic while the chunk is being promoted.
            None | Some(LightContainer::Empty(_)) => set_bit(&mut empty, bit),
        }
    }

    // The section above the maximum world Y is also always empty.
    set_bit(&mut empty, section_count + 1);

    (
        BitSet(full.into_boxed_slice()),
        BitSet(empty.into_boxed_slice()),
        full_sections,
    )
}

/// Serializes the light payload shared by `LightUpdate` and
/// `LevelChunkWithLight`. Keeping one implementation is important: a client
/// must interpret a live update exactly like the initial chunk packet.
pub(crate) fn write_light_data(
    write: &mut impl Write,
    light_engine: &ChunkLight,
) -> Result<(), WritingError> {
    write_light_data_filtered(write, light_engine, None)
}

pub(crate) fn write_light_data_filtered(
    write: &mut impl Write,
    light_engine: &ChunkLight,
    changed_sections: Option<&[usize]>,
) -> Result<(), WritingError> {
    let section_count = light_engine
        .sky_light
        .len()
        .max(light_engine.block_light.len());
    let (sky_mask, sky_empty, sky_sections) =
        build_light_mask(&light_engine.sky_light, section_count, changed_sections);
    let (block_mask, block_empty, block_sections) =
        build_light_mask(&light_engine.block_light, section_count, changed_sections);

    write.write_bitset(&sky_mask)?;
    write.write_bitset(&block_mask)?;
    write.write_bitset(&sky_empty)?;
    write.write_bitset(&block_empty)?;

    let light_data_size = VarInt(LightContainer::ARRAY_SIZE as i32);

    write.write_var_int(&VarInt(sky_sections.len() as i32))?;
    for section_index in sky_sections {
        let Some(LightContainer::Full(data)) = light_engine.sky_light.get(section_index) else {
            continue;
        };
        write.write_var_int(&light_data_size)?;
        // LightContainer uses the same low-nibble-first layout as vanilla's
        // DataLayer; do not rotate bytes here.
        write.write_slice(data.as_ref())?;
    }

    write.write_var_int(&VarInt(block_sections.len() as i32))?;
    for section_index in block_sections {
        let Some(LightContainer::Full(data)) = light_engine.block_light.get(section_index) else {
            continue;
        };
        write.write_var_int(&light_data_size)?;
        write.write_slice(data.as_ref())?;
    }

    Ok(())
}

/// Sent by the server to update light levels (block light and sky light) for a chunk.
///
/// This packet updates lighting data for a specific chunk without sending the full chunk data.
/// It's used when block placement or removal changes the lighting in a chunk.
#[java_packet(PLAY_LIGHT_UPDATE)]
pub struct CLightUpdate<'a>(pub &'a ChunkData, pub &'a [usize]);

impl ClientPacket for CLightUpdate<'_> {
    fn write_packet_data(
        &self,
        write: impl Write,
        _version: &JavaMinecraftVersion,
    ) -> Result<(), WritingError> {
        let mut write = write;

        // Chunk X
        write.write_var_int(&VarInt(self.0.x))?;
        // Chunk Z
        write.write_var_int(&VarInt(self.0.z))?;

        let light_engine = self
            .0
            .light_engine
            .lock()
            .map_err(|_| WritingError::Message("light_engine lock poisoned".into()))?;
        write_light_data_filtered(&mut write, &light_engine, Some(self.1))
    }
}

#[cfg(test)]
mod tests {
    use super::{build_light_mask, write_light_data};
    use pumpkin_world::chunk::ChunkLight;
    use pumpkin_world::chunk::format::LightContainer;

    #[test]
    fn light_masks_reserve_below_and_above_world_sections() {
        let sections = vec![
            LightContainer::new_empty(0),
            LightContainer::new_filled(15),
            LightContainer::new_empty(0),
        ];

        let (mask, empty, full_sections) = build_light_mask(&sections, sections.len(), None);

        assert_eq!(full_sections, vec![1]);
        assert_eq!(mask.0.as_ref(), &[1_i64 << 2]);
        // bit 0 = below, bit 1 and 3 = empty real sections, bit 4 = above
        assert_eq!(
            empty.0[0],
            (1_i64 << 0) | (1_i64 << 1) | (1_i64 << 3) | (1_i64 << 4)
        );
    }

    #[test]
    fn light_masks_support_more_than_one_bitset_word() {
        let sections = vec![LightContainer::new_empty(0); 64];
        let (mask, empty, _) = build_light_mask(&sections, sections.len(), None);

        assert_eq!(mask.0.len(), 2);
        assert_eq!(empty.0.len(), 2);
        assert_eq!(empty.0[0], -1);
        // bit 63 belongs to the real section at index 62; bit 64 is in word 1.
        assert_eq!(empty.0[1] & 0b11, 0b11);
    }

    #[test]
    fn filtered_masks_only_include_changed_real_sections() {
        let sections = vec![
            LightContainer::new_filled(15),
            LightContainer::new_empty(0),
            LightContainer::new_filled(7),
        ];
        let (mask, empty, full_sections) = build_light_mask(&sections, sections.len(), Some(&[2]));

        assert_eq!(full_sections, vec![2]);
        assert_eq!(mask.0[0], 1_i64 << 3);
        // Synthetic below/above layers remain represented; unchanged real
        // sections are absent from both masks.
        assert_eq!(empty.0[0], (1_i64 << 0) | (1_i64 << 4));
    }

    #[test]
    fn light_payload_keeps_vanilla_low_nibble_first_order() {
        let mut sky = vec![0_u8; LightContainer::ARRAY_SIZE].into_boxed_slice();
        sky[0] = 0x12;
        let light = ChunkLight {
            sky_light: vec![LightContainer::new(sky)].into_boxed_slice(),
            block_light: vec![LightContainer::new_empty(0)].into_boxed_slice(),
        };
        let mut bytes = Vec::new();
        write_light_data(&mut bytes, &light).unwrap();

        // Four one-word BitSets: 4 * (VarInt length + i64) = 36 bytes.
        // Then sky count (1), array length (2048 = 0x80, 0x10), and the raw
        // DataLayer bytes.  A previous implementation rotated 0x12 to 0x21.
        assert_eq!(bytes[36], 1);
        assert_eq!(&bytes[37..39], &[0x80, 0x10]);
        assert_eq!(bytes[39], 0x12);
    }
}
