# Каталог модулей

Пути ниже — карта поиска. Если нужной функции нет в узком файле, переходите к
родительскому `mod.rs`: именно там обычно находится trait, registry или общий
контракт.

## `crates/pumpkin`

### Runtime root

- `src/main.rs` — executable entry point and CLI.
- `src/lib.rs` — logger, global shutdown flags, `PumpkinServer` bootstrap.
- `src/server/` — `Server`, tick rate, task scheduler, recipe manager, key store,
  connection cache and seasonal hooks.
- `src/world/mod.rs` — gameplay world façade, players/entities, block mutation,
  tick integration, save/unload and world services.
- `src/entity/mod.rs` — `Entity`, `EntityBase`, NBT contracts, flags and common
  entity operations.
- `src/entity/player.rs` — player state, login lifecycle, inventory, respawn,
  movement, statistics, advancements, server-side recipe-book persistence and
  client sync. `Player::recipe_book` is the authoritative set of learned
  recipe keys; use `unlock_recipe`, `revoke_recipe` and `known_recipes` instead
  of maintaining a second per-player map.

### Blocks and items

- `src/block/mod.rs` — block behavior traits, block state lookup and common drops.
- `src/block/registry.rs` — runtime block and block-entity registration.
- `src/block/blocks/` — behavior by block family. Redstone is in
  `blocks/redstone/`, fluids in `block/fluid/`, piston behavior in
  `blocks/piston/`.
- `src/block/entities/` — mutable block entities: inventories, furnaces,
  hoppers, comparator, piston, sculk, crafter, dispenser and signs.
- `src/item/mod.rs` — `ItemBehaviour`, item registry and generic item contracts.
- `src/item/items/` — concrete player/dispenser behaviors, one file per item or
  family. `items/mod.rs` is the registration hub.
- `src/item/potion.rs` — potion effects and item/component behavior.

### Entities and AI

- `entity/living.rs` — health, damage, armor, effects, death and equipment.
- `entity/mob/` — hostile/passive mobs and mob-specific goals.
- `entity/ai/` — goal selectors, navigation, target selection and sensors.
- `entity/projectile/` — arrows, thrown items, tridents and projectile physics.
- `entity/vehicle/` — boats and minecarts. Minecart submodules contain rideable,
  chest, furnace, hopper and TNT behavior.
- `entity/player/` — advancement, statistics and player-specific support.

### Networking and commands

- `src/net/java/` — Java connection state machine, packet handlers and play
  actions.
- `src/net/bedrock/` — Bedrock/RakNet/NetherNet sessions, forms and status.
- `src/net/authentication.rs` — Mojang auth/profile and key handling.
- `src/net/query.rs`, `rcon/`, `lan_broadcast.rs` — auxiliary server protocols.
- `src/command/dispatcher.rs` — command tree and execution.
- `src/command/args/` — parsers and typed arguments.
- `src/command/commands/` — command implementations; `commands/mod.rs` is the
  registration list.

### World services and plugins

- `src/world/chunker.rs` — player chunk visibility/tickets.
- `src/world/explosion.rs` — explosion ray/blocks/entity damage.
- `src/world/loot.rs` — loot predicates and context.
- `src/world/natural_spawner.rs` — mob spawning.
- `src/world/scoreboard.rs`, `bossbar.rs`, `border.rs`, `time.rs`, `weather.rs` —
  world-visible services.
- `src/plugin/` — plugin loader, permissions, events, command bridge and WASM
  host integration.
- `src/data/` — server files: bans, operators, whitelist, player data,
  advancements and user cache.

## `crates/pumpkin-world`

- `src/level.rs` — level/chunk access façade and chunk scheduling API.
- `src/world.rs` — storage-independent block/chunk access traits.
- `src/chunk/` — chunk sections, palettes, mutations and chunk format.
- `src/chunk/format/{anvil,pump,linear}.rs` — persistence backends.
- `src/chunk/io/` — region/file manager and async IO.
- `src/chunk_system/` — holder, DAG, loading, generation workers, cache and
  lifecycle state.
- `src/tick/` — `ScheduledTick`, priority ordering and inflight scheduler.
- `src/lighting/` — light engine, runtime propagation and section storage.
- `src/generation/` — biome/noise/terrain/carvers/features/structures/template
  processors and proto chunks.
- `src/poi/` — points of interest and region storage.
- `src/world_info/` — `level.dat`, dimension and world metadata.
- `src/inventory/` and `src/block/` — storage-level inventory/property traits.

## `crates/pumpkin-data`

- `src/generated/` — generated immutable registries and ID/state tables.
- `src/block_state.rs`, `block_direction.rs`, `block_rotation.rs` — state and
  transform helpers.
- `src/item_stack/` — item stack and component representation.
- `src/data_component_impl/` — typed component adapters.

## Supporting crates

- `pumpkin-protocol/src/codec/` — primitive and composite wire codecs.
- `pumpkin-protocol/src/serial/` — low-level Java/Bedrock read/write traits.
- `pumpkin-protocol/src/java/`, `bedrock/` — edition packet definitions.
- `pumpkin-inventory/src/` — click, slot, screen, crafting, furnace, brewing,
  merchant, anvil, beacon and player inventory handlers.
- `pumpkin-nbt/src/` — tags, compound, Java/Bedrock readers/writers, NBT ops.
- `pumpkin-codecs/src/` — `Codec`, `DataResult`, DynamicOps and builders.
- `pumpkin-util/src/math/` — positions/vectors/boxes/storage; `random/` — Java
  legacy and Xoroshiro implementations; `text/` — chat components.
- `pumpkin-config/src/` — root configuration and feature-specific sections.
- `pumpkin-plugin-api/src/` — WIT plugin API, events, commands, permissions,
  forms and scheduler.
- `tools/pumpkin-codegen/src/` — data builders; `wit/` — WIT generation;
  `remap/` — ID remapping.

## Generated module rule

If a symbol comes from `pumpkin_data::generated`, search its builder in
`tools/pumpkin-codegen/src` and its asset input before editing generated Rust.
The generated file is an output, not the source of truth.
