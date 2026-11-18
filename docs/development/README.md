# Разработка

## Требования
- Rust stable (проверено на 1.97.0), `rustfmt`, `clippy`.

Что и как проверяют тесты, e2e, рендер и замеры производительности — в [testing.md](testing.md).
Формат данных контента — в [content-format.md](../content-format.md).

## Проверки перед коммитом

Одной командой (останавливается на первой ошибке): `scripts/check.sh`. Запускать после
последней правки, непосредственно перед коммитом — CI 7bf08cd упал на форматировании
из-за правки, внесённой после проверок.

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

## Headless
```bash
cargo run --release -p ferrustry-headless -- run --ticks 3000 --save slice.fsave
cargo run --release -p ferrustry-headless -- resume slice.fsave --ticks 1000
cargo run --release -p ferrustry-headless -- save-check
FERRUSTRY_COMMIT=$(git rev-parse --short HEAD) cargo run --release -p ferrustry-headless -- bench b2 --conveyors 10000 --out bench.json
cargo bench -p ferrustry-sim
scripts/bench-suite.sh          # полный набор с медианами (Windows: scripts/bench-suite.ps1)
```

## Требования к CPU
На x86_64 сборка нацелена на AVX2 (x86-64-v3, `.cargo/config.toml`, решение D-14). На ARM
(Apple Silicon) используется базовый уровень aarch64 (NEON всегда есть).

## Правила кода
- Симуляция (`ferrustry-sim`) не зависит от графики, сети, времени и потоков; меняется только
  через `Command` и `World::tick`/`step`.
- Никаких `HashMap`-итераций, `thread_rng`, системного времени и float-трансцендентных функций
  в логике, влияющей на состояние.
- Любая новая система — с тестом на сохранение ресурсов (`assert_conserved`) и `World::validate`.
- `unwrap`/`expect` в runtime-коде запрещены lint'ом; в тестах разрешены.
- Документация — часть изменения: новая механика описывается в `docs/player-guide.md`, изменение
  данных — перегенерацией `docs/content-reference.md`, заметное изменение — строкой в `CHANGELOG.md`,
  замеры — в `docs/performance.md`. Обновляются **оба языка**: русский в `docs/`, английский в
  `docs/en/` (см. «Два языка» ниже). Тесты в `ferrustry-content` ловят отставание справочника и
  руководств от данных.

## Два языка

Документация есть на русском (`docs/`, `README.md`, `CHANGELOG.md`) и английском (`docs/en/`,
`README.en.md`, `CHANGELOG.en.md`) с одинаковыми именами файлов и разделами. Тест
`english_docs_mirror_russian` в `ferrustry-content` падает, если документ есть только на одном языке
или если у версий разное число разделов. Проверить, что текст говорит одно и то же, он не может —
это задача ревью. Оба справочника контента генерируются из одних данных.

## Клиент

```bash
cargo run --release -p ferrustry-client                         # окно
cargo run --release -p ferrustry-client -- --screenshot shot.png --ticks 600 --zoom 40 --center 96,61
cargo run --release -p ferrustry-client -- --frames 120         # дымовой тест окна
```

Linux: нужны `libvulkan1` (или Mesa) и `libxkbcommon-x11-0`. Без GPU работает программный
Vulkan (`mesa-vulkan-drivers`, lavapipe) — так рендер проверяется в CI.

Тест `ferrustry-render/tests/offscreen.rs` рендерит кадр на реальном GPU и проверяет пиксели
(цвета бренда, эмблему ядра); при отсутствии адаптера пропускается с сообщением `SKIPPED`.
