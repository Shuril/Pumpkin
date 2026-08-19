use pumpkin_data::{
    Block, BlockState,
    block_properties::{BeeNestLikeProperties, BlockProperties, HorizontalFacing},
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use crate::generation::proto_chunk::GenerationCache;

pub struct BeehiveTreeDecorator {
    pub probability: f32,
}

const WORLDGEN_FACING: HorizontalFacing = HorizontalFacing::South;

fn shuffled_positions(positions: &[BlockPos], random: &mut RandomGenerator) -> Vec<BlockPos> {
    let mut shuffled = positions.to_vec();
    for index in (1..shuffled.len()).rev() {
        let swap = random.next_bounded_i32((index + 1) as i32) as usize;
        shuffled.swap(index, swap);
    }
    shuffled
}

fn bee_occupant(ticks_in_hive: i32) -> NbtTag {
    let mut entity_data = NbtCompound::new();
    entity_data.put_string("id", "minecraft:bee".to_string());

    let mut occupant = NbtCompound::new();
    occupant.put_compound("entity_data", entity_data);
    occupant.put_int("ticks_in_hive", ticks_in_hive);
    occupant.put_int("min_ticks_in_hive", 600);
    NbtTag::Compound(occupant)
}

impl BeehiveTreeDecorator {
    pub fn generate<T: GenerationCache>(
        &self,
        chunk: &mut T,
        random: &mut RandomGenerator,
        leaves_positions: &[BlockPos],
        log_positions: &[BlockPos],
    ) {
        if log_positions.is_empty() || random.next_f32() >= self.probability {
            return;
        }

        let first_log_y = log_positions[0].0.y;
        let hive_y = if let Some(first_leaf) = leaves_positions.first() {
            (first_leaf.0.y - 1).max(first_log_y + 1)
        } else {
            (first_log_y + 1 + random.next_bounded_i32(3)).min(log_positions.last().unwrap().0.y)
        };

        // Direction.Plane.HORIZONTAL minus the opposite of SOUTH (NORTH), in
        // vanilla order NORTH, EAST, SOUTH, WEST.
        let spawn_directions = [
            HorizontalFacing::East,
            HorizontalFacing::South,
            HorizontalFacing::West,
        ];
        let mut candidates = log_positions
            .iter()
            .filter(|pos| pos.0.y == hive_y)
            .flat_map(|pos| {
                spawn_directions
                    .iter()
                    .map(move |direction| pos.offset(direction.to_offset()))
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return;
        }
        candidates = shuffled_positions(&candidates, random);

        let Some(hive_pos) = candidates.into_iter().find(|pos| {
            chunk.is_air(&pos.0) && chunk.is_air(&pos.offset(WORLDGEN_FACING.to_offset()).0)
        }) else {
            return;
        };

        let properties = BeeNestLikeProperties {
            facing: WORLDGEN_FACING,
            honey_level: 0,
        };
        chunk.set_block_state(
            &hive_pos.0,
            BlockState::from_id(properties.to_state_id(&Block::BEE_NEST)),
        );

        let mut nbt = NbtCompound::new();
        nbt.put_string("id", "minecraft:beehive".to_string());
        nbt.put_int("x", hive_pos.0.x);
        nbt.put_int("y", hive_pos.0.y);
        nbt.put_int("z", hive_pos.0.z);
        let bee_count = 2 + random.next_bounded_i32(2);
        nbt.put_list(
            "bees",
            (0..bee_count)
                .map(|_| bee_occupant(random.next_bounded_i32(599)))
                .collect(),
        );
        chunk.add_block_entity(&hive_pos.0, nbt);
    }
}

#[cfg(test)]
mod tests {
    use super::bee_occupant;
    use pumpkin_nbt::tag::NbtTag;

    #[test]
    fn generated_occupant_uses_modern_beehive_schema() {
        let NbtTag::Compound(occupant) = bee_occupant(17) else {
            panic!("occupant must be a compound");
        };
        assert_eq!(occupant.get_int("ticks_in_hive"), Some(17));
        assert_eq!(occupant.get_int("min_ticks_in_hive"), Some(600));
        let Some(NbtTag::Compound(entity_data)) = occupant.child_tags.get("entity_data") else {
            panic!("entity_data must be a compound");
        };
        assert_eq!(entity_data.get_string("id"), Some("minecraft:bee"));
    }
}
