# Что ещё нужно сделать

Этот backlog объединяет найденные `TODO`/`FIXME`, parity gaps и архитектурные
риски. Приоритеты: **P0** ломает совместимость/сохранение/крашится; **P1**
видимое отличие vanilla; **P2** качество/расширяемость.

## P0 — сначала

- **Crafter**: основной recipe lookup, disabled-slot semantics, output
  insertion/ejection, cooldown, comparator output, NBT и client updates уже
  реализованы. Component-bearing datapack results и player-facing recipe
  remainders теперь сохраняются/возвращаются. Ejection учитывает face-aware
  `WorldlyContainer` insertion и merge vetoes; остаются точный порядок hopper
  операций, advancement hooks и специальные vanilla recipe handlers.
  Reference: `Minecraft/decompiled_src/sources/
  net/minecraft/world/level/block/CrafterBlock.java` и
  `.../block/entity/CrafterBlockEntity.java`.
- **Sculk event pipeline**: global emission, listener routing, calibrated
  frequency/direction filtering, sensor cooldown/phase, shrieker warning и
  Warden gates уже работают; P0 остаются end-to-end fixtures для каждого
  source event, точный SpawnUtil placement и particle/sound ordering.
- **Dispenser parity**: основная behavior-таблица, bonemeal, armor/equipment,
  shears, shulker, glass/XP bottles и fish/axolotl/tadpole buckets уже
  подключены. Mob-bucket variant/component NBT теперь восстанавливается перед
  spawn, включая стандартные mob flags; осталась certification каждого
  `DispenseItemBehavior` из Mojang и редкие entity-specific state fields.
- **Chunk persistence**: unknown root NBT, block entities, scheduled ticks,
  blending data and `InhabitedTime` round-trip through the Anvil chunk model;
  entity-region root metadata and opaque block-entity fields are now retained,
  and unknown block-entity IDs are put back into pending chunk NBT instead of
  being deleted. Остались реальные vanilla `.mca` fixtures, crash-recovery
  sector tests и live unload/reload tests across every format backend.
- **Respawn/dimension**: cross-dimension bed/anchor validation now resolves and
  loads the stored destination world; stale non-Nether anchor points are now
  rejected without consuming a charge. Safe world-spawn fallback and portal
  passenger semantics still need differential coverage.
- **Inventory safety**: stale menu slot packets now resync instead of indexing
  past the current handler; quick-craft drag distribution now follows the
  vanilla remainder and max-stack rules. Full atomic click transaction,
  bundle and disconnect cleanup coverage remain.
- **Natural spawning**: configured world spawn, active-chunk gating and
  dangerous spawn volumes are enforced; bat checks, any-light/surface monster
  variants, powder-snow-aware stray checks, surface-slime moon-phase rules,
  shared animal/surface
  water predicates, tagged animal support, polar-bear alternate biomes,
  turtle sand bounds, lush-cave tropical-fish, glow-squid and ocelot rules now
  match vanilla, while biome/structure spawn tables and remaining subtype
  rules remain. `PersistenceRequired` is now persisted on entities and
  excluded from natural-spawn cap accounting; cap removals are saturating so a
  persistence transition cannot underflow the counter. Despawn-distance checks
  now run before mob AI, protecting persistent/named/leashed mobs and excluding
  spectators; real multi-player fixtures and subtype-specific persistence
  overrides remain.
- **Entity tracking**: chunk delivery barrier, boundary replay, per-player
  paired-ID state, unload cleanup, overflow-safe absolute teleports, generic
  effect/status updates and bed dismount lifecycle уже работают; position,
  rotation, head-yaw, velocity and metadata deltas now require paired IDs;
  остаются полноценные ACK semantics, attribute/equipment dirty snapshots и
  subtype-specific minecart/projectile interpolation.
- **Mob effects**: repeated applications now use the vanilla stronger/longer
  replacement rule and missing removals are no-ops. The hidden-effect chain
  (weaker effects retained behind a stronger one and restored after expiry) is
  now persisted and promoted by the runtime; effect immunity tags and full
  differential packet fixtures remain.
