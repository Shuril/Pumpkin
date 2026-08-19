use pumpkin_data::BlockState;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, fluid::Fluid, tag};

/// Returns whether `state` is a vanilla-style liquid container for `fluid`.
///
/// Minecraft does not replace a slab, stair, rail, sign, etc. when water flows
/// into it if the block exposes the `waterlogged` property.  Instead the
/// existing block state is retained and that property is set to `true`.
/// Double slabs are the important exception: their collision shape cannot be
/// waterlogged, and vanilla `SlabBlock#canPlaceLiquid` rejects them.
#[must_use]
pub fn can_hold_fluid(block_state: &BlockState, block: &Block, fluid: &Fluid) -> bool {
    let is_water = fluid.id == Fluid::WATER.id || fluid.id == Fluid::FLOWING_WATER.id;
    if !is_water || block.is_waterlogged(block_state.id) {
        return false;
    }

    let Some(properties) = block.properties(block_state.id) else {
        return false;
    };
    let props = properties.to_props();
    props.iter().any(|(key, _)| *key == "waterlogged")
        && !props
            .iter()
            .any(|(key, value)| *key == "type" && *value == "double")
}

/// Computes the state produced by placing a water fluid into a waterloggable
/// block, preserving every other property (facing, shape, powered, ...).
#[must_use]
pub fn waterlogged_state(
    block_state: &BlockState,
    block: &Block,
    fluid: &Fluid,
) -> Option<pumpkin_data::BlockStateId> {
    can_hold_fluid(block_state, block, fluid)
        .then(|| block_state.with_waterlogged().map(|state| state.id))
        .flatten()
}

/// Check if a specific block can be replaced by fluid (based on block properties)
#[must_use]
pub fn can_be_replaced(block_state: &BlockState, block: &Block, fluid: &Fluid) -> bool {
    // Waterlogged blocks should not be replaced by water
    if block.is_waterlogged(block_state.id) {
        return false;
    }

    // A dry waterloggable block is a valid fluid destination.  The caller
    // must use `waterlogged_state` rather than replacing it with a liquid
    // block state.
    if can_hold_fluid(block_state, block, fluid) {
        return true;
    }

    // Fluid Logic
    if let Some(other_fluid) = Fluid::from_state_id(block_state.id) {
        if !fluid.matches_type(other_fluid) {
            return true;
        }
        // Replace current fluid if it is a falling source
        if other_fluid.is_source(block_state.id) && other_fluid.is_falling(block_state.id) {
            return true;
        }
    }

    let id = block.id;

    // Blocks that fluid should never replace
    if block.has_tag(&tag::Block::MINECRAFT_DOORS)
        || block.has_tag(&tag::Block::MINECRAFT_BEDS)
        || block.has_tag(&tag::Block::MINECRAFT_LEAVES)
        || block.has_tag(&tag::Block::MINECRAFT_PRESSURE_PLATES)
        || block.has_tag(&tag::Block::C_CLUSTERS)
        || block.has_tag(&tag::Block::MINECRAFT_WALL_CORALS)
        || block.has_tag(&tag::Block::MINECRAFT_SHULKER_BOXES)
        || block.has_tag(&tag::Block::MINECRAFT_PORTALS)
        || id == Block::BELL.id
        || id == Block::BIG_DRIPLEAF.id
        || id == Block::BIG_DRIPLEAF_STEM.id
        || id == Block::SMALL_DRIPLEAF.id
        || id == Block::CAKE.id
        || id == Block::CONDUIT.id
        || id == Block::CAMPFIRE.id
        || id == Block::DRAGON_EGG.id
        || id == Block::KELP.id
        || id == Block::KELP_PLANT.id
        || id == Block::SEAGRASS.id
        || id == Block::TALL_SEAGRASS.id
        || id == Block::LADDER.id
        || id == Block::POINTED_DRIPSTONE.id
        || id == Block::SCAFFOLDING.id
    {
        return false;
    }

    // Only replace air, explicitly replaceable blocks, or carpets
    block_state.replaceable()
        || id == Block::AIR.id
        || block.has_tag(&tag::Block::MINECRAFT_WOOL_CARPETS)
        // Only use PistonBehavior::Destroy if it didn't pass the checks above
        || block_state.piston_behavior == pumpkin_data::block_state::PistonBehavior::Destroy
}

#[cfg(test)]
mod tests {
    use super::{can_be_replaced, can_hold_fluid, waterlogged_state};
    use pumpkin_data::{Block, fluid::Fluid};

    #[test]
    fn water_can_fill_dry_waterloggable_states_without_losing_properties() {
        let dry = Block::OAK_SLAB
            .states
            .iter()
            .find(|state| !state.is_waterlogged())
            .expect("oak slabs expose dry waterlogged states");

        assert!(can_hold_fluid(dry, &Block::OAK_SLAB, &Fluid::FLOWING_WATER));
        assert!(can_be_replaced(
            dry,
            &Block::OAK_SLAB,
            &Fluid::FLOWING_WATER
        ));
        let wet = waterlogged_state(dry, &Block::OAK_SLAB, &Fluid::WATER).unwrap();
        assert!(Block::OAK_SLAB.is_waterlogged(wet));
        let dry_props = Block::OAK_SLAB.properties(dry.id).unwrap().to_props();
        let wet_props = Block::OAK_SLAB.properties(wet).unwrap().to_props();
        for (key, value) in dry_props {
            if key != "waterlogged" {
                assert!(wet_props.iter().any(|(k, v)| *k == key && *v == value));
            }
        }
    }

    #[test]
    fn double_slabs_and_lava_do_not_waterlog() {
        let double = Block::OAK_SLAB
            .states
            .iter()
            .find(|state| {
                Block::OAK_SLAB
                    .properties(state.id)
                    .unwrap()
                    .to_props()
                    .iter()
                    .any(|(key, value)| *key == "type" && *value == "double")
            })
            .unwrap();
        assert!(!can_hold_fluid(double, &Block::OAK_SLAB, &Fluid::WATER));
        assert!(!can_hold_fluid(
            Block::OAK_SLAB.default_state,
            &Block::OAK_SLAB,
            &Fluid::LAVA
        ));
    }
}
