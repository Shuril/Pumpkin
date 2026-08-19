# Полный план доведения Pumpkin до vanilla-parity

Этот документ — исполняемая спецификация. Он рассчитан на исполнителя, который
не знает архитектуру Pumpkin и не должен принимать решения «по ощущениям».
Каждый пункт должен завершаться кодом, тестом, ссылкой на vanilla-reference и
обновлением parity-ledger. Простое наличие типа, регистрации блока или пакета не
считается реализацией поведения.

## 0. Цель и точное определение «готово»

### 0.1. Основная цель

Основная эталонная версия — Java dedicated server **26.2**, находящийся в
`Minecraft/decompiled_src/sources`. Дополнительная совместимость 26.1
проверяется по `Minecraft/26.1/decompiled_src/sources` через protocol/data
version gates. Нельзя одновременно переносить семантику двух версий в один
неверсионированный алгоритм: различия оформляются как версия данных, пакета,
registry snapshot или явно выбранный compatibility branch.

Pumpkin считается полностью готовым относительно Java 26.2 только если:

1. Java-клиент 26.2 проходит login/config/play без пропущенных обязательных
   пакетов, рассинхронизации registry и необъяснимых disconnect.
2. Каждая серверно-значимая vanilla-система имеет запись в parity-ledger:
   `complete`, `not_applicable` с объяснением или `extension`. Статусы `mostly`,
   `partial`, `missing`, `unknown` к релизу запрещены.
3. Одинаковая последовательность входных действий при одинаковом seed и
   начальном NBT даёт эквивалентные наблюдаемые состояния: блоки, сущности,
   инвентари, redstone power, scheduled ticks, loot, stats, advancements,
   packets и сохранённый мир. Время wall-clock и внутренние Rust/Java структуры
   сравнивать не нужно.
4. Vanilla Anvil/player/level NBT читается без потери неизвестных полей и после
   round-trip снова читается vanilla-сервером.
5. Нет panic, deadlock, бесконечной очереди или потери данных на malformed
   packet, unloaded chunk, despawn race, shutdown и нехватке необязательных
   данных. Ошибки конфигурации и повреждённые файлы возвращают диагностируемый
   `Result`, а не обрывают процесс.
6. Java gameplay parity не зависит от Bedrock-кода. Bedrock использует тот же
   authoritative gameplay state через отдельный adapter и имеет собственную
   сертификационную матрицу.
7. Все generated registry/block/item/packet/recipe данные воспроизводятся одним
   запуском codegen; ручных исправлений в `generated/` нет.
8. Полный test matrix из раздела 18 проходит на чистом checkout два раза подряд,
   включая save/reload и fixed-seed сравнения.

### 0.2. Что не является доказательством готовности

- «Сервер запускается».
- «Клиент видит блок».
- «Есть Rust-файл с нужным именем».
- Успешный unit test чистой функции без world/NBT/packet boundary.
- Совпадение визуального результата при одном ручном сценарии.
- Перенос PR без повторного сравнения с vanilla 26.2.
- Отсутствие слова `TODO`: алгоритм может быть неполным без маркера.

### 0.3. Текущая измеримая база

- В локальном decompile 26.2: 4 839 Java-файлов под `net/minecraft`.
- В runtime crates найдено около 649 явных `TODO`, `FIXME`, `todo!`,
  `unimplemented!` и `panic!` вне generated-кода. Не каждый `panic!` является
  дефектом: тестовые invariant assertions и доказуемо невозможные enum branches
  классифицируются отдельно.
- Известные P0 gaps: recipes/crafter, vibration/sculk, dispenser registry,
  chunk persistence, per-player entity tracking, lighting.
- Вагонетки основных типов и ocean monument уже имеют полноценную основу; это
  не отменяет differential tests и оставшиеся command/spawner/collision cases.

## 1. Правила работы для исполняющей LLM

Эти правила применять к каждой задаче без исключений.

### 1.1. Перед изменением

1. Зафиксировать один узкий task ID из этого документа, например `REC-04`.
2. Открыть все указанные vanilla-классы и полностью прочитать методы, связанные
   с задачей. Если decompiler пропустил тело, использовать bytecode dump или
   другой decompiler; не додумывать алгоритм.
3. Найти все Pumpkin call sites через `rg`, включая Java, Bedrock, persistence,
   commands и plugins.
4. Записать таблицу переходов состояния:
   `preconditions → mutation → scheduled work → effects/packets → persistence`.
5. Проверить dirty worktree. Не reset/stash/overwrite чужие изменения. Новый
   generated output получать только через codegen.
6. Сначала написать failing test или fixture, воспроизводящий отличие.

### 1.2. Во время изменения

1. Gameplay decision должен жить в общем runtime, а Java/Bedrock adapters только
   кодируют результат.
2. Не держать async lock во время packet send, chunk load, plugin callback или
   долгого обхода мира. Снять snapshot, отпустить lock, выполнить работу,
   повторно проверить generation/version перед commit.
3. Порядок scheduled tick, neighbor update, inventory mutation и RNG consumption
   является частью контракта. Нельзя заменять последовательный vanilla-loop на
   `JoinSet` или unordered map iteration.
4. Любая мутация persistent state сопровождается dirty marking.
5. Любой ID хранится как namespaced `Identifier/ResourceLocation`, а не output
   item, protocol index или display ID.
6. Любое обращение к entity/chunk/block entity терпит despawn/unload race.
7. Fallback должен соответствовать vanilla: например dispenser при неуспешном
   специальном поведении обычно выбрасывает предмет, а не уничтожает его.

### 1.3. После изменения

1. Добавить unit test алгоритма.
2. Добавить boundary test: world tick, packet round-trip, NBT round-trip или
   client scenario — в зависимости от задачи.
3. Для RNG/worldgen добавить fixed-seed golden test и проверить число/порядок
   вызовов RNG, не только итоговый блок.
4. Выполнить минимальные команды:

   ```bash
   cargo fmt --all -- --check
   CARGO_INCREMENTAL=0 cargo check -p pumpkin --lib
   CARGO_INCREMENTAL=0 cargo test -p pumpkin --lib
   CARGO_INCREMENTAL=0 cargo test -p pumpkin-world --lib
   git diff --check
   ```

5. Для затронутого crate выполнить его отдельные tests/clippy. Для protocol —
   round-trip и malformed corpus; для persistence — реальные `.mca` fixtures.
6. Обновить parity-ledger, этот план и `docs/VANILLA_PARITY.md`.
7. В отчёте перечислить незакрытые edge cases. Запрещено писать «полностью
   готово», пока task acceptance criteria не выполнены все.

## 2. Система учёта, без которой полная parity недоказуема

### PAR-01. Машиночитаемый parity-ledger

Создать `parity/manifest.toml` или каталог `parity/*.yaml`. Одна запись описывает
не файл, а vanilla contract:

```yaml
id: recipe.server_book
vanilla_version: "26.2"
vanilla_sources:
  - net/minecraft/stats/ServerRecipeBook.java
  - net/minecraft/server/level/ServerPlayer.java
pumpkin_sources:
  - crates/pumpkin/src/entity/player.rs
  - crates/pumpkin/src/server/recipe.rs
status: partial
observable_contracts:
  - known/highlight/settings survive NBT round-trip
  - add/remove packets contain every display entry
tests:
  - recipe_book_real_player_nbt_round_trip
  - recipe_book_command_and_reconnect
blocked_by: [registry.recipe_keys]
notes: "dynamic crafting currently has no independent key"
```

Обязательные поля: `id`, версия, vanilla sources/methods, Pumpkin sources,
status, observable contracts, test names, dependencies, last verified commit.
CI отклоняет неизвестный status, `complete` без теста и ссылку на несуществующий
файл/метод.

### PAR-02. Автоматический inventory

Написать tool, который:

1. индексирует 4 839 Java-файлов, классы и методы;
2. индексирует Rust modules, block/entity/item registries, packet mappings,
   commands и generated IDs;
3. формирует отчёты `unmapped vanilla contracts`, `orphan Pumpkin types`,
   `TODO/FIXME`, `panic sites`, `generated drift`;
4. не считает getter/DTO/client-only renderer отдельной server parity задачей,
   но требует явного `not_applicable` с категорией;
5. сохраняет stable IDs, чтобы rename не сбрасывал историю.

Начальная классификация Java:

- `server_required`: gameplay, network server packets, data, world, entity,
  block, inventory, command, persistence;
- `shared_data`: codec, registry, NBT, math, random;
- `client_only`: render, sound engine, GUI implementation — обычно N/A, но их
  wire/data contracts могут быть server-required;
- `tooling_only`: data generator/debug tool;
- `extension`: Bedrock/plugin/Pump formats, у которых нет vanilla-аналога.

### PAR-03. Differential harness

Создать `tools/parity-runner`:

1. запускает vanilla 26.2 и Pumpkin в отдельных временных каталогах с одним
   `server.properties`, seed, datapacks и стартовыми player/chunk fixtures;
2. управляет обоими через packet bot/RCON и deterministic tick barrier;
3. выполняет сценарий из YAML: connect, place, use, move, wait ticks, command,
   disconnect, restart;
4. собирает normalized packet trace, block/entity snapshots, inventories,
   scheduled ticks, scoreboard/stats/advancements и NBT;
5. нормализует случайные session entity IDs, UUID только когда они специально
   не фиксированы, timestamps, compression framing и порядок независимых
   packets;
6. сравнивает семантику, но не скрывает порядок dependent packets/ticks;
7. при расхождении сохраняет минимальный replay bundle.

Нужны adapters:

- Java packet bot 26.2 и 26.1;
- RCON command driver;
- NBT canonicalizer: сортирует compound keys, но сохраняет list order;
- world snapshot: blocks + states + block entity NBT в bounding box;
- entity snapshot: type, position, velocity, metadata, passengers, NBT;
- tick trace: game time, priority, sub-tick order, position/type;
- RNG trace для unit-level comparison.

### PAR-04. Vanilla GameTest importer

Проиндексировать `net/minecraft/gametest` и test structures/resources. Для каждого
server-relevant test:

1. либо портировать scenario в Pumpkin integration test;
2. либо запускать исходный сценарий через parity-runner;
3. либо отметить N/A с объяснением.

Не изменять expected result, чтобы подогнать Pumpkin. Если vanilla test зависит
от test-only API, воспроизвести только публично наблюдаемое поведение.

## 3. Порядок реализации и dependency gates

Работать в следующем порядке. Переход к следующему milestone разрешён, когда
gate предыдущего зелёный; независимые task cards внутри milestone можно вести
параллельно в разных файлах.

```text
M0 version/baseline
  → M1 parity tools + registries/codegen
    → M2 persistence + chunk lifecycle
      → M3 tick/neighbor/lighting/game-event infrastructure
        → M4 inventory/recipes/crafter/dispenser
        → M5 entities/tracking/movement/AI
          → M6 blocks/items/redstone/fluids
            → M7 worldgen/structures/POI/spawning
              → M8 commands/advancements/loot/gamerules/datapacks
                → M9 Java protocol certification
                  → M10 Bedrock/plugin/extensions
                    → M11 soak/performance/security/release
```

Почему порядок такой:

- Нельзя тестировать crafter без canonical recipe registry.
- Нельзя тестировать sculk без общего game-event dispatcher.
- Нельзя сертифицировать вагонетки без per-player tracking и chunk lifecycle.
- Нельзя доверять worldgen golden snapshots, пока RNG, palettes и persistence
  теряют данные.
