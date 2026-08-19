use pumpkin_util::math::vector2::Vector2;
use std::{num::NonZeroU8, sync::Arc};

use pumpkin_protocol::{
    bedrock::client::network_chunk_publisher_update::CNetworkChunkPublisherUpdate,
    java::client::play::{CCenterChunk, CUnloadChunk},
};
use pumpkin_world::cylindrical_chunk_iterator::Cylindrical;

use crate::{
    entity::{EntityBase, player::Player},
    net::ClientPlatform,
};

pub fn get_view_distance(player: &Player) -> NonZeroU8 {
    let fallback = NonZeroU8::new(2).unwrap_or(NonZeroU8::MIN);
    let Some(server) = player.world().server.upgrade() else {
        return fallback;
    };
    let max_view_distance = match player.client.as_ref() {
        ClientPlatform::Java(_) => server.advanced_config.networking.java.view_distance,
        ClientPlatform::Bedrock(_) => server.advanced_config.networking.bedrock.view_distance,
    };
    player
        .config
        .load()
        .view_distance
        .clamp(fallback, max_view_distance)
}

// Checks if the target chunk is within the view distance
// of the center chunk. Uses Chebyshev distance.
#[must_use]
#[inline]
pub fn is_within_view_distance(
    center: Vector2<i32>,
    target: Vector2<i32>,
    view_distance: i32,
) -> bool {
    (target.x - center.x).abs().max((target.y - center.y).abs()) <= view_distance
}

pub async fn update_position(player: &Arc<Player>) {
    let entity = &player.get_entity();
    let new_chunk_center = entity.chunk_pos.load();
    let old_cylindrical = player.watched_section.load();

    // This does break when a new player spawns
    // if old_cylindrical.center == new_chunk_center {
    //     return;
    // }

    let view_distance = get_view_distance(player);
    let simulation_distance =
        player
            .world()
            .server
            .upgrade()
            .map_or(10, |server| match player.client.as_ref() {
                ClientPlatform::Java(_) => server
                    .advanced_config
                    .networking
                    .java
                    .simulation_distance
                    .get(),
                ClientPlatform::Bedrock(_) => server
                    .advanced_config
                    .networking
                    .bedrock
                    .simulation_distance
                    .get(),
            });
    let new_cylindrical = Cylindrical::new(new_chunk_center, view_distance);

    let simulation_distance_matches = {
        let chunk_manager = player.chunk_manager.lock().await;
        chunk_manager.simulation_distance_matches(simulation_distance)
    };
    if old_cylindrical == new_cylindrical && simulation_distance_matches {
        return;
    }

    match player.client.as_ref() {
        ClientPlatform::Java(java_client) => {
            java_client
                .send_packet_now(&CCenterChunk {
                    chunk_x: new_chunk_center.x.into(),
                    chunk_z: new_chunk_center.y.into(),
                })
                .await;
        }
        ClientPlatform::Bedrock(bedrock_client) => {
            bedrock_client
                .send_game_packet(&CNetworkChunkPublisherUpdate::new(
                    player.get_entity().block_pos.load(),
                    u32::from(view_distance.get()) * 16,
                ))
                .await;
        }
    }

    let (loading_iter, unloading_iter) =
        Cylindrical::changed_chunks(old_cylindrical, new_cylindrical);
    let loading_chunks: Vec<_> = loading_iter.collect();
    let unloading_chunks: Vec<_> = unloading_iter.collect();

    // Use the chunk_manager's world reference, which is updated on dimension change.
    // This ensures we load chunks from the correct world after portal teleportation.
    // Close entity pairings before the client forgets terrain.  Otherwise the
    // next visit to the same chunk would be treated as already tracked and
    // minecarts/projectiles could remain invisible.
    let world = {
        let world = player.world().clone();
        world.remove_tracked_entities_in_chunks(player, &unloading_chunks);
        let mut chunk_manager = player.chunk_manager.lock().await;
        let world = chunk_manager.world().clone();
        chunk_manager.update_center_and_view_distance(
            new_chunk_center,
            view_distance.into(),
            simulation_distance,
            &world.level,
            &loading_chunks,
            &unloading_chunks,
        );
        world
    };
    player.watched_section.store(new_cylindrical);

    if let ClientPlatform::Java(client) = player.client.as_ref() {
        for chunk in &unloading_chunks {
            client
                .enqueue_packet(&CUnloadChunk::new(chunk.x, chunk.y))
                .await;
        }
    }

    // Make sure the watched section and the chunk watcher updates are async atomic. We want to
    // ensure what we unload when the player disconnects is correct.
    world
        .level
        .mark_chunks_as_newly_watched(&loading_chunks)
        .await;
    let chunks_to_clean = world
        .level
        .mark_chunks_as_not_watched(&unloading_chunks)
        .await;

    if !chunks_to_clean.is_empty() {
        world.remove_entities_in_chunks(&chunks_to_clean).await;
        world.level.clean_entity_chunks(&chunks_to_clean);
    }

    if !loading_chunks.is_empty() {
        world.spawn_world_entity_chunks(player.clone(), loading_chunks, new_chunk_center);
    }
}
