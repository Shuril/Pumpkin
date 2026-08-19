# Архитектура Pumpkin

## 1. Ментальная модель

Pumpkin — это асинхронный сервер с разделением на четыре крупных слоя:

```text
client (Java TCP / Bedrock RakNet+NetherNet)
        │ packet decode/encode
        ▼
pumpkin-protocol ──► pumpkin (server, entities, blocks, commands, plugins)
                              │ world API
                              ▼
                    pumpkin-world (chunks, ticks, lighting, generation, IO)
                              │ static/generated data
                              ▼
                    pumpkin-data + pumpkin-util + pumpkin-nbt/codecs
```

`pumpkin` owns gameplay and network sessions. `pumpkin-world` deliberately does
not depend on gameplay entities; it owns chunk/world-generation primitives and
is safe to reuse in tools. `pumpkin-data` contains generated registries and
state tables. `pumpkin-protocol` knows wire formats, но не должен принимать
gameplay decisions.

## 2. Workspace crates

### Runtime crates

- **`pumpkin`** — executable server and all gameplay runtime. Entry point
  `crates/pumpkin/src/main.rs`; orchestration in `lib.rs`, `server/`, `world/`,
  `entity/`, `block/`, `item/`, `net/`, `command/` and `plugin/`.
- **`pumpkin-world`** — chunk storage, chunk pipeline, dimensions, tick queues,
  lighting, POI and Java-like world generation. It exposes `Level`, `ChunkData`,
  `ProtoChunk`, `WorldGenerator` and storage backends.
- **`pumpkin-protocol`** — Java/Bedrock packet types and serializers. The
  `packet` traits define version-aware packet contracts; `java/` and `bedrock/`
  are protocol-specific trees.
- **`pumpkin-inventory`** — inventories, slots, click semantics and container
  screen handlers. It is shared by player and block entities.

### Data and infrastructure crates

- **`pumpkin-data`** — generated blocks, states, items, entities, tags, recipes,
  sounds, packets, dimensions, worldgen registries and remap tables. Do not edit
  `src/generated/*` by hand; change inputs/codegen instead.
- **`pumpkin-util`** — math, positions, vectors, random implementations,
  text components, identifiers, loot and small cross-crate helpers.
- **`pumpkin-nbt`** — Java/Bedrock NBT tags, streaming readers/writers,
  compression and DynamicOps.
- **`pumpkin-codecs`** — typed codec/data-result layer used by registries and
  structured data.
- **`pumpkin-config`** — TOML configuration and validation. Runtime reads the
  immutable loaded configuration from `Server`/`World`.

### Extension and build crates

- **`pumpkin-plugin-api`** — WASM/WIT-facing plugin API, events, commands,
  scheduler, permissions and forms.
- **`pumpkin-api-macros`** — proc macros for plugin methods and runtime wrapping.
- **`pumpkin-macros`** — runtime proc macros used by Pumpkin internals.
- **`tools/pumpkin-codegen`** — turns tracked vanilla/Bedrock data into generated
  Rust. It writes to `crates/pumpkin-data/src/generated` and plugin WIT output.

## 3. Server lifecycle

1. `main.rs` parses CLI/environment and loads `PumpkinConfig`.
2. `VanillaData` loads generated registries/assets.
3. `PumpkinServer::new` creates `Server`, world instances, RCON/query/LAN and
   Java/Bedrock listeners.
4. `Ticker` drives the fixed server tick. `ServerTickRateManager` records the
   target rate and `TaskScheduler` handles delayed asynchronous work.
5. Each network connection advances through handshake/status/login/config/play.
   Java and Bedrock clients share gameplay objects but have separate packet
   codecs and session handlers.
6. `World::tick` updates time/weather, chunk tickets, entities, block entities,
   scheduled block/fluid ticks, random ticks, spawners and broadcasts.
7. Save/shutdown drains task trackers, serializes dirty block entities/chunks,
   flushes region backends and persists player data.

## 4. State ownership and concurrency