- Нельзя заявлять protocol parity, если authoritative gameplay выдаёт неверное
  состояние.

## 4. M0 — версия, baseline и воспроизводимая сборка

### BAS-01. Зафиксировать target versions

Создать единый `VersionProfile`:

- marketing/server version: 26.2;
- Java protocol version;
- data version;
- resource/pack version;
- Bedrock protocol versions;
- registry snapshot hash;
- decompile/resources SHA-256.

Все packet gates, generated data, saved `DataVersion` и status response читают
его, а не дублируют числа. CI проверяет, что `Minecraft/decompiled_src` содержит
`server-26.2.jar` markers. 26.1 оформляется вторым profile.

### BAS-02. Воспроизводимый codegen

На чистом checkout два последовательных запуска codegen должны давать пустой
diff. Builder обязан генерировать:

- blocks/states/properties/shapes/opacity/luminance;
- items/components/max stack/durability/equipment;
- entity types/metadata/tracking range/update interval;
- registries/tags/game events/damage types/biomes/dimensions;
- recipes с **recipe key**, serializers, placement/display info;
- loot/advancements/worldgen/structures/templates/processors;
- Java/Bedrock packet IDs и version maps;
- WIT schema после protocol/API изменения.

Исправление generated output делается в input parser/builder, затем
перегенерируется весь связанный набор.

### BAS-03. Clean baseline report

Сохранить:

- `cargo test --workspace` результат;
- compile warnings;
- TODO/panic inventory;
- список open parity tasks;
- 20-минутный idle/players/chunk-generation profile;
- размер чистого мира после generate/save/reload.

Это не acceptance, а база для обнаружения регрессий. Не включать `Minecraft/` в
git и не удалять пользовательские изменения при обновлении upstream.

## 5. M1 — identifiers, registries, resources и datapacks

### REG-01. Один тип идентификатора

Ввести/доделать строгий `Identifier` (`namespace:path`):

- default namespace `minecraft` применяется только при разборе пользовательской
  строки; во внутреннем состоянии namespace всегда явный;
- валидация разрешённых символов, запрет пустого namespace/path;
- canonical `Display`, serde, NBT и packet codecs;
- отдельные newtypes `RecipeKey`, `RegistryKey<T>`, `TagKey<T>` поверх
  `Identifier`, чтобы item ID нельзя было случайно передать как recipe ID;
- protocol numeric IDs являются свойством конкретного registry snapshot, не
  persistent identity.

Заменить output-item fallback в dynamic crafting: plugin registration уже
получает `_id`, его нужно сохранить в `OwnedCraftingRecipe`/новом runtime recipe
type. Duplicate key отклоняется с ошибкой; два разных рецепта с одним result
разрешены.

### REG-02. Versioned registry snapshots

`RegistryManager` хранит immutable snapshot (`Arc`) с generation number.
Datapack reload строит новый snapshot отдельно, валидирует ссылки/теги/циклы и
атомарно публикует его. Открытые menu/world tasks держат generation и при commit
проверяют, не сменился ли registry.

Каждый registry предоставляет:

- `key → value`;
- `key → protocol id` для версии клиента;
- `protocol id → key` с bounds check;
- tag membership;
- lifecycle/stability;
- codec для JSON/NBT/network;
- deterministic iteration в vanilla registry order.

### REG-03. Resource/datapack loader

Реализовать vanilla pack stack:

1. builtin resources;
2. world datapacks в `datapacks/`;
3. `pack.mcmeta`, supported formats/features/overlays;
4. namespaces и resource shadowing по pack priority;
5. tags с `replace`, required/optional entries и tag references;
6. JSON codecs для recipes, loot, advancements, functions, predicates,
   structures/worldgen registries;
7. reload prepare/apply barrier;
8. `/reload` feedback и сохранение enabled/disabled packs в `level.dat`;
9. rollback: не публиковать половину нового состояния при одной ошибке.

Tests: два packs с override, tag replace/optional, broken reference rollback,
reload while players craft/generate chunks, vanilla datapack fixture.

### REG-04. Components and data-driven behavior

Закрыть unknown component branch в `pumpkin-protocol/src/codec/data_component.rs`.
Для каждого 26.2 component:

- generated numeric ID/version gate;
- owned runtime representation;
- Java and Bedrock codecs;
- equality/hash semantics для stacking;
- NBT/JSON codec;
- tooltip/client-only fields могут быть pass-through, но не должны теряться;
- gameplay hooks: consumable, food, damage, equippable, container, potion,
  fireworks, instrument, repairable, enchantments.

Unknown future components в сохранённом NBT сохраняются opaque, если формат это
позволяет; unknown network ID отклоняется как protocol mismatch.

## 6. M2 — полное persistence и chunk lifecycle

### PST-01. Canonical in-memory chunk model

`ChunkData/ProtoChunk/LevelChunk` должны иметь явные поля для всего vanilla
`SerializableChunkData`:

- `DataVersion`, x/z, status, last update, inhabited time;
- sections с block-state/biome palettes и packed data;
- block/sky light arrays и `isLightOn`;
- heightmaps;
- block ticks и fluid ticks;
- block entities;
- entities там, где формат/версия хранит их внутри chunk;
- structures starts/references;
- carving mask, post-processing;
- blending data, below-zero retrogen, upgrade data;
- unknown root tags и unknown nested tags, которые Pumpkin не понимает.

Разделить known fields и `UnknownNbt` map. При чтении known key извлекается из
opaque map; при записи runtime value побеждает старый known key, остальные opaque
tags возвращаются byte-semantically. Нельзя молча записывать пустой list вместо
неподдержанной структуры.

### PST-02. Anvil read algorithm

Для chunk `(expected_x, expected_z)`:

1. Прочитать region header, проверить sector bounds/length/compression type.
2. Ограничить NBT allocation/depth через accounter.
3. Проверить x/z; mismatch логировать и переместить согласно vanilla policy, не
   загружать в неправильную координату.
4. Применить DataFix/migration или явно отклонить неподдерживаемую старую версию
   без перезаписи файла.
5. Снять root unknown tags.
6. Декодировать section Y как signed; игнорировать/сохранять sections вне
   текущего dimension range по принятой migration policy.
7. Block palette entry: `{Name, Properties}`; неизвестный block/state не
   превращать безвозвратно в air — сохранить raw entry и диагностировать.
8. Biome palette — resource keys; packed bit width соответствует vanilla
   `PalettedContainer` rules, включая single-value no-data case.
9. Декодировать lights строго по 2048 nibble bytes.
10. Отфильтровать saved ticks по chunk coordinates, сохранить priority и delay.
11. Block entity NBT оставить dormant до создания runtime entity; invalid type
    не должен удалять raw NBT.
12. Prime отсутствующие required heightmaps; существующие валидировать.

### PST-03. Anvil write algorithm

1. Перед snapshot сериализовать все live block entities обратно в chunk.
2. Freeze chunk generation/version; изменения после snapshot оставляют chunk
   dirty для следующего save.
3. Сериализовать sections/palettes/lights/heightmaps/ticks в canonical 26.2
   schema.
4. Merge unknown NBT без перезаписи runtime-owned keys.
5. Писать во временный/новый sector, fsync data, затем атомарно обновлять region
   header; старые sectors освобождать только после успешного header commit.
6. External oversized chunk file обрабатывать по vanilla region semantics.
7. Dirty flag снимать только для сохранённой generation.
8. На shutdown дождаться entity/block-entity/chunk/player save tasks и region
   flush; timeout сообщает незаписанные coordinates.

### PST-04. Scheduled ticks

Сохранённый tick содержит type key, absolute/relative trigger time, priority и
sub-tick order. При load:

- восстановить monotonic order без collision;
- deduplicate только по vanilla scheduler contract;
- unknown type сохранить opaque;
- не выполнять tick до FULL/ticking status;
- unload возвращает pending + inflight policy в chunk snapshot;
- block replacement проверяет, применим ли tick к текущему block/fluid.

Tests: repeater/observer/fluid save exactly за один tick до исполнения,
unload/reload, equal-time priority order, canceled tick, unknown type retention.

### PST-05. Block entities

Единый lifecycle:

```text
raw NBT in chunk
  → create only when block state/type compatible
  → loadAdditional
  → onLoad/listener registration
  → ticks and mutations + setChanged
  → write snapshot before chunk unload
  → preRemove/onChunkUnloaded exactly once
```

Каждый block entity получает NBT round-trip test с реальными item components.
Chest double pairing, hopper cooldown, furnace timers, crafter disabled slots,
sculk listener, piston moving state, signs/commands, spawner, beacon, jukebox и
shulker требуют boundary tests.

### PST-06. Player/level/entity persistence

- Player inventory, ender chest, selected slot, abilities, effects, attributes,
  stats, advancements, recipe book, spawn point/dimension, root vehicle,
  shoulder entities, cooldowns и XP.
- Recipe book хранит settings, `recipes`, `toBeDisplayed`.
- Level metadata сохраняет gamerules, border, weather, time, spawn, datapacks,
  wandering trader, dragon fight, custom boss events и unknown tags.
- Entity NBT покрывает base fields, exact subtype, passengers recursively,
  leash, equipment/components, brain/memory/variant и subtype data.
- UUID/entity ID не смешивать: runtime entity ID никогда не persistent.

### PST-07. Real fixtures and crash recovery

Добавить fixtures:

- fresh vanilla 26.2 overworld/nether/end chunks;
- chunks со всеми palette widths;
- каждый block entity;
- structures, POI, ticks, blending/retrogen;
- deliberately corrupted length/compression/NBT depth;
- server crash между data write и header commit.

Acceptance: vanilla → Pumpkin load/save → vanilla load сохраняет неизвестные
tags и наблюдаемое состояние; повреждение одного chunk не уничтожает region.

## 7. M3 — tick engine, neighbor updates и world mutation

### TIK-01. Один authoritative tick pipeline

Зафиксировать order из vanilla 26.2 и реализовать его явно в `Server/World`:

1. server queued tasks;
2. world border/time/weather;
3. chunk tickets/activation;
4. scheduled block ticks по `(trigger, priority, sub_order)`;
5. scheduled fluid ticks;
6. raids/block events;
7. chunk random ticks, precipitation, lightning;
8. block entity ticks;
9. entity tick/passengers;
10. natural spawning/despawn;
11. players, advancements/stats;
12. entity tracking/metadata and packet flush;
13. autosave.

Не распараллеливать зависимые scheduled block ticks. CPU worldgen/lighting может
работать параллельно над изолированным snapshot, но commit проходит deterministic
barrier.

### TIK-02. Scheduler

Scheduler использует min-ordered structure и monotonic `sub_tick_order`:

- `schedule` не теряет higher priority tick;
- `is_scheduled` видит queued и inflight;
- tick снимается в inflight непосредственно перед callback;
- callback может планировать тот же pos/type на будущий tick;
- completion удаляет inflight даже при error/cancel;
- per-tick cap откладывает остаток с сохранением порядка;
- chunk unload/save корректно пакует очередь.

### NBR-01. Iterative neighbor updater

Перенести контракт `CollectingNeighborUpdater`, не рекурсивные вызовы:

- fixed direction order: west, east, down, up, north, south;
- `stack` текущих multi-step updates;
- `added_this_layer` для updates, созданных внутри callback;
- новые updates вставляются в reverse, чтобы сохранить depth-first vanilla order;
- `count` ограничивается gamerule/config `maxChainedNeighborUpdates`;
- ровно один error при первом skipped position;
- отдельные operations: shape update, simple/full neighbor changed,
  multi-neighbor except direction;
- update flags, recursion limit, moved-by-piston и redstone orientation передаются
  без потери.

Псевдокод:

```text
add(update):
  running = count > 0
  count += 1
  if under_limit:
    if running: added_this_layer.push(update)
    else: stack.push(update)
  if !running:
    while stack or added:
      push added in reverse; clear added
      current = stack.top
      while added empty and current.run_next(world): continue
      if current finished: stack.pop
    finally clear all and reset count
```

Tests: `/fill` 4096+ blocks, long wire line, piston clock, observer chain,
self-scheduling neighbor, exact update trace against vanilla.

### MUT-01. Atomic world block mutation

Один `set_block_state` path отвечает за:

- world/chunk bounds и loaded status;
- old/new state equality;
- block entity remove/create ordering;
- heightmap/light updates;
- shape and neighbor updates по flags;
- comparator output updates;
- fluid scheduled tick для waterlogged;
- client block update;
- game event, drops/XP только через вызывающий behavior;
- chunk dirty mark.

Массовые операции используют transaction/batched notifications, но после commit
воспроизводят vanilla ordering. Plugin cancel происходит до необратимой мутации.

## 8. M3 — lighting

### LGT-01. Bounds and storage

- Dimension даёт `min_y`, `height`, min/max section; никаких `319`, `63`, `128`.
- Ниже min Y block/sky light возвращает 0; выше верхней sky section — 15 в
  skylight dimension, block light — 0; Nether/no-skylight — 0.
- Signed offset проверяется до cast в `usize`.
- Section nibble arrays имеют 4096 значений/2048 bytes и copy-on-write storage
  для updating/visible snapshots.
- Empty/non-empty section transitions создают/удаляют storage согласно vanilla.

### LGT-02. Block light algorithm

При изменении блока вычислить emission и opacity/shape. Использовать две очереди:

1. decrease: если сохранённый свет зависел от удалённого пути, обнулить и
   распространить decrease; независимые яркие соседи добавить в increase;
2. increase: `candidate = max(sourceEmission,
   fromLevel - max(1, opacityTowardSource))`; обновлять только если candidate
   больше сохранённого;
3. учитывать face occlusion shapes, а не только scalar opacity;
4. не писать unloaded chunk; сохранить edge checks до загрузки соседнего chunk.

### LGT-03. Sky light algorithm

- Поддерживать per-column top source/height.
- Над первым blocker значение 15.
- Вертикально вниз через полностью прозрачный block 15 остаётся 15.
- Иначе attenuate минимум на 1 с opacity/shape.
- Placement blocker запускает decrease вниз и в стороны; removal запускает
  increase от sky source и surviving neighbors.
- Chunk generation использует 3×3/edge context, runtime updates — section
  storage + pending edges.

### LGT-04. Client updates

На изменение nibble section отметить `(chunk, section, layer)` dirty. Один раз за
tick собрать masks и arrays в Java `ClientboundLightUpdatePacket`, соблюдая
version-specific bitset/nibble order. Отправлять только игрокам, которым chunk
уже доставлен. Bedrock adapter строит соответствующее subchunk update.

Tests: torch place/remove, opaque roof, colored/partial shapes если поддержаны,
chunk edge, below/above world, skylight/no-skylight dimensions, save/reload,
packet decode реальным клиентским codec.

## 9. M3 — game events, vibrations и весь sculk pipeline

### EVT-01. Общий game-event contract

Создать generated registry `GameEventKey → notification_radius/frequency/tags`.
`GameEventContext` содержит source entity UUID/weak ref, affected block state,
projectile owner и точную source position.

Все gameplay paths обязаны emitting events в vanilla-точках: шаг/плавание,
projectile shoot/land, block place/destroy/change/activate, container open/close,
equip/unequip, entity damage/death/place/mount, fluid pickup/place, eat/drink,
prime/explode, note/instrument и остальные registry events. Составить generated
call-site checklist; отсутствие emitter — parity failure.

### EVT-02. Listener registry

Каждая loaded chunk section имеет `GameEventListenerRegistry`:

- register/unregister block entity/entity listener on load/move/unload;
- mutation во время dispatch идёт в `listeners_to_add/remove` и применяется
  после текущего обхода;
- dynamic entity listener перемещается между sections только после position
  commit;
- empty registry освобождается;
- listener source может быть block position или entity UUID.

Dispatcher:

1. берёт event radius;
2. вычисляет inclusive section min/max XYZ;
3. посещает только loaded chunks (`getChunkNow`, без sync generation);
4. фильтрует Euclidean distance `<= radius²`;
5. immediate delivery вызывает listener сразу;
6. by-distance entries сортирует по distance/stable tie order;
7. dispatch не держит chunk map lock во время listener callback.

### VIB-01. Vibration selection and travel

Состояние каждого vibration listener:

- `current_vibration`;
- `travel_time_ticks`;
- selector candidate + candidate tick;
- particle reload flag;
- source entity/projectile owner identifiers;
- NBT under `listener`.

Selection точно как 26.2:

- если candidate пуст — принять;
- candidate можно заменить только событием того же game tick;
- меньшая distance выигрывает;
- при равной distance выигрывает большая vibration frequency;
- candidate выбирается только на следующем tick;
- travel time обычно `floor(distance)`/user override;
- после reload particle стартует с interpolated remaining position.

Перед приёмом проверить event tag, spectator/sneaking dampening, source state,
listener busy state и occlusion. Occlusion: от центра source block к destination
проверить шесть epsilon-offset rays; событие blocked только если **все шесть**
пересекают `OCCLUDES_VIBRATION_SIGNALS`.

При arrival, если user требует соседние chunks, все 3×3 chunks вокруг listener
должны быть loaded и block-ticking; иначе vibration остаётся pending.

### SCL-01. Sculk sensor

State machine:

```text
INACTIVE
  --valid vibration arrives--> ACTIVE(power, last_frequency), schedule +30
ACTIVE
  --scheduled tick--> COOLDOWN(power=0), notify neighbors, schedule +10
COOLDOWN
  --scheduled tick--> INACTIVE, optional click-stop sound
```

Algorithm:

- range 8;
- reject own place/destroy and frequency 0;
- reject Warden step special case;
- power `max(1, 15 - floor(15/range * distance))`;
- weak signal all sides, direct signal only upward;
- comparator during ACTIVE returns last frequency, иначе 0;
- update pos and below neighbors on activate/deactivate;
- waterlogged suppresses click sounds and schedules water fluid tick;
- adjacent amethyst resonators emit `RESONATE_n` and pitch table event;
- block entity saves last frequency + vibration state.

### SCL-02. Calibrated sensor

- range 16;
- read redstone signal behind block: `facing.opposite` position and correct side;
- comparison 0 accepts all frequencies; 1..15 accepts only equal frequency;
- placement facing, rotation/mirror, waterlogged, active/cooldown semantics same as
  base sensor;
- tests place signal on every direction and verify accepted/rejected events.

### SCL-03. Shrieker and Warden warning

- Listener range 8, tag `SHRIEKER_CAN_LISTEN`.
- Resolve responsible player: direct player; controlling passenger; projectile
  owner; item owner.
- Ignore while already shrieking or without responsible player.
- If `can_summon`, difficulty not peaceful and `spawn_wardens=true`, update
  `WardenSpawnTracker`; otherwise shriek may occur without warning progression
  according to vanilla `canRespond/tryToWarn` flow.
- Set `SHRIEKING=true`, schedule 90 ticks, emit shriek particles/game event.
- At response: warning 4 attempts Warden spawn 20 times, XZ range 5, Y range 6,
  `ON_TOP_OF_COLLIDER`; failed/lower level plays mapped reply sound at random
  offset within 10; apply darkness radius 40.
- Preserve `warning_level` and listener NBT; removal while shrieking executes
  response side effects exactly once.

### SCL-04. Catalyst/spreader/veins/worldgen

Проверить уже перенесённый spreader по vanilla:

- charge cursor NBT, decay, movement, merge order;
- GrowthRules/VeinRules replacement tags;
- catalyst death event, XP/no-XP rules;
- bloom sound/particles;
- sculk vein face set, waterlogging, multi-face survival;
- fixed-seed patch generation и RNG consumption.

Acceptance для sculk: event проходит через dispatcher, chunk unload/reload в
середине travel, calibrated filter, wool occlusion, warning progression и Warden
spawn воспроизводятся end-to-end.

## 10. M4 — canonical recipes и server recipe book

### REC-01. Runtime recipe model

Заменить разделённые generated/dynamic representations одним enum с обязательным
`RecipeKey`:

```text
RecipeHolder {
  key: RecipeKey,
  serializer/type,
  category, group, show_notification,
  placement_info,
  displays: Vec<RecipeDisplay>,
  ingredients/pattern/result/remainders,
  special flag
}
```

Поддержать crafting shaped/shapeless/transmute/special/decorated pot, smelting,
blasting, smoking, campfire, stonecutting, smithing и новые 26.2 serializers.
Ingredient поддерживает item, tag, alternatives и component predicates.
Result/remaining items сохраняют полный component patch.

Recipe manager:

- объединяет builtin + datapack + plugin recipes;
- duplicate key — reload error;
- builds property sets, displays and recipe→display mapping;
- stable iteration по registry order;
- cache key включает full input component identity и registry generation;
- plugin unregister/reload invalidates cache и client recipe sync.

### REC-02. Server recipe book state

Заменить голый `HashSet<String>` структурой:

```text
PlayerRecipeBook {
  known: HashSet<RecipeKey>,
  highlight: HashSet<RecipeKey>,
  settings: Map<RecipeBookType, {open, filtering}>
}
```

NBT `recipeBook`:

- `recipes`: known keys;
- `toBeDisplayed`: highlight keys;
- open/filter flags для crafting/furnace/blast/smoker;
- неизвестные recipe keys при load логируются и удаляются только после registry
  validation; до registry-ready момента raw list сохраняется;
- deterministic sorted write.

`add_recipes` пропускает special и уже known, добавляет known+highlight,
триггерит `RECIPE_UNLOCKED`, разворачивает все displays и отправляет Add entries.
`remove_recipes` удаляет known+highlight и отправляет **Remove packet с display
IDs**, а не replace-add всего registry. `seen` снимает highlight. Settings packet
обновляет server state. Initial sync: settings, затем Add(replace=true) только
known recipes со значением highlight.

### REC-03. Limited crafting enforcement

Проверка выполняется на сервере в момент вычисления/commit result:

```text
allowed = recipe.is_special
       || !world.gamerules.limited_crafting
       || player.recipe_book.contains(recipe.key)
```

- Если `allowed=false`, result slot пуст и recipe-used не записывается.
- Проверка повторяется при click/shift-click, чтобы packet race/gamerule change
  не позволил забрать старый result.
- Serverbound place-recipe разрешён только known recipe и valid current menu.
- Ghost recipe отправляется только при невозможности разложить известный рецепт.
- Crafter — не player и не ограничивается player recipe book.
- Furnace/campfire automated crafting использует свои vanilla rules.

