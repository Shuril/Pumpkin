# Как добавлять новую функцию

Ниже приведён практический маршрут. Перед изменением конкретного subsystem
откройте [MODULE_CATALOG.md](MODULE_CATALOG.md) и [VANILLA_PARITY.md](VANILLA_PARITY.md).

## Общий шаблон

```text
vanilla reference
    ↓
generated data / protocol contract
    ↓
registry registration
    ↓
runtime state + behavior
    ↓
client packets / NBT persistence
    ↓
unit + integration + parity test
```

Если один слой пропущен, функция обычно «работает локально», но не работает
после перезапуска, у второго издания, у клиента или при загрузке чанка.

## Новый блок

1. Проверьте, что блок и все его states есть в `pumpkin-data`.
2. Для простой реакции создайте `block/blocks/name.rs` и реализуйте нужные
   методы базового block trait из `block/mod.rs`.
3. Для состояния/инвентаря создайте `block/entities/name.rs`, реализуйте
   `BlockEntity`, `Inventory`/`NBTStorage`, добавьте конструктор в
   `block/entities/mod.rs` и связь в `block/registry.rs`.
4. Для redstone используйте `get_weak_redstone_power`,
   `get_strong_redstone_power`, уведомления соседей и scheduler; не вызывайте
   рекурсивный `set_block_state` без ограничения цепочки.
5. Для waterloggable состояния сохраняйте `waterlogged` при placement,
   fluid tick, rotation/mirror и замене блока.
6. Проверьте placement, break/drop, neighbor update, comparator output,
   scheduled tick, NBT, client block update и chunk reload.

## Новый предмет/поведение dispenser

1. Добавьте item behavior в `item/items/name.rs`.
2. Зарегистрируйте metadata в `item/items/mod.rs`/`item/registry.rs`.
3. Разделяйте player-use и dispenser-use: dispenser не имеет `Player`, поэтому
   поведение должно работать через `DispenseContext`, world и entity target.
4. На успешный use расходуйте stack только после проверки результата и учитывайте
   creative mode.
5. Для item state replacement пользуйтесь `state_with_properties_of`, чтобы не
   терять facing, slab type, waterlogged, lit/powered и аналогичные свойства.
6. Добавьте тесты на success/failure, расход, drop fallback и сохранение state.

## Новая сущность/моб

1. Создайте базовую структуру в `entity/`, выберите базовые `Entity`,
   `LivingEntity`, projectile или vehicle.
2. Добавьте `EntityType`/tracked-data в generated data, если ID ещё отсутствует.
3. Реализуйте `EntityBase`, NBT load/save, spawn packet для Java и Bedrock,
   metadata, collision box, tick и removal reason.
4. Зарегистрируйте factory в `entity/type.rs`/`entity/registry` и spawn egg,
   если он нужен.
5. Для AI используйте `entity/ai/goal`; target selector, LOS, reach и tick
   cooldown должны соответствовать vanilla, а не только «видимому» поведению.
6. Проверьте despawn/unload/reload, passengers/vehicles, cross-dimension и
   unknown entity id в incoming packets.

## Новый packet или protocol feature

1. Сверьте packet ID, fields, optionality, ordering, bounds и version gates с
   `Minecraft/.../protocol` и tracked data.
2. Java packet type обычно находится в `pumpkin-protocol/src/java`; Bedrock — в
   `src/bedrock`. Serializer/deserializer лежат в `serial/`, `ser/`, `codec/`.
3. Добавьте packet mapping/handler в соответствующую `pumpkin/src/net/...` с
   проверкой connection state и permission.
4. Не смешивайте Java endian/VarInt с Bedrock little-endian/VarUInt.
5. Добавьте round-trip codec test и handler test на malformed/truncated input.

## Новая команда

1. Создайте `pumpkin/src/command/commands/name.rs`.
2. Используйте typed arguments из `command/args` и `argument_types`, а не ручной
   split строки.
3. Зарегистрируйте дерево в `command/commands/mod.rs`/`command/mod.rs`.
4. Проверьте permission/op level, Java/Bedrock feedback, selector semantics,
   suggestions, error cursor и world/dimension context.

## Новый worldgen feature/structure

1. Сначала определите vanilla random stream: seed derivation, salt, positional
   random, fork order и RNG consumption.
2. Для feature используйте `generation/feature`; для structure —
   `generation/structure`, template/processor и generated structure data.
3. Не меняйте порядок placement modifiers и не материализуйте lazy stream раньше
   времени: соседние features могут зависеть от текущего состояния.
4. Учитывайте `Dimension`, min/max Y, ocean-floor heightmap, carving, fluid
   ticks, block entities и rotation/mirror.
5. Добавьте fixed-seed test и smoke test на generated chunk/structure graph.

## Новый config option

1. Добавьте поле в `pumpkin-config`, serde default и validation.
2. Документируйте default и отличия от vanilla.
3. Протяните значение до одного runtime owner; не читайте TOML из tick-кода.
4. Проверьте Java/Bedrock и изменение значения при reload, если reload
   поддерживается.

## Новый plugin API surface

WIT schema → generated bindings → host implementation → permission → plugin API
re-export → example/test plugin. Версии WIT нельзя менять молча: добавляйте
совместимый тип либо поднимайте API version.

## Минимальный набор проверок

```bash
cargo fmt --all
cargo check -p pumpkin --lib
cargo test -p pumpkin --lib
cargo test -p pumpkin-world --lib
git diff --check
```

Для data/protocol изменений добавьте соответствующий crate test; для worldgen
запускайте fixed-seed tests; для сетевых изменений используйте malformed packet
tests и обе edition paths.