- **Lighting**: signed Y bounds и Java `CLightUpdate` emission уже подключены;
  Java masks/arrays теперь используют общий vanilla-сериализатор с padding
  ниже/выше мира, variable-length bitsets и фильтрацией реально изменённых
  секций; остаются Bedrock subchunks и real-client fixtures.

## P1 — vanilla-visible behavior

- Daylight detector sky visibility, weather attenuation and the vanilla sun-angle
  easing are implemented; keep a real-client/weather fixture for packet-level
  verification.
- Mushroom placement now enforces the vanilla raw-light `< 13` and solid-support
  rule, including neighbor updates; a world fixture is still needed for
  generated light propagation.
- Довести command-block/spawner minecart до клиентской и дифференциальной
  совместимости: command/spawner state, activator cooldown and NBT are now
  implemented, so `MinecartKind::Other` no longer hides these subtypes. Остались
  display block state/entity events, Bedrock editor/UI and exact spawn
  placement/particle behavior. Mob-spawner delay,
  player-range gate, nearby-entity cap, spawn-volume check и vanilla NBT
  parameter clamping уже исправлены; `SpawnPotentials` теперь выбираются по
  vanilla-весу и сохраняются вместе с полным entity compound; остаются display entity,
  subtype spawn predicates и client spin/particle event fixtures.
- Сверить cross-dimension respawn с bed/anchor dimension, chunk loading и world
  spawn fallback.
- Melee and creeper LOS are checked before attacks; spider/cave-spider climbing
  now updates before movement and publishes metadata, and cave-spider poison
  matches Normal/Hard durations. Remaining work is the target predicate audit
  for every mob and wall-occlusion integration tests.
- Для item equipment остаются dispenser edge cases, общий equipment tick,
  armor/tool durability, binding curse и mob pickup persistence; player-menu
  armor/off-hand click и shift-click теперь публикуют итоговый
  `onEquipStack`-эквивалент через authoritative menu slot, а shield blocking
  корректно повреждает активный щит и синхронизирует break/non-break state.
- Name tags now reject non-living/dead/non-serializable targets, set
  `PersistenceRequired` for mobs and consume only for a valid custom-name
  application; a dedicated despawn/save fixture is still pending.
- Campfire cooking and projectile ignition are implemented; для jukebox осталось проверить
  game-event/comparator edge cases (окончание песни теперь останавливает звук и
  уведомляет соседей, не удаляя пластинку).
- Bell raid hearing and respawn-anchor edge cases remain. Conduit frame refresh,
  conduit power, wet-player effects, hostile selection/attack, ambient timing
  and NBT target persistence are implemented; remaining conduit work is an
  exact `Enemy`-marker predicate (the current runtime exposes
  `MobCategory::MONSTER`), client rotation/particle parity and a live water-frame
  fixture.
- Sponge BFS now follows vanilla depth/count limits and removes water, bubble
  columns and kelp/seagrass variants; remaining work is real-world
  waterlogged/bucket-pickup fixture and drop-table differential coverage.
- Проверить coral/tree exact `shuffledCopy(random)` direction order against
  fixed-seed structure snapshots; the Fisher–Yates order and coral claw
  segment draw are now implemented, but a full generated-feature fixture is
  still required for bit-exact parity.
- Проверить оставшиеся gamerule edge cases: динамическую смену
  `reduced_debug_info` и полноценную vanilla-проверку `limited_crafting`.
  Серверное хранилище изученных рецептов уже добавлено: состояние живёт в
  `Player::recipe_book`, сохраняется в `recipeBook.recipes` и изменяется через
  `/recipe give|take`. Статические сгенерированные рецепты уже получают
  стабильные namespaced ID; для полной совместимости остаются единая проверка
  limited-crafting теперь повторно проверяется на result-slot commit и
  `PlaceRecipe` packet с canonical key/window validation; registry validation
  при player-load и `/reload` теперь удаляет неизвестные keys/highlights; остаются
  единая проверка для всех специальных menu types и миграция legacy payloads
  без собственного ключа через versioned recipe registry. `immediate_respawn`
  теперь проходит через смерть и respawn, `random_tick_speed` управляет числом
  выборок на section, а `advance_weather` корректно замораживает погодный цикл.
  `block_drops`, `fall_damage` и `freeze_damage` применяются в общих runtime-точках.