### REC-04. Recipe unlock sources

- advancement rewards;
- `/recipe give|take` с wildcard и exact key;
- plugin API;
- recipe discovery hooks, если они есть в 26.2 data;
- clone/respawn copies recipe book;
- datapack reload удаляет displays deleted recipes и валидирует known keys.

Tests: два recipes с одним result, multiple displays, NBT settings/highlight,
unknown key, reconnect, give/take remove packet, limited crafting manual click и
place-recipe exploit, reload while menu open.

## 11. M4 — inventory, screen handlers и equipment

### INV-01. Authoritative inventory transactions

Каждый click декодируется в semantic action и применяется к server snapshot:

1. проверить sync/container/state ID и still-valid distance/block;
2. проверить slot index/type, carried stack и button/mode bounds;
3. вычислить transaction без доверия client-provided stack;
4. проверить `mayPlace`, `mayPickup`, max stack, components, curse/equipment;
5. atomically commit slots+cursor;
6. вызвать onTake/onEquip/craft hooks;
7. увеличить state ID и отправить diff; при mismatch — full resync.

Реализовать vanilla pickup, quick move, swap, clone, throw, quick craft drag,
pickup all и bundle actions. Drag state machine хранит stage/button/visited slots;
распределяет floor count и remainder в vanilla order, учитывая per-item/per-slot
max stack.

### INV-02. Player inventory persistence

Закрыть явный TODO load-from-NBT: slot numbering main/armor/offhand, selected
hotbar, full item components, invalid/duplicate slot policy. Tests со всеми
слотами, max/overstack corruption, unknown component retention.

### INV-03. Result slots and remainders

Crafting result хранит matched `RecipeKey`, не только output. При take:

- повторно match/allowed;
- consume ровно ingredient counts;
- container/remainder item идёт обратно в input, затем inventory, затем drop;
- onCrafted/stat/advancement/plugin event один раз на фактический count;
- shift-craft повторяет до первого изменения recipe/capacity;
- component-bearing inputs/results не теряют patches.

### INV-04. Equipment

- `Equippable` component определяет slot, allowed entities, dispensable, swap;
- binding curse запрещает pickup в non-creative;
- `onEquipStack(old,new)` вызывается для armor/offhand и entity metadata;
- equipment tick/durability, break event/status, attributes modifier add/remove;
- dispenser equipment выбирает живую подходящую entity в target AABB и первый
  допустимый slot;
- Java/Bedrock slot mapping tests.

### INV-05. Menu lifecycle

Open → initial content/properties → updates → close:

- closing returns crafting inputs/carried stack по vanilla disconnect logic;
- block removal/teleport/death/disconnect closes exactly once;
- viewers count/chest sounds/game events;
- furnace/brewing properties без magic numbers;
- stale packet после закрытия ignored/resynced, не mutates новый menu.

## 12. M4 — Crafter

### CRF-01. Crafter block entity state

Поля: 9 item slots, 9 disabled flags, triggered flag mirror, crafting ticks
remaining, optional loot table. NBT keys 26.2: `Items`, `disabled_slots`,
`triggered`, `crafting_ticks_remaining`. Slot можно disable только если 0..8 и
empty. Setting item в disabled slot re-enables его.

Insertion balancing `canPlaceItem(slot, stack)`:

- disabled → false;
- existing different item/components → false;
- full stack → false;
- если среди более поздних enabled slots есть empty или меньший stack того же
  item/components, не вставлять в текущий slot;
- иначе true. Это обеспечивает vanilla hopper distribution.

Comparator output = число non-empty **или disabled** slots, диапазон 0..9.

### CRF-02. Trigger state machine

- Rising neighbor power при `TRIGGERED=false`: schedule block tick +4, set
  triggered true flags=2, update block entity mirror.
- Falling power: set triggered=false и crafting=false, mirror false.
- Placement уже powered schedule +4.
- Scheduled tick вызывает craft attempt.
- Успех устанавливает `CRAFTING=true`, entity countdown=6.
- Block entity tick decrement; при 0 state `CRAFTING=false` flags=3.
- Fail: fail sound/event, не consume input и не начинает animation.

### CRF-03. Craft algorithm

1. Build 3×3 `CraftingInput`, disabled slots считаются empty, но shape position
   сохраняется.
2. Lookup через versioned `RecipeCache(10)`/RecipeManager; cache key включает
   registry generation и components.
3. Assemble full result; empty result → fail.
4. Set crafting animation before external insertion.
5. `result.onCraftedBySystem`.
6. Для result, затем каждого non-empty remainder вызвать `dispense_item`.
7. После успешной assembly shrink каждый non-empty input ровно на 1.
8. Dirty mark, comparator/inventory updates.

`dispense_item`:

- output direction = orientation.front;
- target container через hopper lookup + sided face opposite;
- если target — crafter или result count превышает target max, вставлять по 1,
  пока failure; иначе вставлять batch до отсутствия progress;
- remainder, который не вошёл, spawn как item на center + direction*0.7 со
  speed 6;
- только при external ejection: craft sound, white smoke, advancement trigger
  игрокам в box 17×17×17 с recipe key и pre-consumption inputs.

Tests: shaped offset/disabled, remainder buckets, target hopper/full chest,
component equality, redstone pulse shorter/longer 4 ticks, comparator 0..9,
save during animation, rotate/mirror orientation.

## 13. M4 — Dispenser и Dropper

### DSP-01. Behavior registry architecture

Создать `DispenseBehaviorRegistry<ItemId, Arc<dyn DispenseBehavior>>` и immutable
bootstrap table. Interface принимает:

```text
DispenseContext { world, dispenser_pos, facing, block_state, block_entity, rng }
DispenseOutcome { remaining_stack, success, used_default_fallback, effects }
```

Default behavior split one item, spawn at vanilla position/velocity, play eject
sound + smoke. Optional behavior меняет success sound/event, но при невозможном
special action применяет строго указанный fallback. Dropper всегда использует
default insertion/drop и никогда item-specific behavior.

Dispenser block:

- rising edge schedules +4, state triggered;
- выбирает случайный non-empty slot vanilla reservoir algorithm;
- empty inventory играет fail event;
- behavior получает copy/reference по transaction rules;
- commit remaining stack только после успешного behavior result;
- block entity dirty + slot update;
- sounds/particles ровно один раз.

### DSP-02. Полная таблица behaviors

Реализовать и проверить **каждую** bootstrap registration 26.2:

- projectiles: arrows, tipped/spectral arrows, eggs variants, snowball, XP
  bottle, splash/lingering potion, firework, fire charge, wind charge;
- armor stand, spawn eggs;
- boats/chest boats/rafts;
- all minecart variants включая command block;
- filled buckets: water/lava/powder snow/fish/axolotl/tadpole/sulfur cube;
- empty bucket pickup;
- flint and steel: fire, TNT, candle/campfire, durability;
- bone meal crop/water plant + level event;
- TNT/gamerule;
- skull/carved pumpkin: place, wither/golem check, иначе equipment fallback;
- all shulker boxes with facing/survival/block entity components;
- glass bottle: beehive honey, water/potion pickup;
- glowstone respawn anchor;
- shears: beehive/entity/shearable durability/events;
- brush behavior;
- honeycomb waxing with state property preservation;
- potion water special interaction;
- equipment via Equippable component;
- chest on tamed chested-horse targets;
- any new 26.2 item discovered by generated bootstrap diff.

Для каждого: success, failure fallback, stack consumption/remainder, creative not
applicable, durability/components, event/sound/particle, target occupied,
unloaded target chunk.

### DSP-03. Projectile details

Spawn position, inaccuracy, power, owner/source, item stack metadata, potion/
firework components и Java/Bedrock spawn metadata должны совпасть. Projectile
spawn failure возвращает stack, а не теряет его. Firework lifetime вычисляется
из Flight component и RNG в vanilla order.

## 14. M6 — полный redstone contract

### RST-01. Общая модель сигнала

Для каждого block behavior явно реализовать:

- `is_signal_source`;
- weak/own signal по запрашиваемой стороне;
- direct/strong signal;
- conductor semantics;
- analog comparator output;
- connection shape;
- neighbor notifications и scheduled delay/priority.

Направление всегда формулировать как «сторона target, из которой читается
источник» и покрывать тестом все 6 directions. Запрещены `unwrap()` после
horizontal conversion без проверки vertical sides.

### RST-02. Wire

- Shape connections north/east/south/west: none/side/up, включая solid support,
  transparent exceptions и wire above/below.
- Placement recalculates shape; dot/cross manual toggle.
- Target power = max(non-wire block signal, incoming wire - 1), clamp 0..15.
- При вычислении direct input временно не учитывать собственное wire emission,
  аналог vanilla `wiresGivePower=false`/evaluator guard.
- State commit flags=2, затем update set pos + six neighbors в deterministic
  order; shape updates отдельно.
- Weak signal только connected horizontal sides/up according to shape; strong
  signal follows vanilla direct rule.
- Rotation/mirror remaps four properties.

Если поддерживается experimental redstone orientation, реализовать отдельный
evaluator, не смешивать порядок с default.

### RST-03. Repeater

- Placement facing/delay 1..4/locked/powered.
- Input только с back; side locking от repeaters/comparators with side power.
- Rising/falling schedule delay `delay*2` с vanilla TickPriority, включая pulse
  extension и already-scheduled check.
- Locked state не переключает output.
- Interaction cycles delay, permission/gamemode checks.
- Notify output neighbors в правильном направлении/order.

### RST-04. Comparator

- Main input from block signal; если conductor и <15, проверить block behind;
- analog source: block entity/container output, item frame/projected sources по
  vanilla правилам;
- side input max(left,right);
- compare: powered iff main >= side; output main else 0;
- subtract: max(main-side,0);
- block entity хранит output; state POWERED отражает output >0;
- schedule priority учитывает facing diode before update;
- interaction toggles mode + sound/state/output recalc.

### RST-05. Torch, observer, buttons, plates, tripwire, target

- Torch burnout history per world/position, 8 toggles/60 ticks, cooldown 160,
  sound/particles.
- Observer: detect front state change, 2-tick pulse, output back, moved-by-piston
  ordering.
- Buttons: material-specific duration, arrows keep wooden pressed, projectile
  check and sounds.
- Pressure plates: stone only living entities; wood all valid entities; weighted
  output `ceil(min(count,max)/max*15)`; collision AABB and recheck ticks.
- Tripwire/hook: full line scan max 41, attached/disarmed/powered/suspended,
  shears, entity collision and neighbor updates.
- Target: hit position strength geometry, arrow 20 ticks/trident 8, remaining
  projectile types, stat/advancement, bullseye threshold and weak signal.

### RST-06. Pistons

Проверить/доделать `PistonStructureResolver`:

- push limit 12;
- destroy/push/block reactions;
- world border/build height;
- slime/honey branching и взаимное non-stick;
- immovable block entities/extended pistons;
- retract sticky pull, short pulse, quasi-connectivity;
- moving piston block entity carries moved state, facing, extending, source,
  progress 0→0.5→1 and collision shapes;
- entity displacement/velocity/slime bounce/honey drag;
- block events and neighbor order;
- save/reload mid-motion and piston-head cleanup.

### RST-07. Rails

