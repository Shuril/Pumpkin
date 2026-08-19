use std::sync::Arc;

use pumpkin_data::block_rotation::Rotation;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::{
    math::{block_box::BlockBox, position::BlockPos},
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
                StructureTemplate, for_each_data_marker_with_pivot, get_template,
                place_template_with_pivot,
            },
        },
    },
};

const BEACHED_TEMPLATES: &[&str] = &[
    "shipwreck/with_mast",
    "shipwreck/sideways_full",
    "shipwreck/sideways_fronthalf",
    "shipwreck/sideways_backhalf",
    "shipwreck/rightsideup_full",
    "shipwreck/rightsideup_fronthalf",
    "shipwreck/rightsideup_backhalf",
    "shipwreck/with_mast_degraded",
    "shipwreck/rightsideup_full_degraded",
    "shipwreck/rightsideup_fronthalf_degraded",
    "shipwreck/rightsideup_backhalf_degraded",
];

const OCEAN_TEMPLATES: &[&str] = &[
    "shipwreck/rightsideup_backhalf",
    "shipwreck/rightsideup_backhalf_degraded",
    "shipwreck/rightsideup_fronthalf",
    "shipwreck/rightsideup_fronthalf_degraded",
    "shipwreck/rightsideup_full",
    "shipwreck/rightsideup_full_degraded",
    "shipwreck/sideways_backhalf",
    "shipwreck/sideways_backhalf_degraded",
    "shipwreck/sideways_fronthalf",
    "shipwreck/sideways_fronthalf_degraded",
    "shipwreck/sideways_full",
    "shipwreck/sideways_full_degraded",
    "shipwreck/upsidedown_backhalf",
    "shipwreck/upsidedown_backhalf_degraded",
    "shipwreck/upsidedown_fronthalf",
    "shipwreck/upsidedown_fronthalf_degraded",
    "shipwreck/upsidedown_full",
    "shipwreck/upsidedown_full_degraded",
    "shipwreck/with_mast",
    "shipwreck/with_mast_degraded",
];

pub struct ShipwreckGenerator {
    pub is_beached: bool,
}

impl StructureGenerator for ShipwreckGenerator {
    fn get_structure_position(
        &self,
        mut context: StructureGeneratorContext<'_>,
    ) -> Option<StructurePosition> {
        let chunk_center_x = get_center_x(context.chunk_x);
        let chunk_center_z = get_center_z(context.chunk_z);

        // Deterministically select rotation and template
        let rotation_idx = context.random.next_bounded_i32(4) as u8;
        let rotation = Rotation::from_index(rotation_idx);

        let templates = if self.is_beached {
            BEACHED_TEMPLATES
        } else {
            OCEAN_TEMPLATES
        };
        let template_idx = context.random.next_bounded_i32(templates.len() as i32) as usize;
        let template_name = templates[template_idx];
        let template = get_template(template_name)?;

        let origin_x = start_block_x(context.chunk_x);
        let origin_z = start_block_z(context.chunk_z);
        let bounding_box = shipwreck_bounds(&template, origin_x, origin_z, rotation);

        let mut collector = StructurePiecesCollector::default();
        collector.add_piece(Box::new(ShipwreckPiece {
            piece: StructurePiece::new(StructurePieceType::Shipwreck, bounding_box, 0),
            template,
            rotation,
            is_beached: self.is_beached,
            origin_x,
            origin_z,
            placed_y: None,
        }));

        Some(StructurePosition {
            start_pos: BlockPos::new(chunk_center_x, 64, chunk_center_z),
            collector: Arc::new(collector.into()),
        })
    }
}

