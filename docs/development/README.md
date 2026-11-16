# Разработка

## Требования
- Rust stable (проверено на 1.97.0), `rustfmt`, `clippy`.

## Проверки перед коммитом
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
