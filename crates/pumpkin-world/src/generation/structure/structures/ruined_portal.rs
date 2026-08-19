use std::sync::Arc;

use pumpkin_data::{Mirror, block_rotation::Rotation, structures::StructureKeys};
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
            template::{StructureTemplate, get_template, place_template_with_settings},
        },
    },
};

const PORTALS: &[&str] = &[
    "ruined_portal/portal_1",
    "ruined_portal/portal_2",
    "ruined_portal/portal_3",
    "ruined_portal/portal_4",
    "ruined_portal/portal_5",
    "ruined_portal/portal_6",
    "ruined_portal/portal_7",
    "ruined_portal/portal_8",
    "ruined_portal/portal_9",
    "ruined_portal/portal_10",
    "ruined_portal/giant_portal_1",
    "ruined_portal/giant_portal_2",
    "ruined_portal/giant_portal_3",
];

pub struct RuinedPortalGenerator {
    pub variant: StructureKeys,
}

#[derive(Clone, Copy)]
enum PortalPlacement {
    Land,
    Buried,
    OceanFloor,
    Mountain,
    Underground,
    Nether,
}

#[derive(Clone, Copy)]
struct PortalSetup {
    placement: PortalPlacement,
    air_pocket_probability: f32,
    mossiness: f32,
    overgrown: bool,
    vines: bool,
    replace_with_blackstone: bool,
}

fn setup_for(variant: StructureKeys, random: &mut RandomGenerator) -> PortalSetup {
    use PortalPlacement::*;
    match variant {
        StructureKeys::RuinedPortal => {
            // The standard pool is two equal-weight entries. Keep this draw
            // before all template draws to preserve vanilla's RNG sequence.
            if random.next_f32() < 0.5 {
                PortalSetup {
                    placement: Underground,
                    air_pocket_probability: 1.0,
                    mossiness: 0.2,
                    overgrown: false,
                    vines: false,
                    replace_with_blackstone: false,
                }
            } else {
                PortalSetup {
                    placement: Land,
                    air_pocket_probability: 0.5,
                    mossiness: 0.2,
                    overgrown: false,
                    vines: false,
                    replace_with_blackstone: false,
                }
            }
        }
        StructureKeys::RuinedPortalMountain => {
            if random.next_f32() < 0.5 {
                PortalSetup {
                    placement: Mountain,
                    air_pocket_probability: 1.0,
                    mossiness: 0.2,
                    overgrown: false,
                    vines: false,
                    replace_with_blackstone: false,
                }
            } else {
                PortalSetup {
                    placement: Land,
                    air_pocket_probability: 0.5,
                    mossiness: 0.2,
                    overgrown: false,
                    vines: false,
                    replace_with_blackstone: false,
                }
            }
        }
        StructureKeys::RuinedPortalDesert => PortalSetup {
            placement: Buried,
            air_pocket_probability: 0.0,
            mossiness: 0.0,
            overgrown: false,
            vines: false,
            replace_with_blackstone: false,
        },
        StructureKeys::RuinedPortalJungle => PortalSetup {
            placement: Land,
            air_pocket_probability: 0.5,
            mossiness: 0.8,
            overgrown: true,
            vines: true,
            replace_with_blackstone: false,
        },
        StructureKeys::RuinedPortalNether => PortalSetup {
            placement: Nether,
            air_pocket_probability: 0.5,
            mossiness: 0.0,
            overgrown: false,
            vines: false,
            replace_with_blackstone: true,
        },
        StructureKeys::RuinedPortalOcean => PortalSetup {
            placement: OceanFloor,
            air_pocket_probability: 0.0,
            mossiness: 0.8,
            overgrown: false,
            vines: false,
            replace_with_blackstone: false,
        },
        StructureKeys::RuinedPortalSwamp => PortalSetup {
            placement: OceanFloor,
            air_pocket_probability: 0.0,
            mossiness: 0.5,
            overgrown: false,
            vines: true,
            replace_with_blackstone: false,
        },
        _ => unreachable!("non-portal structure passed to ruined portal generator"),
    }
}

fn portal_bounds(
    template: &StructureTemplate,
    origin_x: i32,
    origin_z: i32,
    rotation: Rotation,
    mirror: Mirror,
    pivot: Vector3<i32>,
) -> BlockBox {
    let mut min_x = i32::MAX;
    let mut min_z = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_z = i32::MIN;
    for point in [
        Vector3::new(0, 0, 0),
        Vector3::new(template.size.x - 1, 0, 0),
        Vector3::new(0, 0, template.size.z - 1),
        Vector3::new(template.size.x - 1, 0, template.size.z - 1),
    ] {
        let point = match mirror {
            Mirror::None => point,
            Mirror::LeftRight => Vector3::new(2 * pivot.x - point.x, 0, point.z),
            Mirror::FrontBack => Vector3::new(point.x, 0, 2 * pivot.z - point.z),
        };
        let dx = point.x - pivot.x;
        let dz = point.z - pivot.z;
        let point = match rotation {
            Rotation::None => point,
            Rotation::Clockwise90 => Vector3::new(pivot.x - dz, 0, pivot.z + dx),
            Rotation::Rotate180 => Vector3::new(pivot.x - dx, 0, pivot.z - dz),
            Rotation::CounterClockwise90 => Vector3::new(pivot.x + dz, 0, pivot.z - dx),
        };
        min_x = min_x.min(origin_x + point.x);
        min_z = min_z.min(origin_z + point.z);
        max_x = max_x.max(origin_x + point.x);
        max_z = max_z.max(origin_z + point.z);
    }
    BlockBox::new(min_x, -64, min_z, max_x, 320, max_z)
}