fn shipwreck_bounds(
    template: &StructureTemplate,
    origin_x: i32,
    origin_z: i32,
    rotation: Rotation,
) -> BlockBox {
    const PIVOT_X: i32 = 4;
    const PIVOT_Z: i32 = 15;
    let mut min_x = i32::MAX;
    let mut min_z = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_z = i32::MIN;
    for point in [
        pumpkin_util::math::vector3::Vector3::new(0, 0, 0),
        pumpkin_util::math::vector3::Vector3::new(template.size.x - 1, 0, 0),
        pumpkin_util::math::vector3::Vector3::new(0, 0, template.size.z - 1),
        pumpkin_util::math::vector3::Vector3::new(template.size.x - 1, 0, template.size.z - 1),
    ] {
        let dx = point.x - PIVOT_X;
        let dz = point.z - PIVOT_Z;
        let point = match rotation {
            Rotation::None => point,
            Rotation::Clockwise90 => {
                pumpkin_util::math::vector3::Vector3::new(PIVOT_X - dz, 0, PIVOT_Z + dx)
            }
            Rotation::Rotate180 => {
                pumpkin_util::math::vector3::Vector3::new(PIVOT_X - dx, 0, PIVOT_Z - dz)
            }
            Rotation::CounterClockwise90 => {
                pumpkin_util::math::vector3::Vector3::new(PIVOT_X + dz, 0, PIVOT_Z - dx)
            }
        };
        min_x = min_x.min(origin_x + point.x);
        min_z = min_z.min(origin_z + point.z);
        max_x = max_x.max(origin_x + point.x);
        max_z = max_z.max(origin_z + point.z);
    }
    BlockBox::new(min_x, -64, min_z, max_x, 320, max_z)
}

pub struct ShipwreckPiece {
    piece: StructurePiece,
    template: Arc<StructureTemplate>,
    rotation: Rotation,
    is_beached: bool,
    origin_x: i32,
    origin_z: i32,
    placed_y: Option<i32>,
}

impl StructurePieceBase for ShipwreckPiece {
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
        _random: &mut RandomGenerator,
        _seed: i64,
        chunk_box: &BlockBox,
    ) {
        let height_map_type = if self.is_beached {
            pumpkin_util::HeightMap::WorldSurfaceWg
        } else {
            pumpkin_util::HeightMap::OceanFloorWg
        };

        let target_y = *self.placed_y.get_or_insert_with(|| {
            let sample_y = chunk.get_top_y(&height_map_type, self.origin_x, self.origin_z);
            if self.is_beached {
                sample_y - (self.template.size.y / 2)
            } else {
                sample_y
            }
        });
        let final_origin =
            pumpkin_util::math::vector3::Vector3::new(self.origin_x, target_y, self.origin_z);

        place_template_with_pivot(
            chunk,
            &self.template,
            final_origin,
            (0, 0),
            self.rotation,
            pumpkin_util::math::vector3::Vector3::new(4, 0, 15),
            true,
            !self.is_beached,
            &[],
            Some(chunk_box),
        );

        // Vanilla stores the shipwreck loot mapping in structure-block data
        // markers one block above the actual chest.
        for_each_data_marker_with_pivot(
            &self.template,
            final_origin,
            self.rotation,
            pumpkin_util::math::vector3::Vector3::new(4, 0, 15),
            chunk_box,
            |marker, marker_pos| {
                let loot_table = match marker {
                    "map_chest" => "minecraft:chests/shipwreck_map",
                    "treasure_chest" => "minecraft:chests/shipwreck_treasure",
                    "supply_chest" => "minecraft:chests/shipwreck_supply",
                    _ => return,
                };
                let chest_pos = pumpkin_util::math::vector3::Vector3::new(
                    marker_pos.x,
                    marker_pos.y - 1,
                    marker_pos.z,
                );
                if !chunk_box.contains_pos(&chest_pos) {
                    return;
                }

                let mut nbt = NbtCompound::new();
                nbt.put_string("id", "minecraft:chest".to_string());
                nbt.put_int("x", chest_pos.x);
                nbt.put_int("y", chest_pos.y);
                nbt.put_int("z", chest_pos.z);
                nbt.put_string("LootTable", loot_table.to_string());
                nbt.put_long("LootTableSeed", _random.next_i64());
                chunk.add_block_entity(nbt);
            },
        );
    }
}
