use crate::entity::{Entity, EntityBase};
use std::pin::Pin;

pub enum EntityPredicate<'a> {
    ValidEntity,
    ValidLivingEntity,
    NotMounted,
    ValidInventories,
    ExceptCreativeOrSpectator,
    ExceptSpectator,
    CanCollide,
    CanHit,
    Rides(&'a Entity),
}

/// Vanilla's melee-target predicate excludes creative and spectator players,
/// while ordinary entities and survival/adventure players remain valid.  Keep
/// the truth table independent from the async entity adapter so the polarity
/// cannot silently regress when the predicate is used by another goal.
#[must_use]
const fn except_creative_or_spectator(
    is_player: bool,
    is_spectator: bool,
    is_creative: bool,
) -> bool {
    !is_player || (!is_spectator && !is_creative)
}

impl EntityPredicate<'_> {
    pub fn test<'b>(
        &'b self,
        entity: &'b Entity,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'b>> {
        Box::pin(async move {
            match self {
                EntityPredicate::ValidEntity => entity.is_alive(),
                EntityPredicate::ValidLivingEntity => {
                    entity.is_alive() && entity.get_living_entity().is_some()
                }
                EntityPredicate::NotMounted => {
                    entity.is_alive()
                        && !entity.has_passengers().await
                        && !entity.has_vehicle().await
                }
                EntityPredicate::ValidInventories => {
                    // Entity containers are the inventory-bearing minecart
                    // variants.  This mirrors vanilla's
                    // `VALID_INVENTORIES` predicate (`isAlive && Container`)
                    // without consuming the Arc required by the inventory
                    // accessor; ordinary minecarts, players and mobs are not
                    // valid hopper/entity-inventory targets.
                    entity.is_alive()
                        && matches!(
                            entity.entity_type.id,
                            id if id == pumpkin_data::entity::EntityType::CHEST_MINECART.id
                                || id == pumpkin_data::entity::EntityType::HOPPER_MINECART.id
                        )
                }
                EntityPredicate::ExceptCreativeOrSpectator => {
                    entity.get_player().is_none_or(|player| {
                        except_creative_or_spectator(
                            true,
                            player.is_spectator(),
                            player.is_creative(),
                        )
                    })
                }
                EntityPredicate::ExceptSpectator => !entity.is_spectator(),
                EntityPredicate::CanCollide => {
                    EntityPredicate::ExceptSpectator.test(entity).await
                        && entity.is_collidable(None)
                }
                EntityPredicate::CanHit => {
                    EntityPredicate::ExceptSpectator.test(entity).await && entity.can_hit()
                }
                EntityPredicate::Rides(target_entity) => {
                    let target: &Entity = target_entity;

                    let mut opt_vehicle_arc = {
                        let vehicle_lock = entity.vehicle.lock().await;
                        vehicle_lock.clone()
                    };

                    while let Some(vehicle_arc) = opt_vehicle_arc {
                        let vehicle_entity_base: &dyn EntityBase = &*vehicle_arc;
                        let target_base: &dyn EntityBase = target;

                        if std::ptr::eq(vehicle_entity_base, target_base) {
                            return false;
                        }

                        opt_vehicle_arc = {
                            let vehicle_lock =
                                vehicle_entity_base.get_entity().vehicle.lock().await;
                            vehicle_lock.clone()
                        }
                    }
                    true
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::except_creative_or_spectator;

    #[test]
    fn except_creative_or_spectator_matches_vanilla_target_filter() {
        assert!(except_creative_or_spectator(false, false, false));
        assert!(except_creative_or_spectator(true, false, false));
        assert!(!except_creative_or_spectator(true, true, false));
        assert!(!except_creative_or_spectator(true, false, true));
        assert!(!except_creative_or_spectator(true, true, true));
    }
}
