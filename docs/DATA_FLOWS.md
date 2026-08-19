# Потоки данных и границы ответственности

## Player login

```text
TCP/RakNet accept
  → packet decoder
  → handshake/status/login/config validation
  → authentication/encryption/compression
  → Player + Entity registration
  → chunk tickets and spawn position
  → registry/recipe/tag packets
  → play state
```

Java path: `pumpkin/src/net/java/{pending,login,configuration,play}.rs`.
Bedrock path: `pumpkin/src/net/bedrock/` plus `pumpkin-protocol/src/bedrock/`.
Shared state is created in `entity/player.rs`, `server/mod.rs` and `world/mod.rs`.

Recipe state follows the same lifecycle:

```text
Player NBT read (`recipeBook.recipes`)
  → Player::recipe_book (authoritative server set)
  → /recipe give|take or plugin API
  → CRecipeBookAdd for Java clients
  → Player NBT write on save/disconnect
```

`server/recipe.rs::recipe_id` is the single ID mapping used by commands and
login synchronization. Dynamic cooking recipes keep their vanilla ID, while
dynamic shaped/shapeless recipes retain a canonical namespaced key in the
server registry and recipe-book NBT. The remaining migration task is to assign
stable keys to every generated static recipe and reject legacy payloads whose
crafting identity is inferred only from the output item.

## Block placement/use

```text
UseItemOn packet
  → edition-specific handler
  → reach/gamemode/permission checks
  → world block + state lookup
  → ItemBehaviour / Block::normal_use / placement
  → set state with BlockFlags
  → neighbor/game-event/redstone/fluid updates
  → consume item and send block/inventory feedback
```

Never consume before the behavior returns success. A block replacement must copy
shared state properties and must notify both client and server-side neighbors.

## Redstone

```text
block state/neighbor change
  → neighbor notification
  → calculate weak/strong power
  → schedule block tick with delay + priority
  → scheduler ordered queue/inflight set
  → on_scheduled_tick
  → state update
  → repeat until stable, with bounded chained updates
```

Relevant code: `block/blocks/redstone/`, `block/entities/{comparator,hopper,
piston}.rs`, `pumpkin-world/src/tick/scheduler.rs`, `world/mod.rs`. The common
failure modes are reading the wrong side for strong power, using `unwrap()` on a
non-horizontal direction, losing scheduled priority, and recursive neighbor
updates that overflow the stack.

## Entity tick and packet emission

```text
active chunk/entity list
  → EntityBase tick
  → movement/collision/passengers
  → AI/effects/damage
  → tracked metadata changes
  → per-player visibility/tracking
  → Java/Bedrock spawn/move/metadata/remove packets
```

Entity IDs are session-local; UUIDs are persistent. A missing target during an
incoming interaction is a normal despawn race and must not kick a player except
for explicit protocol violations such as self-attack where vanilla disconnects.

## Chunk load/generation/save

```text
player ticket
  → chunk_system DAG/holder
  → disk read (Anvil/Pump/Linear) or proto generation
  → terrain → biomes → carvers → features → structures → light
  → full chunk + block entities
  → packet serialization
  → dirty tracking
  → block entity flush + chunk encode + region write
```

Generation lives in `pumpkin-world/src/generation`; the holder/DAG is in
`chunk_system`. Save ordering matters: serialize live block entities before
unload, and do not drop unknown NBT/ticks when round-tripping a vanilla chunk.

## Inventory click

```text
click packet
  → Click decode/version mapping
  → ScreenHandler click/drag semantics
  → Slot validation + item merge/split
  → recipe/furnace/brewing result update
  → authoritative inventory sync
```

`pumpkin-inventory` owns slot math and screen state; `pumpkin` owns player
inventory, block entity inventory and network adapter. Client-provided slot
contents are never authoritative.

## Plugin event

```text
runtime event
  → plugin host event conversion
  → WIT event id/handler registry
  → permission/sandbox boundary
  → plugin callback
  → cancellable result or modified event
  → runtime commit
```

Event conversion must preserve cancellation, identity, mutable item stacks and
edition-specific packet data. Use plugin API types, not internal runtime structs.