- Target block projectile routing, per-face power calculation, arrow/trident
  20-tick versus other-projectile 8-tick reset, weak/strong output, scheduled
  reset and `TARGET_HIT` statistics are implemented for all owner-bearing
  projectile entities. Остались criterion trigger fixtures and real-client
  redstone/game-event tests.
- Arrow base damage is now mutable through the parameter consumed by the hit
  damage formula; fishing catches are delivered to inventory or dropped rather
  than deleted, and retrieval applies vanilla rod durability in the active
  hand. Remaining projectile TODOs are firework particle/color payloads, exact
  fishing loot-table rolls/open-water checks and piercing-entity bookkeeping.

## P1 — protocol/edition coverage

- Сгенерировать и проверять WIT после изменения protocol derive; schema drift
  сейчас может быть скрыт до plugin build.
- Довести Bedrock simulation distance, inventory mappings, forms и entity
  tracking до per-player parity.
- Завершить NetherNet discovery/signaling only as a coherent transport change;
  не смешивать старый HTTP signaling с UDP discovery.
- Проверить 1.21.x/26.x packet gates отдельно от текущей 26.2 target.

## P2 — качество кода

- Сократить дублирование async piston/neighbor update code.
- Удалить magic numbers в inventory/window properties и scheduler limits.
- Убрать `TODO: ugly` adapters из mob spawner, infested blocks and command args.
- Добавить unit tests для `pumpkin-util` random implementations and noise helpers.
- Добавить inventory NBT load tests, bundle/max-stack tests beyond quick-craft,
  and disconnect cleanup.
- Player autosaves now retain unknown root NBT fields, and weather/game-rule
  data files merge recognized values into the existing extensible document;
  remaining persistence work is nested component preservation plus real
  crash-recovery fixtures.
- Number-key inventory swaps now exchange both non-empty stacks and keep an
  oversized hotbar remainder instead of duplicating it; a client transaction
  fixture and full component/max-stack matrix are still required.
- Укрепить SNBT escaping и resource-location autocomplete.
- Sign text click events now decode vanilla camel-case JSON and execute root
  `run_command` lines with the player's command permissions; nested component
  events and full client criterion coverage remain.
- Вынести codegen TODOs (PHF maps, enum conversions, recipe components,
  advancement rewards/functions) в отдельные generated-data tasks.
- `minecraft:use_effects` data/codec and movement consumers are implemented;
  block placement and boat placement now pass the authoritative item stack to
  the vibration dispatcher, so `interact_vibrations=false` suppresses only
  that item's event. Remaining component work is the broader generated
  component inventory and the event-source audit for dispenser/projectile
  collision events and consumable interactions whose source stack is not yet
  threaded through their authoritative runtime path.

## Состояние generated data

Большая часть `pumpkin-data/src/generated` (≈1.4M строк) является артефактом.
Если там найдена ошибка, исправляйте input asset или builder в
`tools/pumpkin-codegen`, затем запускайте генератор и проверяйте diff. Ручной
патч generated-файла будет потерян при следующем обновлении версии.

## Definition of done для parity-задачи

- есть ссылка на vanilla class/method и описание отличия;
- state transition покрыт тестом;
- есть persistence/client/edition test на границе;
- no panic/unwrap on malformed, unloaded or despawned state;
- fixed-seed/random sequence проверена для worldgen;
- docs/parity row and this backlog updated;
- `cargo fmt`, `cargo check`, targeted tests и `git diff --check` проходят.
