# Pumpkin: внутренняя документация

Эта папка — рабочий справочник по текущей копии Pumpkin. Он предназначен для
разработчика, который добавляет новую механику и хочет сначала понять, где
хранится состояние, кто запускает тик, как отправляется пакет и где находится
vanilla-правило. Документация описывает **состояние исходного дерева на момент
последнего обновления**, поэтому при изменении кода нужно обновлять и этот
справочник.

## С чего начинать

| Задача | Документ |
|---|---|
| Понять границы crates и жизненный цикл сервера | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Найти правильный файл для новой функции | [MODULE_CATALOG.md](MODULE_CATALOG.md) |
| Добавить блок, предмет, сущность, пакет, команду или worldgen | [EXTENDING.md](EXTENDING.md) |
| Проверить, какие данные проходят между слоями | [DATA_FLOWS.md](DATA_FLOWS.md) |
| Сравнить поведение с vanilla и увидеть оставшиеся пробелы | [VANILLA_PARITY.md](VANILLA_PARITY.md) |
| Получить приоритизированный backlog | [TODO.md](TODO.md) |
| Последовательно довести весь сервер до доказуемой vanilla-parity | [FULL_IMPLEMENTATION_PLAN.md](FULL_IMPLEMENTATION_PLAN.md) |
| Посмотреть состав публикуемого checkpoint и честный список незакрытого | [INTERMEDIATE_RELEASE.md](INTERMEDIATE_RELEASE.md) |

## Версии и источники истины

- Java protocol/game target: `26.2`.
- Bedrock target: `1.26.40`, protocol `2168`.
- Текущая версия задаётся в `crates/pumpkin-world/src/lib.rs` и также
  используется data/codegen-слоем.
- Декомпилированный Mojang server находится в `Minecraft/26.1/decompiled_src`
  и `Minecraft/decompiled_src`. Это **read-only reference**, не часть сборки и
  не должен попадать в коммиты.
- Для protocol/data parity сначала проверяйте локальный decompile, затем
  generated data и только после этого runtime-код Pumpkin.

## Размер и границы текущего дерева

На момент составления справочника в workspace было примерно 1,900 Rust-файлов:
около 182k строк runtime `pumpkin`, 69k строк `pumpkin-world`, 26k строк
protocol и 1.4M строк generated `pumpkin-data`. Полный физический список файлов
не дублируется в markdown: его можно получить командой
`rg --files crates tools -g '*.rs' | sort`; смысловой индекс находится в
[MODULE_CATALOG.md](MODULE_CATALOG.md). Generated data намеренно описывается как
один слой, потому что поиск поведения в миллионах строк таблиц вместо builder-а
почти всегда приводит к ошибочному патчу.

## Правило чтения документации

В таблицах путь указывает на реализацию, а имя vanilla-класса — на правило,
которое нужно сверять. Если написано `partial`, это значит, что код существует,
но ещё не покрывает полный vanilla contract; такую функцию нельзя считать
готовой только потому, что она компилируется.

## Быстрый цикл изменения

1. Найдите subsystem в `MODULE_CATALOG.md`.
2. Проверьте контракт и поток данных в `ARCHITECTURE.md` и `DATA_FLOWS.md`.
3. Сверьте vanilla reference и ограничения в `VANILLA_PARITY.md`.
4. Реализуйте минимальный слой: data → registry → runtime → packet/persistence.
5. Добавьте unit-тест на чистое правило и integration/smoke-тест на границу.
6. Запустите `cargo fmt --all`, `cargo check -p pumpkin --lib`, целевые тесты и
   `git diff --check`.
7. Обновите parity-таблицу и backlog, если поведение всё ещё отличается.
