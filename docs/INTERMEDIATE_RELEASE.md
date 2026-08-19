# Промежуточная версия Pumpkin: checkpoint vanilla-parity

Этот файл описывает именно состояние репозитория на промежуточном checkpoint,
который можно самостоятельно отправить в GitHub. Это не заявление о полной
совместимости с Mojang: ниже отдельно перечислены реализованные подсистемы,
оставшиеся расхождения и фактически выполненные проверки.

Checkpoint baseline commit: `d35ccb2ca` (the immutable restore point created
before post-checkpoint fixes). The published branch additionally contains the
follow-up fixes through `1c914a4d7`; the local tag remains the immutable
baseline `intermediate-parity-checkpoint`.
Checkpoint branch: `codex/intermediate-parity-checkpoint`

## Что входит в checkpoint

- Все текущие изменения Pumpkin в исходном коде, generated data, protocol,
  worldgen, persistence, recipes, redstone, entities, commands и документации.
- Новые файлы из `docs/`, `parity/` и `tools/parity_inventory.py`.
- Полный машиночитаемый реестр контрактов `parity/manifest.toml` и отчётный
  скрипт `tools/parity_inventory.py`.
- Локальный decompile `Minecraft/` **намеренно не входит** в checkpoint: это
  пользовательский reference-каталог, а не runtime-исходники сервера.

## Уже сделано относительно vanilla 26.2

Это рабочие реализации, для которых в текущем дереве есть код и локальные
регрессионные проверки. Статус `mostly` в ledger означает, что не хватает
реального Java/Bedrock differential-fixture, а не что базовый путь отсутствует.

### Gameplay и redstone

- Поршни, moving piston/block entity, honey/slime passenger movement и
  same-tick retraction; neighbor updates ограничены итеративным budget.
- Redstone wire (включая weak/strong power guards и rotate/mirror), repeater,
  comparator, pressure plates, weighted plates, buttons, daylight detector,
  target block, rails, hopper/dropper/dispenser notification paths.
- Sculk Sensor и Calibrated Sculk Sensor: pending vibration с travel delay,
  nearest/highest-frequency selector, calibrated back-signal filter, wool
  damping, resonance и persistence.
- Sculk Shrieker: player/source filtering, shared Warden warning cooldown,
  darkness/reply effects, 20-attempt spawn search и game-event chain.
- Dispenser: projectiles, buckets/entity buckets, equipment, shears/beehives,
  shulker boxes, wither skull, TNT gamerule/fallback, brush/scute и optional
  order rules. Оставшиеся edge cases перечислены ниже.
- Crafter: recipe matching, disabled slots, atomic input snapshot, remainders,
  output insertion/drop, six-tick crafting state и comparator refresh.

### Minecart, structures и worldgen

- Основные вагонетки (rideable, chest, furnace, hopper, TNT, command/spawner
  paths), rail movement, fall-off-rail correction, NBT/load and minecart loot
  persistence.
- Полный порт ocean monument и существенно расширенный mineshaft/structure
  generation, template transforms и structure entity hydration.
- ProtoChunk resume, scheduled block/fluid tick ownership, deterministic loot
  and worldgen RNG streams, sculk spreader/charge cursor feature code.
- Shared player spawn finder: `respawn_radius`, border, heightmap/fluid/support
  checks, complete player collision and bounded fallback height fixup для Java,
  Bedrock login и fallback respawn.

### Recipes, datapacks и persistence

- Server-side recipe book: canonical namespaced keys, generated+dynamic recipe
  registry, highlights/categories, give/take, reconnect state and limited
  crafting validation.
- Shaped/shapeless/cooking recipe matching, dynamic result components, crafter
  remainders and `/recipe`/PlaceRecipe boundary checks.
- Transactional directory/ZIP datapack load and `/reload`, pack-priority
  recipes/tags/functions, optional tag entries, cycle detection, function tags,
  `/function`, `/schedule`, `minecraft:load`/`minecraft:tick` and lossless
  scheduled callback NBT.
- Lossless unknown-root NBT for player/level/chunk/entity-region/block entity,
  vanilla palette/biome decoding, ProtoChunk persistence and custom bossbars.
- Deterministic loot seed propagation, supported deferred chest/storage-minecart
  loot subset, count saturation and LocationCheck world resolution.

### Entities, items и protocol

- Minecart/entity world transfer and unload cleanup, synchronous center-chunk
  delivery barriers, experience orb lifecycle,
  firework component/lifetime persistence and exact placement raycast for boats.
- Mob bucket variants (fish/axolotl/tadpole/sulfur cube), dispenser equipment,
  beehive occupants, item component codecs, use-cooldown and hand validation.
- Cross-dimension respawn, bed/anchor candidate order, swimming/flying/mining
  behavior, riptide pose, fall/fire/freeze/armor/XP fixes, selected AI LOS and
  target predicates.
- Java/Bedrock packet fixes, main/off-hand mapping, command/recipe/light/entity
  metadata codecs, Java `ClientTickEnd` known-movement reset, configured
  Bedrock member/operator permission, NetherNet discovery and malformed input
  guards.

