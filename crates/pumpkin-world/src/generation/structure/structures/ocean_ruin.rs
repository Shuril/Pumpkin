//! Vanilla ocean-ruin placement.
//!
//! Ocean ruins are not a single template: large ruins receive a surrounding
//! cluster and cold ruins layer three matching templates with different decay
//! integrities.  Keeping the pieces separate is important because template
//! processors use the world position as their random seed.

use std::sync::Arc;

use pumpkin_data::Block;
use pumpkin_data::block_rotation::Rotation;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::{
    math::{block_box::BlockBox, position::BlockPos, vector3::Vector3},
    random::{RandomGenerator, RandomImpl},
};

use crate::{
    ProtoChunk,
    generation::{
        positions::chunk_pos::{get_center_x, get_center_z, start_block_x, start_block_z},
        structure::{
            piece::StructurePieceType,
            structures::{
                StructureGenerator, StructureGeneratorContext, StructurePiece, StructurePieceBase,
                StructurePiecesCollector, StructurePosition, WorldPortalExt,
            },
            template::{
                StructureProcessor, StructureTemplate, for_each_data_marker, get_template,
                place_template,
            },
        },
    },
};

const WARM_RUINS: &[&str] = &[
    "underwater_ruin/warm_1",
    "underwater_ruin/warm_2",
    "underwater_ruin/warm_3",
    "underwater_ruin/warm_4",
    "underwater_ruin/warm_5",
    "underwater_ruin/warm_6",
    "underwater_ruin/warm_7",
    "underwater_ruin/warm_8",
];
const BIG_WARM_RUINS: &[&str] = &[
    "underwater_ruin/big_warm_4",
    "underwater_ruin/big_warm_5",
    "underwater_ruin/big_warm_6",
    "underwater_ruin/big_warm_7",
];
const BRICK_RUINS: &[&str] = &[
    "underwater_ruin/brick_1",
    "underwater_ruin/brick_2",
    "underwater_ruin/brick_3",
    "underwater_ruin/brick_4",
    "underwater_ruin/brick_5",
    "underwater_ruin/brick_6",
    "underwater_ruin/brick_7",
    "underwater_ruin/brick_8",
];
const CRACKED_RUINS: &[&str] = &[
    "underwater_ruin/cracked_1",
    "underwater_ruin/cracked_2",
    "underwater_ruin/cracked_3",
    "underwater_ruin/cracked_4",
    "underwater_ruin/cracked_5",
    "underwater_ruin/cracked_6",
    "underwater_ruin/cracked_7",
    "underwater_ruin/cracked_8",
];
const MOSSY_RUINS: &[&str] = &[
    "underwater_ruin/mossy_1",
    "underwater_ruin/mossy_2",
    "underwater_ruin/mossy_3",
    "underwater_ruin/mossy_4",
    "underwater_ruin/mossy_5",
    "underwater_ruin/mossy_6",
    "underwater_ruin/mossy_7",
    "underwater_ruin/mossy_8",
];
const BIG_BRICK_RUINS: &[&str] = &[
    "underwater_ruin/big_brick_1",
    "underwater_ruin/big_brick_2",
    "underwater_ruin/big_brick_3",
    "underwater_ruin/big_brick_8",
];
const BIG_CRACKED_RUINS: &[&str] = &[
    "underwater_ruin/big_cracked_1",
    "underwater_ruin/big_cracked_2",
    "underwater_ruin/big_cracked_3",
    "underwater_ruin/big_cracked_8",
];
const BIG_MOSSY_RUINS: &[&str] = &[
    "underwater_ruin/big_mossy_1",
    "underwater_ruin/big_mossy_2",
    "underwater_ruin/big_mossy_3",
    "underwater_ruin/big_mossy_8",
];

pub struct OceanRuinGenerator {
    pub is_warm: bool,
}