- Rail shape recalculation graph, ascending/corners, neighbor survival.
- Powered rail propagation max 8, direction/ascending continuity.
- Activator behavior per minecart type.
- Detector collision box, minecart presence recheck, comparator: command success
  count/container fullness.
- Tests loops, slopes, chunk border, multiple carts, unload during scheduled tick.

### RST-08. Redstone certification suite

Создать reusable test DSL и сравнить tick-by-tick с vanilla:

- clocks (torch/repeater/comparator/observer);
- 0-tick/short pulses where version permits;
- quasi-connectivity piston contraptions;
- hopper lock/cooldown;
- rail lines/detectors;
- chunk save/reload;
- `/fill` stress without stack overflow;
- exact neighbor trace and output after every tick.

## 15. M6 — fluids, fire, weather и block fundamentals

### FLD-01. Fluid state/tick engine

- Separate block state and fluid state; waterlogged exposes source water.
- Source/flowing levels/falling property and dimension tags.
- Tick delay, slope distance, source conversion gamerules.
- Flow computes downward first, then horizontal shortest paths in deterministic
  direction/RNG order.
- LiquidBlockContainer accepts water into dry waterloggable state and schedules
  tick without breaking block.
- Lava-water interactions create stone/cobblestone/basalt with correct sound.
- Block replacement/drop rules, collision/swimming/pushing.
- Unloaded boundary queues no infinite work and resumes when neighbor loads.

### FLD-02. Buckets and cauldrons

Raycast source mode, permission/world border, BucketPickup/emptyContents,
ultrawarm evaporation, sounds/game events, fish/entity NBT, powder snow,
waterlogging and item remainder. Cauldron interaction table data-driven for
water/lava/powder snow/potions/leather/banner/shulker.

### FIR-01. Fire/weather

- Fire age/spread encouragement/flammability generated from blocks.
- Rain exposure uses precipitation, heightmap and adjacent faces; weather-cycle
  gamerule only advances timers, не инвертирует текущее состояние.
- Fire damage gamerule, portal ignition, TNT/campfire/candle interactions.
- Farmland hydration ищет water и rain; trampling predicates.
- Lightning selection, rods, copper oxidation, entity strikes, skeleton horse.

### BLK-01. Universal block checklist

Для каждого registered block family parity-ledger требует:

- default state and every property;
- placement/replacement/survival;
- shape/collision/occlusion/pathfinding;
- rotate/mirror;
- use/attack/step/fall/projectile/entity-inside;
- neighbor/shape/scheduled/random tick;
- redstone/comparator;
- fluid/waterlogged;
- drops/XP/silk/fortune/gamerules;
- sound/particles/game events/stats/advancements;
- block entity/NBT/client updates;
- Java and Bedrock state mapping.

Высокоприоритетные известные gaps: daylight detector sky/ambient calculation,
bell raid hearing, sponge kelp/seagrass/waterlogged BFS, conduit frame/effects/
hunting, respawn anchor explosion/damage/offhand/dimension, beacon beam scanning,
mob spawner entity selection, signs RunCommand security, jukebox event edges.

### ITM-01. Universal item checklist

Для каждого item/component:

- use/use-on/release/finish/consume;
- creative/survival consumption only after success;
- durability/unbreaking/break event;
- cooldown/stat/advancement;
- placement state property preservation;
- entity spawn/projectile payload;
- container remainder;
- dispenser behavior;
- full components in NBT/network/stack equality;
- Java/Bedrock mapping.

Known gaps: campfire cooking/projectile hit, complete bonemeal feature table,
firework payload/lifetime/consumption, wax/scrape state preservation, crop light
threshold/random tick gamerules and special target projectile hooks.

## 16. M5 — entity lifecycle and per-player tracking

### ENT-01. Entity identity/storage lifecycle

World maps UUID→entity and runtime ID→entity. Registration transaction:

1. reject duplicate UUID/runtime ID;
2. attach world and chunk section;
3. publish entity;
4. create tracker;
5. call added hooks;
6. only then emit spawn to eligible players.

Removal has reason, is idempotent, removes tracker/pairings, passengers/leash,
section index и persistent state in correct order. Missing entity in attack/use
packet is normal despawn race and ignored; self-attack/protocol violations retain
vanilla disconnect behavior.

### TRK-01. Per-player visibility tracker

Для каждой entity хранить:

- base tracking range/update interval/track delta from generated EntityType;
- last section/chunk;
- `seen_by` set player connections;
- last sent position/rotation/head/velocity/onGround;
- last passengers, metadata, attributes, equipment;
- edition-specific sync state.

`update_player(entity, player)`:

```text
if same entity: return
effective_range = max(entity range, every indirect passenger range)
visible_range = min(scaled effective_range, player view_distance*16)
visible = horizontal_distance² <= visible_range²
       && entity.broadcast_to_player(player)
       && player has received entity current chunk
if visible and newly seen: send atomic pairing bundle; add set
if not visible and was seen: send remove; remove set
```

Критически важно: «chunk loaded on server» недостаточно — у player должен быть
`delivered_chunks` barrier после успешной отправки terrain packet. При unload/
view-distance move сначала remove entities, затем forget chunk в vanilla order.

В Pumpkin ACK-window bookkeeping не должен считать отменённый плагином либо уже
освободившийся weak chunk полноценной отправленной batch: такие пустые попытки
не увеличивают `batches_sent_since_ack`, а Java `ChunkBatchEnd` сообщает только
число реально записанных terrain packets.

Pairing bundle: spawn, non-default metadata, syncable attributes, equipment,
passengers/vehicle, leash/link и subtype initialization. Removal Java — one/many
entity IDs; Bedrock — RemoveActor unique ID.

### TRK-02. Delta synchronization

Каждый entity update interval:

- relative move only if delta fits codec and teleport delay <400;
- forced position update 60 ticks; forced teleport 400;
- rotation packed bytes with tolerance;
- head rotation separate;
- velocity threshold and zero transition;
- onGround changes;
- dirty metadata/attributes/equipment;
- passenger changes immediately;
- minecart 26.2 uses minecart step packet/lerp sequence where required.

Нельзя очищать dirty metadata до отправки всем current trackers. Java и Bedrock
adapters получают один snapshot и отдельно кодируют.

### ENT-02. Entity persistence and chunk movement

- Перемещение между sections atomically обновляет index и trackers.
- Не tick entity, пока current chunk не FULL/block-ticking according to type.
- Persistent entities сохраняются на actual live position, не spawn chunk.
- Unload/reload быстрых minecart/projectile не создаёт ghost/stale IDs.
- Passenger tree сохраняется root-first; disconnected player root vehicle UUID
  восстанавливается безопасно.

Tests: быстрое пересечение chunk border, teleport через view edge, passenger range,
spectator invisibility, Java+Bedrock simultaneous players, remove/re-add same
runtime ID prevention, 10k entity churn.

## 17. M5 — movement, collision, combat, AI и mobs

### PHY-01. Collision/movement

- Voxel-shape sweep по axes в vanilla order;
- step height candidate path;
- onGround/horizontal/vertical collision flags;
- world border and unloaded chunk policy;
- fluids, ladders/climbable, powder snow, bubble columns, elytra, swimming;
- entity-entity push/collision and vehicle passenger transforms;
- fall distance/reset/damage gamerules;
- anti-cheat movement thresholds/version packet ACK.

Сравнивать position/velocity каждый tick для walking, jumping, sprinting,
sneaking edges, stairs/slabs/fences, ice/slime/honey, water/lava и elytra.

### CMB-01. Damage pipeline

Порядок:

1. invulnerability/difficulty/gamerule/source tags;
2. shield/blocking angle and cooldown;
3. armor+toughness;
4. enchantments/effects/resistance;
5. absorption;
6. hurt cooldown сравнивает полный effective amount;
7. health/lastDamage/combat tracker;
8. knockback/fire/equipment durability;
9. sounds/status/stats/advancements;
10. death, loot/XP, message, removal.

PvP=false блокирует только player-caused damage по player, не атаки mobs.
Explosion damage source/exposure, projectile owner/direct entity и bypass tags
должны сохраняться.

### AI-01. Goal scheduler

- Goal flags mutual exclusion, priority, canUse/canContinue/start/stop/tick order;
- reduced tick rate/adjusted delays;
- target selector separate from behavior selector;
- navigation/path invalidation and stuck detection;
- senses cache LOS per tick;
- memory/brain/schedule for mobs that используют Brain.

Melee attack требует reach + raycast eye-to-eye без solid occluder. Creeper swell
тоже проверяет LOS. Spider climbing metadata следует horizontal collision.
Target predicates учитывают team, invisibility, creative/spectator, follow range,
line of sight и mob-specific tags.

### MOB-01. Полнота типов мобов

Сгенерировать matrix по каждому registered EntityType:

- constructor/default attributes;
- spawn rules/reason/finalizeSpawn;
- goals/brain/sensors;
- variant/baby/breeding/taming;
- interaction/trading;
- sounds/animations/metadata;
- drops/XP;
- NBT;
- special environment behavior.

Запись `generic mob` не считается complete для subtype с уникальной vanilla
логикой. Для каждого типа нужен spawn→tick→save→reload test.

### SPN-01. Natural spawning/despawn

Закрыть явные TODO `natural_spawner.rs`:

- biome/structure-specific mob lists (Nether fortress etc.);
- SpawnPlacements heightmap/rules;
- category caps с spawnable chunk count;
- distance от players/world spawn;
- peaceful/difficulty/gamerules;
- block/fluid collision, dangerous blocks, allowsSpawning;
- pack min/max and group data;
- persistence required/custom persistence;
- despawn immediate/random distance and no-despawn conditions;
- patrol/trader/phantom/cat special spawners.

RNG iteration order и cap accounting сравнить fixed seed за тысячи ticks.

## 18. M5 — vehicles, minecarts и projectiles

### VEH-01. Shared vehicle/passenger contract

- start/stop riding validates cycles, capacity, dimension, removal;
- controlling passenger selection;
- passenger local offset/rotation updated after vehicle movement;
- dismount finds safe pose using collision/fluid rules;
- damage/wobble/hurt animation/drop gamerules;
- portal/dimension transition either carries allowed tree atomically or detaches
  exactly as vanilla;
- save/reload recursive passengers.

### MCT-01. Minecart physics

Сравнить 26.2 `AbstractMinecart`, `OldMinecartBehavior` и выбранный experiment
profile. Для default behavior:

- detect rail block below/current;
- project position onto rail endpoints for every `RailShape`;
- slope acceleration, powered rail acceleration/brake, activator callback;
- max speed, natural slowdown, water penalty;
- off-rail gravity/ground friction/collision;
- yaw flip behavior;
- entity collision: cart-cart momentum by direction/type, mobs/players boarding;
- passenger position;
- interpolation/network step list;
- fall/void/portal.

Не смешивать old/new minecart experiment. Feature flag выбирает отдельную
strategy и packet behavior.

### MCT-02. Minecart subtypes

- Rideable: interaction/passenger rules.
- Chest: 27 slots, loot table deferred unpack, NBT/drop/content signal.
- Hopper: 5 slots, enabled activator state, pickup AABB, transfer cooldown,
  sided inventory/components.
- Furnace: fuel interaction, push vector, fuel decrement, lit metadata/smoke.
- TNT: ignite sources/activator/projectile/fire/fall, fuse, rail protection,
  speed/fall explosion bonus, gamerule.