### World and chunks

`pumpkin::world::World` is the gameplay façade. It owns references to a
`pumpkin_world::level::Level`, server, players, entity storage, gamerules,
weather, border and world-specific services. Use `World` methods instead of
reaching into chunk internals from gameplay code.

`pumpkin-world::Level` owns chunk holders and the async chunk pipeline. A chunk
may be absent, proto, generating, loaded, saving or unloaded. Never assume that
an entity/block lookup implies a loaded chunk; return `Option`/`Result` and make
the race harmless.

### Entities

`Entity` is the common mutable state (id, UUID, position, velocity, rotation,
world, passengers, flags). `EntityBase` is the erased gameplay interface and
`LivingEntity` adds health, effects, attributes, equipment and damage. Concrete
entities use `Arc<dyn EntityBase>` when stored in world/player maps.

Entity persistence is split between `NBTStorage` and `NBTStorageInit`. Adding a
field requires: constructor default → NBT write → NBT read → spawn/metadata if
the client needs it → unload/save test.

### Blocks and block entities

`Block`/`BlockStateId` are generated immutable data. Runtime behavior lives in
`crates/pumpkin/src/block/blocks`; inventory/stateful blocks use
`block/entities`. A block implementation should be stateless; mutable contents,
timers and viewers belong in a block entity. Registry wiring is in
`block/registry.rs` and entity constructors in `block/entities/mod.rs`.

### Async rules

- Tokio is for IO and server orchestration.
- Rayon is for CPU-heavy generation/lighting work; do not block Tokio on Rayon.
- Do not hold an async mutex across network sends or long world operations.
- Read state, release the lock, perform world/network work, then reacquire for
  the commit where necessary.
- All entity/chunk lookups must tolerate despawn/unload races.

## 5. Gameplay tick order

The exact order is a parity contract. The relevant pieces are:

1. server tick rate and scheduled tasks;
2. world time/weather and chunk activation;
3. chunk block/fluid scheduler (`pumpkin-world/src/tick`);
4. random ticks and fluid physics;
5. block entity ticks (furnaces, hoppers, sculk, piston, etc.);
6. entity movement, collision, AI, effects, damage and passengers;
7. natural spawning and despawn;
8. player inventory/advancement/stat updates;
9. metadata, block updates and entity packets.

`ChunkTickScheduler` stores ordered ticks and inflight ticks; the priority and
sequence rules must not be replaced with a hash-map iteration. Redstone and
fluid behavior depend on this ordering.

## 6. Persistence formats

- **Anvil Java**: `pumpkin-world/src/chunk/format/anvil.rs` and `format/mod.rs`.
  Named block palettes use `{Name, Properties}`; biome palettes use resource
  locations. Region IO is in `chunk/io/file_manager.rs`.
- **Pump**: compact custom format in `format/pump.rs`.
- **Linear**: append/index format in `format/linear.rs`.
- **Player/level data**: `world_info/`, `pumpkin/src/data/` and entity/player
  NBT implementations.

When changing an on-disk structure, preserve unknown root tags, `DataVersion`,
`InhabitedTime`, block entities, scheduled ticks and coordinate validation.
Round-trip tests are mandatory; a successful server boot is not enough.

## 7. Generated data boundary

Inputs are assets, tracked-data JSON, Bedrock NBT/JSON and Mojang registry
exports. `tools/pumpkin-codegen/src/main.rs` registers builders. Generated
outputs include block state tables, packet types, tags, recipes, structures,
noise routers and remaps. Runtime code may add adapters and behavior, but must
not duplicate generated IDs or property indexing.

## 8. Plugin boundary

Plugins execute through WASM/WIT. `pumpkin-plugin-api` exposes a stable host
surface; `pumpkin` owns host dispatch and permission checks. A new plugin-facing
feature needs WIT schema, generated bindings, host implementation, permission
definition, event/command registration and a compatibility test. Do not expose
internal `Arc<World>` or generated implementation details directly.
