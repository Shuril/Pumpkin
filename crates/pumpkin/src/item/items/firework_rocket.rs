use std::pin::Pin;
use std::sync::Arc;

use crate::entity::player::Player;
use crate::entity::projectile::firework_rocket::FireworkRocketEntity;
use crate::entity::{Entity, EntityBase};
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::Block;
use pumpkin_data::BlockDirection;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_util::Hand;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

pub struct FireworkRocketItem;

#[inline]
fn firework_placement_position(
    location: BlockPos,
    face: BlockDirection,
    cursor_pos: Vector3<f32>,
) -> Vector3<f64> {
    let face_offset = face.to_offset();
    Vector3::new(
        f64::from(location.0.x) + f64::from(cursor_pos.x) + f64::from(face_offset.x) * 0.15,
        f64::from(location.0.y) + f64::from(cursor_pos.y) + f64::from(face_offset.y) * 0.15,
        f64::from(location.0.z) + f64::from(cursor_pos.z) + f64::from(face_offset.z) * 0.15,
    )
}

impl ItemMetadata for FireworkRocketItem {
    fn ids() -> Box<[u16]> {
        [Item::FIREWORK_ROCKET.id].into()
    }
}

impl ItemBehaviour for FireworkRocketItem {
    fn use_on_block<'a>(
        &'a self,
        item: &'a mut ItemStack,
        player: &'a Player,
        location: BlockPos,
        face: BlockDirection,
        cursor_pos: Vector3<f32>,
        _block: &'a Block,
        _server: &'a Server,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            // Vanilla reserves use-on-block while fall-flying for the hand
            // use path, which attaches the rocket to the player for an elytra
            // boost.  Spawning a free rocket here would consume the item and
            // bypass that path on clients that send both interactions.
            if player.get_entity().is_fall_flying() {
                return;
            }
            let world = player.world();
            let entity = Entity::new(
                world.clone(),
                firework_placement_position(location, face, cursor_pos),
                &EntityType::FIREWORK_ROCKET,
            );
            let entity = FireworkRocketEntity::new_with_item(entity, item.clone());
            world.spawn_entity(Arc::new(entity)).await;
            item.decrement_unless_creative(player.gamemode.load(), 1);
            world
                .emit_game_event_from_item(
                    location,
                    crate::world::game_event::GameEventKind::EntityPlace,
                    Some(player.get_entity().entity_uuid),
                    item,
                )
                .await;
        })
    }

    fn normal_use<'a>(
        &'a self,
        _item: &'a Item,
        player: &'a Player,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {
            if player.get_entity().is_fall_flying() {
                let world = player.world();
                // The projectile carries the exact stack used by the player.
                // In particular, the Fireworks component controls flight time
                // and explosion payload; constructing it from a bare item
                // silently turned every rocket into the default 1-flight,
                // no-explosion variant.
                let mut stack = player.inventory().held_item().await;
                let entity = Entity::new(
                    world.clone(),
                    player.get_entity().pos.load(),
                    &EntityType::FIREWORK_ROCKET,
                );
                let entity = FireworkRocketEntity::new_shot_with_item(
                    entity,
                    player.get_entity(),
                    stack.clone(),
                );
                world.spawn_entity(Arc::new(entity)).await;
                stack.decrement_unless_creative(player.gamemode.load(), 1);
                player.inventory().set_held_item(stack).await;
            }
        })
    }

    fn normal_use_with_hand<'a>(
        &'a self,
        item: &'a Item,
        player: &'a Player,
        hand: Hand,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        let _ = item;
        Box::pin(async move {
            if !player.get_entity().is_fall_flying() {
                return;
            }
            let world = player.world();
            let inventory = player.inventory();
            let stack = inventory.get_stack_in_hand(hand).await;
            let entity = Entity::new(
                world.clone(),
                player.get_entity().pos.load(),
                &EntityType::FIREWORK_ROCKET,
            );
            let entity = FireworkRocketEntity::new_shot_with_item(
                entity,
                player.get_entity(),
                stack.clone(),
            );
            world.spawn_entity(Arc::new(entity)).await;
            let mut stack = inventory.get_stack_in_hand(hand).await;
            stack.decrement_unless_creative(player.gamemode.load(), 1);
            inventory.set_stack_in_hand(hand, stack).await;
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::firework_placement_position;
    use pumpkin_data::BlockDirection;
    use pumpkin_util::math::position::BlockPos;
    use pumpkin_util::math::vector3::Vector3;

    #[test]
    fn placement_position_matches_click_and_face_offset() {
        let position = firework_placement_position(
            BlockPos::new(10, 20, 30),
            BlockDirection::East,
            Vector3::new(0.25, 0.5, 0.75),
        );
        assert!((position.x - 10.4).abs() < f64::EPSILON);
        assert!((position.y - 20.5).abs() < f64::EPSILON);
        assert!((position.z - 30.75).abs() < f64::EPSILON);
    }
}
