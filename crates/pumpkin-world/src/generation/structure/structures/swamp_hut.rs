use std::sync::Arc;

use pumpkin_data::{
    Block, BlockState,
    block_properties::{BlockProperties, HorizontalFacing, OakStairsLikeProperties, StairsShape},
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::{
    BlockDirection,
    math::{block_box::BlockBox, position::BlockPos, vector3::Vector3},
    random::RandomGenerator,
};
use serde::Deserialize;

use crate::{
    ProtoChunk,
    generation::{
        positions::chunk_pos::{get_center_x, get_center_z},
        structure::{
            piece::StructurePieceType,
            shiftable_piece::ShiftableStructurePiece,
            structures::{
                StructureGenerator, StructureGeneratorContext, StructurePieceBase,
                StructurePiecesCollector, StructurePosition,
            },
        },
    },
};

#[derive(Deserialize)]
pub struct SwampHutGenerator;

impl StructureGenerator for SwampHutGenerator {
    fn get_structure_position(
        &self,
        mut context: StructureGeneratorContext<'_>,
    ) -> Option<StructurePosition> {
        let x = get_center_x(context.chunk_x);
        let z = get_center_z(context.chunk_z);

        let mut collector = StructurePiecesCollector::default();
        collector.add_piece(Box::new(SwampHutPiece {
            shiftable_structure_piece: ShiftableStructurePiece::new(
                StructurePieceType::SwampHut,
                x,
                64,
                z,
                7,
                7,
                9,
                BlockDirection::get_random_horizontal_direction(&mut context.random).get_axis(),
            ),
            spawned_witch: false,
            spawned_cat: false,
        }));

        Some(StructurePosition {
            start_pos: BlockPos::new(x, 64, z),
            collector: Arc::new(collector.into()),
        })
    }
}

pub struct SwampHutPiece {
    shiftable_structure_piece: ShiftableStructurePiece,
    spawned_witch: bool,
    spawned_cat: bool,
}

impl StructurePieceBase for SwampHutPiece {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    #[allow(clippy::too_many_lines)]
    fn place(
        &mut self,
        chunk: &mut ProtoChunk,
        _block_registry: &dyn crate::world::WorldPortalExt,
        _random: &mut RandomGenerator,
        _seed: i64,
        chunk_box: &BlockBox,
    ) {
        if !self
            .shiftable_structure_piece
            .adjust_to_average_height(chunk)
        {
            return;
        }

        let box_limit = *chunk_box;
        let p = &self.shiftable_structure_piece.piece;

        let spruce_planks = Block::SPRUCE_PLANKS.default_state;
        let oak_log = Block::OAK_LOG.default_state;
        let oak_fence = Block::OAK_FENCE.default_state;
        let air = Block::AIR.default_state;

        p.fill_with_outline(
            chunk,
            &box_limit,
            false,
            1,
            1,
            1,
            5,
            1,
            7,
            spruce_planks,
            spruce_planks,
        );
        p.fill_with_outline(
            chunk,
            &box_limit,
            false,
            1,
            4,
            2,
            5,
            4,
            7,
            spruce_planks,
            spruce_planks,
        );
        p.fill_with_outline(
            chunk,
            &box_limit,
            false,
            2,
            1,
            0,
            4,
            1,
            0,
            spruce_planks,
            spruce_planks,
        );

        p.fill_with_outline(
            chunk,
            &box_limit,
            false,
            2,
            2,
            2,
            3,
            3,
            2,
            spruce_planks,
            spruce_planks,
        );
        p.fill_with_outline(
            chunk,
            &box_limit,
            false,
            1,
            2,
            3,
            1,
            3,
            6,
            spruce_planks,
            spruce_planks,
        );
        p.fill_with_outline(
            chunk,
            &box_limit,
            false,
            5,
            2,
            3,
            5,
            3,
            6,
            spruce_planks,
            spruce_planks,
        );
        p.fill_with_outline(
            chunk,
            &box_limit,
            false,
            2,
            2,
            7,
            4,
            3,
            7,
            spruce_planks,
            spruce_planks,
        );

        p.fill_with_outline(chunk, &box_limit, false, 1, 0, 2, 1, 3, 2, oak_log, oak_log);
        p.fill_with_outline(chunk, &box_limit, false, 5, 0, 2, 5, 3, 2, oak_log, oak_log);
        p.fill_with_outline(chunk, &box_limit, false, 1, 0, 7, 1, 3, 7, oak_log, oak_log);
        p.fill_with_outline(chunk, &box_limit, false, 5, 0, 7, 5, 3, 7, oak_log, oak_log);

        p.add_block(chunk, oak_fence, 2, 3, 2, &box_limit);
        p.add_block(chunk, oak_fence, 3, 3, 7, &box_limit);
        p.add_block(chunk, air, 1, 3, 4, &box_limit);
        p.add_block(chunk, air, 5, 3, 4, &box_limit);
        p.add_block(chunk, air, 5, 3, 5, &box_limit);
        p.add_block(
            chunk,
            Block::POTTED_RED_MUSHROOM.default_state,
            1,
            3,
            5,
            &box_limit,
        );
        p.add_block(
            chunk,
            Block::CRAFTING_TABLE.default_state,
            3,
            2,
            6,
            &box_limit,
        );
        p.add_block(chunk, Block::CAULDRON.default_state, 4, 2, 6, &box_limit);
        p.add_block(chunk, oak_fence, 1, 2, 1, &box_limit);
        p.add_block(chunk, oak_fence, 5, 2, 1, &box_limit);

        let stairs_n = Self::spruce_stairs(HorizontalFacing::North, StairsShape::Straight);
        let stairs_e = Self::spruce_stairs(HorizontalFacing::East, StairsShape::Straight);
        let stairs_w = Self::spruce_stairs(HorizontalFacing::West, StairsShape::Straight);
        let stairs_s = Self::spruce_stairs(HorizontalFacing::South, StairsShape::Straight);
        p.fill_with_outline(
            chunk, &box_limit, false, 0, 4, 1, 6, 4, 1, stairs_n, stairs_n,
        );
        p.fill_with_outline(
            chunk, &box_limit, false, 0, 4, 2, 0, 4, 7, stairs_e, stairs_e,
        );
        p.fill_with_outline(
            chunk, &box_limit, false, 6, 4, 2, 6, 4, 7, stairs_w, stairs_w,
        );
        p.fill_with_outline(
            chunk, &box_limit, false, 0, 4, 8, 6, 4, 8, stairs_s, stairs_s,
        );
        p.add_block(
            chunk,
            Self::spruce_stairs(HorizontalFacing::North, StairsShape::OuterRight),
            0,
            4,
            1,
            &box_limit,
        );
        p.add_block(
            chunk,
            Self::spruce_stairs(HorizontalFacing::North, StairsShape::OuterLeft),
            6,
            4,
            1,
            &box_limit,
        );
        p.add_block(
            chunk,
            Self::spruce_stairs(HorizontalFacing::South, StairsShape::OuterLeft),
            0,
            4,
            8,
            &box_limit,
        );
        p.add_block(
            chunk,
            Self::spruce_stairs(HorizontalFacing::South, StairsShape::OuterRight),
            6,
            4,
            8,
            &box_limit,
        );

        for i in [2, 7] {
            for j in [1, 5] {
                p.fill_downwards(chunk, oak_log, j, -1, i, &box_limit);
            }
        }

        // Vanilla creates one persistent witch and one persistent black cat at
        // this marker.  The flags matter because a structure can be generated
        // through several chunk passes.
        let entity_pos = p.offset_pos(2, 2, 5);
        if box_limit.contains_pos(&entity_pos) {
            if !self.spawned_witch {
                chunk.add_structure_entity(Self::structure_entity("minecraft:witch", entity_pos));
                self.spawned_witch = true;
            }
            if !self.spawned_cat {
                chunk.add_structure_entity(Self::structure_entity("minecraft:cat", entity_pos));
                self.spawned_cat = true;
            }
        }
    }
    fn get_structure_piece(&self) -> &super::StructurePiece {
        &self.shiftable_structure_piece.piece
    }

    fn get_structure_piece_mut(&mut self) -> &mut super::StructurePiece {
        &mut self.shiftable_structure_piece.piece
    }
}

impl SwampHutPiece {
    fn spruce_stairs(facing: HorizontalFacing, shape: StairsShape) -> &'static BlockState {
        let mut props = OakStairsLikeProperties::default(&Block::SPRUCE_STAIRS);
        props.facing = facing;
        props.shape = shape;
        BlockState::from_id(props.to_state_id(&Block::SPRUCE_STAIRS))
    }

    fn structure_entity(id: &str, position: Vector3<i32>) -> NbtCompound {
        let mut nbt = NbtCompound::new();
        nbt.put_string("id", id.to_string());
        nbt.put(
            "Pos",
            NbtTag::List(vec![
                (f64::from(position.x) + 0.5).into(),
                f64::from(position.y).into(),
                (f64::from(position.z) + 0.5).into(),
            ]),
        );
        nbt.put(
            "Motion",
            NbtTag::List(vec![0.0.into(), 0.0.into(), 0.0.into()]),
        );
        nbt.put("Rotation", NbtTag::List(vec![0.0f32.into(), 0.0f32.into()]));
        nbt.put_bool("PersistenceRequired", true);
        nbt
    }
}
