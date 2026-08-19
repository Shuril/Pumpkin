use std::pin::Pin;
use std::sync::Arc;

use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component_impl::CustomNameImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;

pub struct NameTagItem;

impl ItemMetadata for NameTagItem {
    fn ids() -> Box<[u16]> {
        [Item::NAME_TAG.id].into()
    }
}

impl ItemBehaviour for NameTagItem {
    fn use_on_entity<'a>(
        &'a self,
        item: &'a mut ItemStack,
        player: &'a Player,
        entity: Arc<dyn EntityBase>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let entity = entity.get_entity();
            let has_name = item.get_data_component::<CustomNameImpl>().is_some();
            if name_tag_can_apply(
                entity.entity_type.saveable,
                entity.get_living_entity().is_some(),
                entity.is_alive(),
                has_name,
            ) && let Some(name) = item.get_data_component::<CustomNameImpl>()
            {
                entity.set_custom_name(name.name.clone());
                if entity.entity_type.mob {
                    entity.set_persistence_required();
                }
                item.decrement_unless_creative(player.gamemode.load(), 1);
            }
        })
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[inline]
fn name_tag_can_apply(saveable: bool, living: bool, alive: bool, has_name: bool) -> bool {
    saveable && living && alive && has_name
}

#[cfg(test)]
mod tests {
    use super::name_tag_can_apply;

    #[test]
    fn name_tags_require_a_live_serializable_living_entity_and_name() {
        assert!(name_tag_can_apply(true, true, true, true));
        assert!(!name_tag_can_apply(false, true, true, true));
        assert!(!name_tag_can_apply(true, false, true, true));
        assert!(!name_tag_can_apply(true, true, false, true));
        assert!(!name_tag_can_apply(true, true, true, false));
    }

    #[test]
    fn successful_mob_name_tag_path_is_the_persistence_boundary() {
        // The actual atomic flag lives on Entity; this table pins the vanilla
        // gate used before calling `set_persistence_required`.
        assert!(name_tag_can_apply(true, true, true, true));
    }
}