- Command block: command block logic, success count, activator cooldown,
  comparator, permissions/NBT/output.
- Spawner: embedded BaseSpawner tick/spawn/NBT/client event.

Dispenser placement, detector comparator, save/reload and entity tracking tests
обязательны для каждого subtype.

### BOAT-01. Boats

Water status sampling, buoyancy, paddle input, friction by block, bubble columns,
collision, passenger seats/dismount, chest inventory, leash if version supports,
fall break/drop and Java/Bedrock interpolation.

### PRJ-01. Projectiles

Общий projectile pipeline:

- owner/direct source and left-owner check;
- continuous segment raycast blocks + entities, nearest hit ordering;
- portal/gateway, friendly-fire/team;
- hit cancellation only where plugin contract permits;
- item/component payload;
- gravity/drag/water/bubbles;
- in-ground state, shake, despawn/pickup;
- save/reload;
- game events and damage source.

Subtype matrix: arrows/tipped/spectral/trident, thrown potions/XP/egg/snowball,
ender pearl, firework, fireball/fire charge, wind charge/breeze, fishing hook,
wither skull, dragon fireball, llama spit, shulker bullet. Каждый subtype должен
иметь unique effects, metadata, NBT и tests.

## 19. M7 — world generation, structures и POI

### WGN-01. Deterministic RNG foundation

Закрыть tests в `pumpkin-util/random`:

- LegacyRandomSource/Java LCG;
- Xoroshiro128++;
- positional/hash seed derivation;
- fork/forkPositional;
- `nextInt(bound)` rejection behavior;
- Gaussian cache;
- shuffle/Fisher–Yates;
- large feature seed/carver/decoration seed formulas.

Golden vectors получить вызовом vanilla helper для fixed seeds. Любая
оптимизация сохраняет число и порядок RNG calls.

### WGN-02. Chunk generation stages

Для каждого dimension:

```text
EMPTY → STRUCTURE_STARTS → STRUCTURE_REFERENCES → BIOMES → NOISE
→ SURFACE → CARVERS → FEATURES → INITIALIZE_LIGHT → LIGHT
→ SPAWN → FULL
```

Каждая stage:

- принимает immutable dependencies и expected prior status;
- idempotent/cancel-safe;
- не публикует partial LevelChunk;
- сохраняет status/NBT;
- имеет deterministic neighbor radius/cache;
- cancellation/unload не оставляет holder stuck.

### WGN-03. Noise/surface/carvers/aquifers

- Generated density functions/noise router exact 26.2 data.
- minY/height/sea level from dimension, без constants 63/32/319.
- RandomState/noise caches keyed by seed+registry generation.
- Climate biome selection.
- Surface rules and material conditions.
- Aquifer pressure/fluid status and barrier noise.
- Cave/canyon carver masks, lava levels and RNG.

Сравнивать chunk block/biome hashes по fixed seeds для overworld/nether/end,
затем full NBT snapshots на representative coordinates.

### WGN-04. Placed/configured features

Placement modifiers должны выполняться lazy per-position:

```text
positions = [origin]
for modifier in ordered_modifiers:
  for each current position in stream order:
    emit modifier positions and immediately continue chain
for final position:
  configured_feature.place(world, rng, pos)
```

Не materialize все позиции до размещения feature: ранний feature меняет мир для
следующего. Реализовать полный codec table providers/predicates/modifiers/
features. Известные gaps: coral/tree exact `shuffledCopy`, random axis providers,
root water checks, vines ages, sea-level constants, tree foliage radius.

### WGN-05. Structures

Общий pipeline:

- StructureSet placement (random spread/concentric rings), salt/spacing/
  separation/frequency reduction;
- biome/terrain checks;
- starts/references persisted;
- piece graph bounding boxes/orientation;
- template palette, processors, jigsaw pools/connectors/projection;
- rotation/mirror transforms block states и block entity NBT;
- post-processing/fluid ticks;
- loot/spawner/entity placement with deterministic seeds.

Для каждой vanilla structure создать ledger/test:

- villages variants;
- mineshafts variants;
- strongholds;
- monuments;
- mansions;
- ocean ruins/shipwrecks;
- ruined portals;
- jungle/desert pyramids, swamp hut, igloo;
- Nether fortress/bastion/fossil;
- End city;
- ancient city, trail ruins, trial chambers;
- buried treasure, witch hut and remaining structure sets in 26.2 registry.

Mineshaft acceptance включает rails, cobwebs, cave-spider spawner, support
beams, chest minecarts с loot table, chains/intersections и liquid clipping.
Ocean monument — graph/rooms, shell/wings/penthouse, elder guardians, sponge/gold
rooms, water fill, exact heightmap restrictions.

### WGN-06. POI and villages

- Generated POI types/block states;
- section storage NBT and consistency rebuild;
- add/remove при block mutation;
- occupancy tickets claim/release;
- nearest/random queries deterministic;
- villager brain uses POI across chunk load/unload;
- portals тоже используют correct POI/search cache;
- village raids/golem/cat mechanics.

### WGN-07. Worldgen certification

Набор seeds: 0, 1, -1, min/max i64 и минимум 20 regression seeds. Сравнивать
representative 32×32 chunk regions всех dimensions: status, blocks, biomes,
heightmaps, structures, block entities, ticks, POI и RNG trace for selected
features. Большие golden blobs хранить compressed с documented generator hash.

## 20. M8 — commands, gamerules, loot, advancements и stats

### CMD-01. Brigadier semantics

Для каждой vanilla command 26.2:

- exact tree/literals/arguments/redirect/forks;
- permission level and command source capabilities;
- suggestions/resource registries;
- error type, cursor and translatable arguments;
- return value and success/failure feedback/broadcast;
- dimension/anchor/rotation/selectors;
- Java command tree version encoding и Bedrock available commands mapping.

Создать generated command inventory, чтобы отсутствующая команда видна в CI.
Сравнить dispatcher parse result на corpus valid/invalid commands.

### CMD-02. Selectors and SNBT/NBT paths

- `@p/@a/@r/@e/@s`, sort/limit/distance/box/level/gamemode/team/name/type/tag/
  scores/advancements/predicate/nbt;
- local/world coordinates and anchors;
- SNBT escaping, numeric suffixes, arrays/lists/compounds;
- NBT path get/set/merge/remove with bounds;
- resource-location autocomplete.

Malformed/expensive selectors имеют limits и не panic.

### GMR-01. Gamerule wiring

Сгенерировать table всех 26.2 rules, default/type/range/callback. Для каждой
rule ledger указывает runtime call sites. Особое внимание:

- drops/XP/spawning/damage/fire/weather/time;
- random tick speed;
- immediate respawn/reduced debug/limited crafting packets on live change;
- command chain/update limits;
- wardens/TNT/raids/patrols/traders/phantoms;
- water/lava source conversion;
- players sleeping percentage;
- global sounds/locator bar/version additions.

Test меняет rule во время активной системы и после restart.

### LOT-01. Loot

Полный codec/runtime для pools, entries, conditions, functions, number providers,
context params, luck, enchantments, explosion decay, looting, tool/entity/
damage-source predicates. RNG sequence per table/pool deterministic. Block/entity/
chest/fishing/advancement rewards use correct LootContext. Deferred container
loot table unpack only on first access with saved seed.

Текущий промежуточный gate: `DataPackLoader` уже атомарно валидирует и публикует
raw loot-table JSON (вместе с predicates/advancements/structures) с canonical
resource IDs и pack priority. Отложенные сундуки теперь исполняют поддержанный
datapack-поднабор (item/empty/tag/вложенные loot_table, deterministic rolls,
set_count/limit_count) через тот же split/shuffle путь, что и встроенные таблицы;
неподдержанные или циклические таблицы сохраняются в NBT и не превращаются в
пустой сундук. Это не заменяет LOT-01: следующим исполнителю нужно подключить
полные typed loot-table codecs к `LootTableExt`, сохранив неизвестные поля,
conditions/functions/components, context seed/luck и deferred semantics для
entity/block/fishing/advancement/command consumers.

### ADV-01. Advancements, criteria, rewards

- Load datapack trees/requirements/display;
- every trigger used by 26.2 gameplay;
- listener registration/unregistration;
- progress timestamps/NBT JSON save;
- visibility algorithm and packets;
- rewards: XP, loot, recipes, functions;
- revoke/grant command;
- recipe unlock integration;
- plugin events without double trigger.

`DataPackLoader` уже даёт validated raw advancement snapshot и отклоняет
malformed `criteria` до reload commit. Для acceptance ADV-01 этот snapshot нужно
преобразовать в runtime advancement graph с dynamic IDs, listeners, rewards и
Java/Bedrock update packets; статический generated `Advancement` registry нельзя
считать заменой datapack деревьев.

### STA-01. Stats/scoreboards/bossbars/teams

- Generated stat registries and increments at exact action commit;
- persistence and client sync;
- scoreboard objectives/criteria/display slots/scores, teams/options;
- command-triggered and gameplay criteria;
- bossbar persistence/viewers/properties;
- death messages/team visibility/collision rules.

## 21. M9 — Java protocol 26.2/26.1

### NET-01. State machine

Явные allowed packet sets для handshake/status/login/configuration/play. Packet в
неверном state → vanilla-equivalent disconnect/ignore. Timeouts, rate limits,
compression/encryption order, online/offline auth, secure profile и proxy modes
покрываются tests.

### NET-02. Codec completeness

Для каждого packet:

- ID per version/state/direction;
- field order/type/endian/VarInt bounds;
- optional discriminants and collection size caps;
- registry-aware IDs;
- text/NBT/component depth limits;
- round-trip golden bytes;
- truncated, overlong, invalid enum/id tests.

CI сравнивает generated packet table с 26.2 registry/codecs. Unknown clientbound
packet gap и handlerless serverbound packet не могут иметь status complete.

### NET-03. Login/config/play ordering

Проверить реальным клиентом:

- status/version/sample/favicon;
- login encryption/session auth/compression;
- known packs, registries, tags, recipes, commands, resource packs;
- configuration finish ACK;
- login packet flags/gamerules/dimensions;
- spawn position, abilities, difficulty, inventory, recipes, advancements,
  chunks/light, entity pairing;
- respawn/dimension change ordering;
- disconnect/save.

### NET-04. Chunk and entity packets

- Chunk palette/network bit packing independent from disk packing;
- block entities/heightmaps/light masks;
- delivery ACK/barrier if protocol supplies it;
- block/section updates;
- entity spawn/move/teleport/metadata/attributes/equipment/passengers/remove;
- no entity packet before its chunk terrain;
- batching/bundles in legal order.

### NET-05. Interaction security

- reach, loaded chunk, world border, gamemode, sequence ACK;
- main/offhand mapping by protocol version;
- duplicate/reordered packet resistance;
- missing entity race ignored;
- inventory server authoritative;
- chat signatures/last-seen chain/reporting according to selected online mode;
- custom payload length/channel permissions;
- command block/sign command permissions.

Compatibility matrix: vanilla clients 26.2 and 26.1, online/offline, compression
threshold variants, resource pack accept/decline, reconnect, high latency and
packet fuzz corpus.

## 22. M10 — Bedrock, plugins и Pumpkin extensions

Java parity сначала фиксирует authoritative semantics. Следующие задачи не
должны добавлять Bedrock-specific условия внутрь gameplay algorithms.