impl StructureGenerator for OceanRuinGenerator {
    fn get_structure_position(
        &self,
        mut context: StructureGeneratorContext<'_>,
    ) -> Option<StructurePosition> {
        let origin = Vector3::new(
            start_block_x(context.chunk_x),
            90,
            start_block_z(context.chunk_z),
        );
        let rotation = Rotation::from_index(context.random.next_bounded_i32(4) as u8);
        let is_large = context.random.next_f32() <= 0.3;
        let mut collector = StructurePiecesCollector::default();

        add_ruin(
            &mut collector,
            &mut context.random,
            origin,
            rotation,
            self.is_warm,
            is_large,
            if is_large { 0.9 } else { 0.8 },
        );
        if is_large && context.random.next_f32() <= 0.9 {
            add_cluster(&mut collector, &mut context.random, origin, self.is_warm);
        }

        Some(StructurePosition {
            start_pos: BlockPos::new(
                get_center_x(context.chunk_x),
                64,
                get_center_z(context.chunk_z),
            ),
            collector: Arc::new(collector.into()),
        })
    }
}

fn add_ruin(
    collector: &mut StructurePiecesCollector,
    random: &mut RandomGenerator,
    origin: Vector3<i32>,
    rotation: Rotation,
    is_warm: bool,
    is_large: bool,
    integrity: f32,
) {
    if is_warm {
        let names = if is_large { BIG_WARM_RUINS } else { WARM_RUINS };
        add_piece(
            collector,
            origin,
            rotation,
            names[random.next_bounded_i32(names.len() as i32) as usize],
            integrity,
            true,
            is_large,
        );
        return;
    }

    let bricks = if is_large {
        BIG_BRICK_RUINS
    } else {
        BRICK_RUINS
    };
    let cracked = if is_large {
        BIG_CRACKED_RUINS
    } else {
        CRACKED_RUINS
    };
    let mossy = if is_large {
        BIG_MOSSY_RUINS
    } else {
        MOSSY_RUINS
    };
    let index = random.next_bounded_i32(bricks.len() as i32) as usize;
    add_piece(
        collector,
        origin,
        rotation,
        bricks[index],
        integrity,
        false,
        is_large,
    );
    add_piece(
        collector,
        origin,
        rotation,
        cracked[index],
        0.7,
        false,
        is_large,
    );
    add_piece(
        collector,
        origin,
        rotation,
        mossy[index],
        0.5,
        false,
        is_large,
    );
}

fn add_piece(
    collector: &mut StructurePiecesCollector,
    origin: Vector3<i32>,
    rotation: Rotation,
    name: &str,
    integrity: f32,
    is_warm: bool,
    is_large: bool,
) {
    let Some(template) = get_template(name) else {
        return;
    };
    let bounds = template_bounds(&template, origin.x, origin.z, rotation);
    collector.add_piece(Box::new(OceanRuinPiece {
        piece: StructurePiece::new(StructurePieceType::OceanRuin, bounds, 0),
        template,
        origin_x: origin.x,
        origin_z: origin.z,
        rotation,
        integrity,
        is_warm,
        is_large,
        placed_y: None,
    }));
}

fn add_cluster(
    collector: &mut StructurePiecesCollector,
    random: &mut RandomGenerator,
    origin: Vector3<i32>,
    is_warm: bool,
) {
    // This is OceanRuinPieces#allPositions in its original ordering. The parent
    // bounding box check is redundant for these deliberately spaced offsets.
    let offsets = [
        (-16 + between(random, 1, 8), 16 + between(random, 1, 7)),
        (-16 + between(random, 1, 8), between(random, 1, 7)),
        (-16 + between(random, 1, 8), -16 + between(random, 4, 8)),
        (between(random, 1, 7), 16 + between(random, 1, 7)),
        (between(random, 1, 7), -16 + between(random, 4, 6)),
        (16 + between(random, 1, 7), 16 + between(random, 3, 8)),
        (16 + between(random, 1, 7), between(random, 1, 7)),
        (16 + between(random, 1, 7), -16 + between(random, 4, 8)),
    ];
    let mut positions = offsets
        .into_iter()
        .map(|(x, z)| Vector3::new(origin.x + x, 90, origin.z + z))
        .collect::<Vec<_>>();
    let ruin_count = between(random, 4, 8);
    for _ in 0..ruin_count {
        if positions.is_empty() {
            break;
        }
        let index = random.next_bounded_i32(positions.len() as i32) as usize;
        let position = positions.swap_remove(index);
        let rotation = Rotation::from_index(random.next_bounded_i32(4) as u8);
        add_ruin(collector, random, position, rotation, is_warm, false, 0.8);
    }
}

fn between(random: &mut RandomGenerator, min: i32, max: i32) -> i32 {
    min + random.next_bounded_i32(max - min + 1)
}