### Documentation and audit tooling

- `docs/FULL_IMPLEMENTATION_PLAN.md` — acceptance specification and dependency
  gates.
- `docs/IMPLEMENTATION_STATUS.md` — implementation map and explicit release
  blockers.
- `docs/VANILLA_PARITY.md` — subsystem-by-subsystem comparison with Mojang
  classes and remaining work.
- `parity/manifest.toml` — 151 tracked vanilla contracts with sources,
  observable behavior, tests and dependencies.

## Что ещё НЕ готово для заявления «полная совместимость»

Эти пункты намеренно остаются отмеченными как `mostly`/pending. Их наличие
нельзя скрывать при публикации checkpoint.

1. **Differential certification.** Нет завершённого `tools/parity-runner` с
   packet bot для Java 26.2/26.1, RCON/tick barrier, normalized packet/entity/
   block/NBT/tick traces и автоматическим сравнением с запущенным vanilla.
2. **Реальный клиент и soak.** Не выполнены два чистых прогона полного matrix
   на Java и Bedrock, save/reload/restart, malformed packet fuzz, долгий soak,
   crash-recovery и performance/security gates.
3. **Entity tracking.** Нужен полный per-player `ChunkMap.TrackedEntity` delta
   tracker: chunk-send ACK barriers для всех типов, metadata/attribute/equipment
   dirty snapshots, unload/reload ghost removal и minecart live-client fixture.
4. **Chunk persistence.** Нужны реальные `.mca` fixtures для всех block entities,
   ticks, entities, unknown components и crash-safe atomic save/unload; текущие
   round-trip tests не заменяют такую certification.
5. **Datapack typed consumers.** Raw loot/predicate/advancement/structure
   resources загружаются losslessly, но полные typed условия/functions/components
   loot, predicate evaluation, advancement triggers/rewards и structure consumers
   ещё не покрыты runtime-путями.
6. **Dispenser completeness.** Остаются точные vanilla cases для bonemeal,
   armor/equip edge cases, shears/shulker variants, glass/XP bottle details,
   fish/axolotl/tadpole custom data и все редкие fallback/order combinations.
7. **Sculk/Warden completeness.** Нужна проверка каждого server-originated
   vibration source и exact `VibrationSystem` filters, полный Warden spawn rule
   (difficulty/structure/biome/obstruction) и real block-entity fixture.
8. **Spawning и AI.** Остаются structure-specific Nether fortress rules,
   exact `SpawnPlacements`/spawn boxes, biome holder paths и отдельные subtype
   goals/brains (phantom, witch, drowned, spider jumping, dragon handling и
   другие TODO из inventory).
9. **Lighting/fluids/weather.** Java changed-section и Bedrock subchunk packet
   fixtures, cross-dimension light reload, full lava-water interaction,
   waterlogged edge cases и precipitation/weather-cycle parity ещё не закрыты.
10. **Worldgen/structures.** Нужны fixed-seed golden snapshots для каждого
    dimension/feature/structure и точная RNG-consumption проверка; sapling
    configured-tree/mega-tree generation и некоторые decorators остаются
    неполными.
11. **Commands/stats/advancements/POI.** Полная command return-value matrix,
    advancement listener coverage, villager POI claim/release across reload и
    все gamerule/stat edge cases ещё требуют реализации и fixtures.
12. **Generated/versioned data.** Нужны clean-checkout codegen drift checks и
    protocol registry certification для каждой поддерживаемой версии, а также
    полноценная Bedrock forms/inventory/editor parity.

## Проверки, фактически выполненные перед checkpoint

Успешно:

- `cargo fmt --all -- --check`
- `git diff --check`
- `python3 tools/parity_inventory.py` — 151 contracts, `manifest_errors = []`
- `CARGO_INCREMENTAL=0 cargo check -p pumpkin --lib`
- `CARGO_INCREMENTAL=0 cargo test -p pumpkin --lib --no-fail-fast` — **388/388**
- `CARGO_INCREMENTAL=0 cargo test -p pumpkin-world --lib --no-fail-fast` —
  **226/226**
- targeted spawn-finder regression — **1/1**

До этого checkpoint также проходили targeted suites для protocol, inventory,
recipe/crafter, sculk, minecart, persistence и worldgen. Полный
`cargo check --workspace`/`cargo test --workspace --no-fail-fast` именно после
последней правки не следует считать доказанными, пока команда не завершится
снова на машине публикации; ранее полный workspace test упирался в нехватку
места на диске, а не в тестовую ошибку.

## Как опубликовать checkpoint

После создания commit текущая ветка будет называться
`codex/intermediate-parity-checkpoint`. Для публикации в свой fork:

```bash
git push -u fork codex/intermediate-parity-checkpoint
```

Перед push проверьте `git status --short`: каталог `Minecraft/` должен остаться
неотслеживаемым и не должен попадать в commit. Следующую работу можно вести
поверх этого commit; он является точкой восстановления, а не финальным
релизом.