### BDR-01. Bedrock transport/session

- RakNet discovery/connect/reliability/order/split/reassembly/MTU;
- encryption/auth chain/XUID/skin validation;
- NetherNet UDP discovery + signaling как единый transport, без смеси старого
  HTTP flow;
- login/resource packs/start game/biome/entity/block mappings;
- disconnect/reconnect and malformed packets;
- rate/size limits.

### BDR-02. Gameplay adapter

Для каждого shared action существует Bedrock decode → authoritative command →
Bedrock result mapping:

- movement/teleport ACK;
- inventory stack request/response and stack network IDs;
- crafting/recipes;
- block/item/entity interaction;
- forms/commands/chat;
- chunks/subchunks/light;
- entity metadata/attributes/equipment/passengers;
- simulation distance per player.

Generated remaps versioned. Unknown Java-only state получает documented fallback
и round-trip test; silent air/zero mapping запрещён.

### PLG-01. Plugin contract

- WIT schema versioned and regenerated in same commit;
- host methods validate resource ownership/lifetime;
- events emitted at pre/post commit points exactly once;
- cancellation rollback-safe;
- mutable item/block values preserve components/state;
- permissions checked server-side;
- callback timeout/fuel/memory limits;
- plugin panic/trap disables callback, не server;
- unload removes commands/listeners/tasks/recipes and invalidates registries;
- compatibility tests with example plugin for every API area.

### EXT-01. Pump/Linear formats

Это extensions, но production-ready требует:

- documented schema/version/migration;
- atomic writes/checksums/index recovery;
- conversion Anvil↔extension без semantic loss;
- backup/restore tool;
- fuzz/corruption tests;
- unknown vanilla NBT preserved.

## 23. M11 — reliability, performance, security и operations

### REL-01. Panic audit

Классифицировать все найденные `panic!`:

- `test assertion` — оставить в tests;
- `proven internal invariant` — заменить `debug_assert` + typed error at input
  boundary или доказать in comment/test;
- `external data/network/world race` — обязательно `Result/Option`, quarantine
  corrupted object/connection;
- `decompiler/unsupported feature` — parity task, не panic.

Fuzz targets: NBT/SNBT, Anvil region, palettes, every packet state, commands,
item components, datapack JSON/codecs, plugin resources.

### REL-02. Async/deadlock audit

- lock order document;
- no lock across await unless lock специально reentrant-safe и операция bounded;
- no network send under world/entity inventory lock;
- chunk load cancellation releases tickets/wakers;
- shutdown drains tasks;
- loom/model tests for scheduler/dirty generation where practical;
- watchdog prints task/lock/chunk holder diagnostics.

### PERF-01. Performance gates

Correctness first, затем profiles. Budgets измерить относительно hardware CI:

- 20 TPS with defined players/entities/chunks;
- p95/p99 tick time;
- chunk generation throughput;
- memory per loaded chunk/entity/player;
- packet bandwidth;
- save latency and dirty backlog;
- no unbounded queue/map/cache.

Optimization допускается только с parity tests. Hash iteration нельзя делать
источником gameplay order. Blocking compression/file IO вынести с Tokio workers.

### OPS-01. Production lifecycle

- config validation and clear errors;
- EULA/server properties compatibility where intended;
- signals/console EOF graceful shutdown;
- logs without secrets/session tokens;
- RCON/query/LAN rate/security;
- backups at consistent save barrier;
- metrics for tick/chunks/entities/network/save/tasks;
- crash report with version profile and affected object;
- upgrade/downgrade warning and read-only recovery mode.

### SEC-01. Abuse resistance

- packet/collection/NBT/string/decompression limits;
- auth/proxy trust boundaries;
- command/plugin permissions;
- inventory duplication tests under replay/reorder;
- chunk generation/ticket amplification limits;
- entity/particle/sound broadcast caps;
- regex/JSON/SNBT worst cases;
- dependency audit and unsafe-code review.

## 24. Полная test matrix

### 24.1. На каждый PR/task

- formatter + diff check;
- affected crate check/test/clippy;
- new unit test;
- one boundary test;
- parity-ledger validation;
- no generated drift.

### 24.2. Nightly

- `cargo test --workspace --all-targets`;
- protocol round-trip/malformed corpus all supported versions;
- real Anvil/player/level round-trips;
- 26.2 differential scenarios;
- fixed-seed worldgen regions;
- Java headless client connect/play/reconnect;
- Bedrock client/bot scenarios;
- plugin examples;
- fuzz smoke;
- 2-hour mixed gameplay soak.

### 24.3. Release candidate

Минимальные сценарии, каждый vanilla vs Pumpkin:

1. Новый мир всех dimensions → explore → save → restart → explore.
2. Import vanilla world with all block entities/structures/ticks → mutate →
   reopen vanilla.
3. Two players: visibility, inventories, combat, death/respawn/dimensions.
4. Redstone certification world for 10 000 ticks + reload.
5. Crafter/dispenser/hopper/furnace automated factory.
6. Sculk sensor/calibrated/shrieker/Warden and wool occlusion.
7. Every vehicle/projectile across chunk/view borders.
8. Villages/POI/raids/trading/spawning.
9. Datapack override/reload recipes/loot/functions/worldgen.
10. Every command parse/permission/error corpus.
11. Network high latency/loss/reorder where transport permits.
12. Graceful shutdown and forced crash recovery during save.
13. Java 26.2, Java 26.1 compatibility profile.
14. Supported Bedrock versions.
15. 24-hour soak with periodic autosave/reload/client churn.

### 24.4. Release blockers

Любое из следующего блокирует «complete»:

- data loss or vanilla cannot reopen saved world;
- duplication/item loss under legal interaction;
- panic/deadlock/infinite loop from client/world data;
- entity ghost/stale tracking;
- deterministic parity mismatch without documented version difference;
- missing mandatory packet/registry entry;
- `partial/mostly/missing/unknown` in server-required ledger;
- flaky test (его нельзя просто rerun до green);
- manual generated-file patch;
- TODO в behavior, который marked complete.

## 25. Конкретный порядок первых implementation batches

Ниже — порядок, который минимизирует переделки текущего кода.

### Batch A — foundational blockers

1. `BAS-01` VersionProfile.
2. `PAR-01..04` ledger + differential harness skeleton.
3. `REG-01` Identifier/RecipeKey и сохранение plugin recipe ID.
4. `REG-02..03` registry snapshot/datapack reload foundation.
5. `PST-01..04` complete chunk model, unknown NBT, ticks.
6. `PST-05..07` live block entity/player/entity save and real fixtures.
7. `TIK-01..02`, `NBR-01`, `MUT-01`.
8. `LGT-01..04`.

Gate A: vanilla fixture round-trip, scheduler trace, lighting suite and 1-hour
chunk load/unload/save soak green.

### Batch B — requested gameplay blockers

1. `REC-01`: recipe IDs/model/manager.
2. `REC-02..04`: complete recipe book/limited crafting/unlocks.
3. `INV-01..05` enough for canonical crafting transaction.
4. `CRF-01..03` full Crafter.
5. `DSP-01..03` full behavior registry/table.
6. `EVT-01..02`, `VIB-01`, `SCL-01..04`.

Gate B: automated factory + recipe reconnect + full sculk differential scenarios
green; no known missing dispenser bootstrap registrations.

### Batch C — entities and redstone

1. `ENT-01`, `TRK-01..02`, `ENT-02`.
2. `PHY-01`, `CMB-01`, `AI-01`, `SPN-01`.
3. `VEH-01`, `MCT-01..02`, `BOAT-01`, `PRJ-01`.
4. `RST-01..08`, `FLD-01..02`, `FIR-01`.
5. Complete block/item matrices.

Gate C: multi-player tracking, combat/mob/spawn soak, redstone certification,
all vehicle subtype reload tests green.

### Batch D — world/data/protocol completeness

1. `WGN-01..07`.
2. `CMD-01..02`, `GMR-01`, `LOT-01`, `ADV-01`, `STA-01`.
3. `NET-01..05` full generated protocol inventory.
4. `BDR-01..02`, `PLG-01`, `EXT-01`.
5. Reliability/performance/security audit.

Gate D: parity-ledger has no incomplete server-required rows and the release
candidate matrix passes twice from clean checkout.

## 26. Шаблон одной задачи для менее способной LLM

Копировать этот шаблон для каждого task ID. Не объединять несвязанные systems в
один огромный diff.

```markdown
# Task: <ID> <название>

## Scope
Только: <files/contracts>.
Не менять: <adjacent systems>.

## Vanilla reference
- File: Minecraft/decompiled_src/sources/<path>.java
- Methods/constants: <exact list>
- Version: 26.2

## Current Pumpkin path
- <Rust files/call sites>

## Preconditions/dependencies
- <completed task IDs>

## Required state
- <fields, types, owner, persistence keys>

## Exact algorithm
1. <ordered step>
2. <ordered step>
...

## Side effects
- state/dirty mark
- scheduled ticks/order/priority
- neighbors/redstone/light
- game events/sounds/particles
- stats/advancements/plugins
- Java packets
- Bedrock packets

## Edge cases
- unloaded chunk
- despawn/removal
- save/reload mid-operation
- malformed/unknown data
- dimension bounds/gamerules
- duplicate/reordered packet

## Tests to add first
- unit: <name and assertion>
- boundary: <name and assertion>
- differential: <scenario and expected trace>
- persistence/protocol: <fixture>

## Acceptance criteria
- [ ] every algorithm step implemented
- [ ] no fallback approximation/TODO
- [ ] tests green
- [ ] no unrelated diff
- [ ] parity ledger updated to complete
- [ ] docs updated

## Commands
<exact cargo/parity commands>

## Final report
- changed files
- vanilla differences found
- tests/results
- remaining blockers (must be empty for this task)
```

## 27. Обязательные subsystem cards, которые нельзя потерять между milestones

Следующие системы должны получить отдельные parity-ledger rows и task cards,
даже если в предыдущих разделах описан общий механизм.

### DIM-01. Dimensions, portals and respawn

- Dimension registry: minY/height/logical height, skylight, ultrawarm, bed/anchor
  rules, coordinate scale, ambient light, infiniburn, effects.
- Nether portal: frame validation/creation, axis, portal blocks, entity cooldown,
  coordinate scaling, world-border clamp, destination search order/radius/POI,
  safe exit shape/orientation/velocity, platform creation, chunk tickets.
- End portal: frame eyes, activation, dimension transition, spawn platform,
  credits/win state where server-visible.
- End gateway: exact exit search/generation, cooldown/beam event, entity teleport.
- Bed respawn: dimension lookup, chunk load, block still valid, obstruction,
  explosion in forbidden dimension, spawn angle/position search.
- Respawn anchor: charge/use/offhand priority, Nether dimension rule, safe position,
  explosion damage calculator/resistance/fire, consume charge.
- Death fallback resolves target dimension world spawn, not current death world.
- Dimension change preserves/removes passengers/projectiles/effects according to
  subtype and sends Java/Bedrock packets in exact order.

Tests: portal at borders/build limits, linked portals, missing target world,
bed/anchor changed while dead, player respawn in other dimension, reconnect in
portal, vanilla NBT spawn dimension.

### CNT-01. Furnace family