impl StructureGenerator for RuinedPortalGenerator {
    fn get_structure_position(
        &self,
        mut context: StructureGeneratorContext<'_>,
    ) -> Option<StructurePosition> {
        let chunk_center_x = get_center_x(context.chunk_x);
        let chunk_center_z = get_center_z(context.chunk_z);
        let setup = setup_for(self.variant, &mut context.random);
        let air_pocket = setup.air_pocket_probability == 1.0
            || (setup.air_pocket_probability != 0.0
                && context.random.next_f32() < setup.air_pocket_probability);
        let templates = if context.random.next_f32() < 0.05 {
            &PORTALS[10..]
        } else {
            &PORTALS[..10]
        };
        let template_name =
            templates[context.random.next_bounded_i32(templates.len() as i32) as usize];
        let template = get_template(template_name)?;
        let rotation = Rotation::from_index(context.random.next_bounded_i32(4) as u8);
        let mirror = if context.random.next_f32() < 0.5 {
            Mirror::None
        } else {
            Mirror::FrontBack
        };
        let pivot = Vector3::new(template.size.x / 2, 0, template.size.z / 2);
        let origin_x = start_block_x(context.chunk_x);
        let origin_z = start_block_z(context.chunk_z);
        let bounding_box = portal_bounds(&template, origin_x, origin_z, rotation, mirror, pivot);
        // All vertical random choices are taken at structure-start time.  The
        // surface height becomes available only during the chunk pass.
        let vertical_roll = context.random.next_f32();

        let mut collector = StructurePiecesCollector::default();
        collector.add_piece(Box::new(RuinedPortalPiece {
            piece: StructurePiece::new(StructurePieceType::RuinedPortal, bounding_box, 0),
            template,
            rotation,
            mirror,
            pivot,
            variant: self.variant,
            setup,
            air_pocket,
            origin_x,
            origin_z,
            vertical_roll,
            placed_y: None,
        }));

        Some(StructurePosition {
            start_pos: BlockPos::new(chunk_center_x, 64, chunk_center_z),
            collector: Arc::new(collector.into()),
        })
    }
}

pub struct RuinedPortalPiece {
    piece: StructurePiece,
    template: Arc<StructureTemplate>,
    rotation: Rotation,
    mirror: Mirror,
    pivot: Vector3<i32>,
    variant: StructureKeys,
    setup: PortalSetup,
    air_pocket: bool,
    origin_x: i32,
    origin_z: i32,
    vertical_roll: f32,
    placed_y: Option<i32>,
}

impl StructurePieceBase for RuinedPortalPiece {
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
        let target_y = *self.placed_y.get_or_insert_with(|| {
            let height_map = if matches!(self.setup.placement, PortalPlacement::OceanFloor) {
                pumpkin_util::HeightMap::OceanFloorWg
            } else {
                pumpkin_util::HeightMap::WorldSurfaceWg
            };
            let surface_y = chunk.get_top_y(&height_map, self.origin_x, self.origin_z) - 1;
            let span = self.template.size.y;
            match self.setup.placement {
                PortalPlacement::Land | PortalPlacement::OceanFloor => surface_y,
                PortalPlacement::Buried => surface_y - span + 2 + (self.vertical_roll * 7.0) as i32,
                PortalPlacement::Mountain => {
                    let max = surface_y - span;
                    if 70 < max {
                        70 + (self.vertical_roll * (max - 70 + 1) as f32) as i32
                    } else {
                        max
                    }
                }
                PortalPlacement::Underground => {
                    let min = chunk.bottom_y() as i32 + 15;
                    let max = surface_y - span;
                    if min < max {
                        min + (self.vertical_roll * (max - min + 1) as f32) as i32
                    } else {
                        max
                    }
                }
                PortalPlacement::Nether => {
                    if self.air_pocket {
                        32 + (self.vertical_roll * 69.0) as i32
                    } else {
                        27 + (self.vertical_roll * 74.0) as i32
                    }
                }
            }
        });
        let final_origin = Vector3::new(self.origin_x, target_y, self.origin_z);

        // Air-pocket portals must retain template air; the other variants use
        // the normal structure-and-air ignore processor.
        place_template_with_settings(
            chunk,
            &self.template,
            final_origin,
            (0, 0),
            self.rotation,
            self.mirror,
            self.pivot,
            !self.air_pocket,
            matches!(self.setup.placement, PortalPlacement::OceanFloor),
            &[],
            Some(chunk_box),
        );
        let _ = (
            self.variant,
            self.setup.mossiness,
            self.setup.overgrown,
            self.setup.vines,
            self.setup.replace_with_blackstone,
        );
    }
}
