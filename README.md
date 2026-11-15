# Ferrustry

**Reforge the Industry.** Industrial strategy, reengineered in Rust.

Ferrustry — оригинальная open-source игра жанра «автоматизация производства + защита базы»
на Rust: собственный движок, headless-симуляция с фиксированным шагом, выделенный сервер
и упор на измеримую производительность больших баз.

> Статус: **этап 1 — headless-ядро симуляции.** Графического клиента пока нет.

## Быстрый старт

```bash
cargo run --release -p ferrustry-headless -- run
```

Запускает вертикальный срез: буры → конвейер → плавильня на ветряках → конвейер → ядро.

## Структура

| Крейт | Назначение |
|---|---|
| `ferrustry-content` | Схема контента (RON), валидация, реестр |
| `ferrustry-sim` | Детерминированная симуляция: мир, постройки, конвейеры, энергия, производство |
| `ferrustry-format` | Формат сохранений v1.0 |
| `ferrustry-headless` | Прогон сценариев, бенчмарки, проверка сохранений |

## Документация

- [Разработка](docs/development/README.md)

- [Архитектура](docs/architecture.md)
- [Анализ жанра](docs/genre-analysis.md)
- [Совместимость](docs/compatibility.md)
- [Roadmap и backlog](docs/roadmap.md)
- [Производительность](docs/performance.md)
- [Лицензирование](docs/licensing.md)
- [Решения на согласование](docs/decisions.md)

## Лицензия

Код — MIT OR Apache-2.0 на выбор ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
Политика для ассетов и бренда — [docs/licensing.md](docs/licensing.md).