- input/fuel/result slot predicates;
- fuel table and container remainder;
- burn time/cooking progress reset/rescale when recipe changes;
- recipe lookup by key and components;
- XP and recipes-used accumulation, extraction awards;
- lit block state, NBT, menu properties, hopper sided rules;
- blast/smoker speed and allowed recipe types.

### CNT-02. Brewing stand

- potion/container/ingredient recipe registries;
- fuel blaze powder;
- 400-tick brew, ingredient identity recheck, bottle slots;
- full components and remainder handling;
- block state bottle flags, NBT/menu/hopper sides, events/stats.

### CNT-03. Enchanting/anvil/grindstone/smithing/stonecutter

- Enchanting seed, bookshelf power/occlusion, clue generation, lapis/levels,
  weighted enchant selection, item components, stats/criteria.
- Anvil rename/combine/repair/enchantment compatibility/cost/too-expensive,
  prior-work penalty, material consumption, damage/break chance, creative rules.
- Grindstone removal/curses/XP and item merge.
- Smithing template/base/addition recipe, component transfer and consumption.
- Stonecutter recipe list/selected ID/result/quick move/stat.

### CNT-04. Loom/cartography/beacon/merchant/horse inventories

- Loom pattern registry, banner components, selection/result.
- Cartography map scale/lock/clone, invalid combinations.
- Beacon pyramid scan/beam colors, primary/secondary validation, payment,
  periodic effects, advancement, block update.
- Merchant offers: demand/special price, uses/max uses, XP, restock, reputation,
  trade result matching components and menu synchronization.
- Horse/chested horse equipment/chest slots/access/drops.

### EFF-01. Attributes, effects, enchantments, hunger

- Generated attribute base values/ranges/sync; modifier identity, operation order
  (add, multiply-base, multiply-total), persistent/transient separation.
- Effect instance hidden chain, duration/amplifier/ambient/particles/icon,
  combine/remove/tick cadence, immunity and metadata.
- Implement every 26.2 effect behavior: damage/heal, regeneration/poison/wither,
  hunger/saturation, absorption/health boost, levitation, darkness, raid omen,
  wind charged/weaving/oozing/infested and version additions.
- Enchantment provider/effect components, applicability/tags/costs, combat/mining/
  loot hooks, curses, anvil/enchanting integration.
- Hunger exhaustion/saturation/food level, natural regeneration, starvation,
  difficulty/gamerule and saturating arithmetic.

### ECO-01. Villagers, trading and reputation

- Professions/POI acquisition/release, schedules/brain memories;
- gossip transfer/decay, player reputation, curing discounts;
- trade generation by level/biome, demand/restock/max uses;
- farming, breeding, panic, sleep, doors/bells/gathering;
- wandering trader spawn/despawn/offers/llamas;
- zombie villager conversion/cure persistence.

### RAID-01. Raids, patrols and village defense

- Bad omen/raid omen timing, raid creation from POI village;
- wave composition by difficulty/wave/bonus, spawn position search;
- raider membership/captain/banner, bossbar, horn, regroup;
- victory/loss/hero effect, persistence/reload;
- patrol special spawner and gamerules;
- iron golem/cat village spawning and bell raid hearing.

### BOS-01. Bosses and complex entities

- Ender dragon fight persistent state, crystals, phases/path nodes, exit portal,
  gateways, XP and respawn ritual.
- Wither spawn pattern, invulnerability, heads/targets, block breaking, armor
  phase, explosion/nether star.
- Warden brain/anger/vibration/sonic boom/dig/roar/sniff and spawn tracker.
- Elder guardian curse, shulker attachment/bullet, phantom size/swoop, slime/
  magma split, guardian beam and remaining subtype mechanics.

### TRL-01. Trial spawner, vault and archaeology

- Trial/ominous spawner state machine, participant detection, wave mobs,
  cooldown/rewards, NBT and client events.
- Vault key/ominous key, per-player rewarded set, display item, ejection and NBT.
- Brushable blocks: brushing progress/decay, loot item, falling behavior,
  archaeology structures/processor placement.

### MAP-01. Maps, waypoints, locator and world-visible state

- Filled map saved data, dimension/scale/center/colors, decorations, tracking,
  banner/frame markers, update packets and cartography.
- Lodestone compass tracker/components.
- Waypoints/locator bar eligibility, dimension translation and live updates.
- World border lerp/damage/warning/commands and packets.
- Time/weather/difficulty/spawn position live packets.

### FUN-01. Functions, schedules and server automation

- Datapack function parser/execution queue, command source/return/run limits;
- function tags tick/load;
- `/schedule` persistent timer queue and replace/append;
- command blocks/minecart command blocks chain/conditional/auto/power, updateLast
  execution, max chain gamerule, output/success count/comparator;
- game loops cannot recurse Rust stack unboundedly.

### FSH-01. Fishing

- Hook casting/owner validation, flight/bobber water state/collision;
- open-water check volume, wait/lure/hooked timings and weather/sky modifiers;
- entity hook/pull, loot table with luck/lure, durability/stats/advancements;
- NBT/removal on owner invalid/dimension/death and client metadata.

### SND-01. Observable effects

Sounds, level events, particles, entity status and game events являются частью
parity. Для каждого behavior task acceptance должен указать:

- event ID/key;
- exact source position/category;
- audience/tracking radius/global gamerule;
- volume/pitch RNG order;
- block/entity data payload;
- suppression (waterlogged, silent entity, failed action).

Отсутствующий звук редко ломает state, но остаётся видимым отличием и поэтому не
может находиться в complete task без явного N/A.

### 27.1. Карта исходников для начала каждой группы задач

| Область | Pumpkin entry points | Vanilla 26.2 entry points |
|---|---|---|
| Server tick/lifecycle | `crates/pumpkin/src/{lib.rs,server/,world/mod.rs}` | `server/MinecraftServer.java`, `server/level/ServerLevel.java` |
| Chunk pipeline | `crates/pumpkin-world/src/{chunk_system/,level.rs}` | `server/level/{ServerChunkCache,ChunkMap,ChunkHolder}.java` |
| Chunk NBT | `crates/pumpkin-world/src/chunk/format/{mod,anvil}.rs` | `world/level/chunk/storage/SerializableChunkData.java` |
| Lighting | `crates/pumpkin-world/src/lighting/` | `world/level/lighting/` |
| Neighbor updates | `crates/pumpkin/src/world/mod.rs`, `block/mod.rs` | `world/level/redstone/{NeighborUpdater,CollectingNeighborUpdater}.java` |
| Scheduler | `crates/pumpkin-world/src/tick/` | `world/ticks/` |
| Recipes | `crates/pumpkin/src/server/recipe.rs`, `crates/pumpkin-inventory/src/crafting/` | `world/item/crafting/RecipeManager.java`, `stats/ServerRecipeBook.java` |
| Player recipe book | `crates/pumpkin/src/entity/player.rs`, `command/commands/recipe.rs` | `stats/{RecipeBook,ServerRecipeBook,RecipeBookSettings}.java` |
| Crafter | `crates/pumpkin/src/block/blocks/redstone/crafter.rs`, `crates/pumpkin/src/block/entities/crafter.rs` | `block/CrafterBlock.java`, `block/entity/CrafterBlockEntity.java` |
| Dispenser | `crates/pumpkin/src/block/blocks/redstone/dispenser.rs`, `item/items/` | `core/dispenser/`, `block/DispenserBlock.java` |
| Game events/sculk | `crates/pumpkin/src/block/entities/sculk*`, `crates/pumpkin-world/src/generation/feature/features/sculk/` | `world/level/gameevent/`, `block/*Sculk*`, `block/entity/*Sculk*` |
| Redstone | `crates/pumpkin/src/block/blocks/redstone/` | `block/{RedStoneWireBlock,RepeaterBlock,ComparatorBlock,ObserverBlock}.java`, `world/level/redstone/` |
| Pistons | `crates/pumpkin/src/block/blocks/piston/`, `block/entities/piston.rs` | `world/level/block/piston/` |
| Fluids | `crates/pumpkin/src/block/fluid/` | `world/level/material/`, `block/LiquidBlock.java` |
| Inventory/menu | `crates/pumpkin-inventory/src/`, `crates/pumpkin/src/entity/player.rs` | `world/inventory/`, `world/entity/player/Inventory.java` |
| Entities/tracking | `crates/pumpkin/src/entity/`, `world/chunker.rs` | `world/entity/`, `server/level/{ServerEntity,ChunkMap}.java` |
| Minecarts | `crates/pumpkin/src/entity/vehicle/minecart*` | `world/entity/vehicle/minecart/` |
| AI/spawning | `crates/pumpkin/src/entity/ai/`, `world/natural_spawner.rs` | `world/entity/ai/`, `world/level/NaturalSpawner.java` |
| Combat/effects | `crates/pumpkin/src/entity/{living,combat,hunger}.rs` | `world/entity/LivingEntity.java`, `world/damagesource/`, `world/effect/` |
| Worldgen | `crates/pumpkin-world/src/generation/` | `world/level/levelgen/`, `data/worldgen/` |
| Structures | `crates/pumpkin-world/src/generation/structure/` | `world/level/levelgen/structure/`, `data/structures/` |
| POI/villages | `crates/pumpkin-world/src/poi/`, `crates/pumpkin/src/entity/passive/villager*` | `world/entity/ai/village/poi/`, `world/entity/npc/` |
| Commands | `crates/pumpkin/src/command/` | `commands/`, `server/commands/` |
| Advancements/loot/stats | `crates/pumpkin/src/{entity/player/advancement.rs,world/loot.rs}` | `advancements/`, `world/level/storage/loot/`, `stats/` |
| Java protocol | `crates/pumpkin-protocol/src/java/`, `crates/pumpkin/src/net/java/` | `network/protocol/`, `server/network/ServerGamePacketListenerImpl.java` |
| Bedrock | `crates/pumpkin-protocol/src/bedrock/`, `crates/pumpkin/src/net/bedrock/` | Bedrock protocol data + shared authoritative Java gameplay references |
| Plugins | `crates/pumpkin-plugin-api/`, `crates/pumpkin/src/plugin/` | Pumpkin extension; compare only event timing with relevant gameplay method |
| Generated data | `crates/pumpkin-data/src/generated/`, `tools/pumpkin-codegen/` | `Minecraft/resources/data/minecraft/`, builtin registries/data generators |

Пути в vanilla table относительны `Minecraft/decompiled_src/sources/net/minecraft/`.
Перед началом задачи всё равно выполнить `rg` по имени method/type: decompiler
может разнести один contract по helper classes.

## 28. Как доказывать, что «всё» действительно закончено

Финальный аудит не ищет только TODO. Он выполняет четыре независимые проверки:

1. **Coverage proof:** каждый server-relevant vanilla contract 26.2 имеет
   complete/N/A ledger entry с tests.
2. **Behavior proof:** differential scenarios совпадают по observable state и
   ordered effects.
3. **Persistence proof:** двусторонний vanilla↔Pumpkin round-trip не теряет
   известные и неизвестные данные.
4. **Operational proof:** release matrix, fuzz, crash recovery и soak проходят
   без panic, deadlock, data loss и unbounded growth.

Только одновременное выполнение всех четырёх пунктов означает полную готовность.
Пока хотя бы один contract не учтён или проверен только вручную, корректный
статус проекта — `in progress`, даже если обычная игра выглядит рабочей.