fn template_bounds(template: &StructureTemplate, x: i32, z: i32, rotation: Rotation) -> BlockBox {
    let mut min_x = i32::MAX;
    let mut min_z = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_z = i32::MIN;
    for corner in [
        Vector3::new(0, 0, 0),
        Vector3::new(template.size.x - 1, 0, 0),
        Vector3::new(0, 0, template.size.z - 1),
        Vector3::new(template.size.x - 1, 0, template.size.z - 1),
    ] {
        let rotated = rotation.transform_pos(corner, template.size);
        min_x = min_x.min(x + rotated.x);
        min_z = min_z.min(z + rotated.z);
        max_x = max_x.max(x + rotated.x);
        max_z = max_z.max(z + rotated.z);
    }
    // The exact Y is discovered from the ocean floor when the piece is first
    // generated. Keep the start intersecting every chunk that could contain it.
    BlockBox::new(min_x, -64, min_z, max_x, 320, max_z)
}

pub struct OceanRuinPiece {
    piece: StructurePiece,
    template: Arc<StructureTemplate>,
    origin_x: i32,
    origin_z: i32,
    rotation: Rotation,
    integrity: f32,
    is_warm: bool,
    is_large: bool,
    placed_y: Option<i32>,
}

impl StructurePieceBase for OceanRuinPiece {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn get_structure_piece(&self) -> &StructurePiece {
        &self.piece
    }
    fn get_structure_piece_mut(&mut self) -> &mut StructurePiece {
        &mut self.piece
    }

    fn place(
        &mut self,
        chunk: &mut ProtoChunk,
        _block_registry: &dyn WorldPortalExt,
        random: &mut RandomGenerator,
        _seed: i64,
        chunk_box: &BlockBox,
    ) {
        let y = *self.placed_y.get_or_insert_with(|| {
            chunk.get_top_y(
                &pumpkin_util::HeightMap::OceanFloorWg,
                self.origin_x,
                self.origin_z,
            )
        });
        let origin = Vector3::new(self.origin_x, y, self.origin_z);
        let processors = [StructureProcessor::BlockRot {
            integrity: self.integrity,
            blocks: None,
        }];
        place_template(
            chunk,
            &self.template,
            origin,
            (0, 0),
            self.rotation,
            true,
            true,
            &processors,
            Some(chunk_box),
        );

        for_each_data_marker(
            &self.template,
            origin,
            self.rotation,
            chunk_box,
            |marker, pos| match marker {
                "chest" => {
                    chunk.set_block_state(pos.x, pos.y, pos.z, Block::CHEST.default_state);
                    let mut nbt = NbtCompound::new();
                    nbt.put_string("id", "minecraft:chest".to_string());
                    nbt.put_int("x", pos.x);
                    nbt.put_int("y", pos.y);
                    nbt.put_int("z", pos.z);
                    nbt.put_string(
                        "LootTable",
                        if self.is_large {
                            "minecraft:chests/underwater_ruin_big"
                        } else {
                            "minecraft:chests/underwater_ruin_small"
                        }
                        .to_string(),
                    );
                    nbt.put_long("LootTableSeed", random.next_i64());
                    chunk.add_block_entity(nbt);
                }
                "drowned" => {
                    let mut nbt = NbtCompound::new();
                    nbt.put_string("id", "minecraft:drowned".to_string());
                    nbt.put_list(
                        "Pos",
                        vec![
                            NbtTag::Double(f64::from(pos.x) + 0.5),
                            NbtTag::Double(f64::from(pos.y)),
                            NbtTag::Double(f64::from(pos.z) + 0.5),
                        ],
                    );
                    nbt.put_list(
                        "Motion",
                        vec![
                            NbtTag::Double(0.0),
                            NbtTag::Double(0.0),
                            NbtTag::Double(0.0),
                        ],
                    );
                    nbt.put_list("Rotation", vec![NbtTag::Float(0.0), NbtTag::Float(0.0)]);
                    nbt.put_bool("PersistenceRequired", true);
                    chunk.add_structure_entity(nbt);
                    chunk.set_block_state(
                        pos.x,
                        pos.y,
                        pos.z,
                        if pos.y > 63 {
                            Block::AIR.default_state
                        } else {
                            Block::WATER.default_state
                        },
                    );
                }
                _ => {}
            },
        );
        let _ = self.is_warm; // retained for persisted-piece parity and future archaeology processor.
    }
}
