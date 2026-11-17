//! Справочник контента в Markdown, генерируемый из реестра.
//!
//! Файл `docs/content-reference.md` не пишется руками: тест `reference_doc_is_up_to_date`
//! сравнивает его с выводом [`ContentRegistry::reference_markdown`] и падает при расхождении.
//! Обновить: `FERRUSTRY_BLESS=1 cargo test -p ferrustry-content reference`.

use std::fmt::Write as _;

use crate::{BlockDef, BlockKind, ContentRegistry, ItemStack};

/// Тиков в секунду (D-04). Только для перевода в секунды в справочнике.
const HZ: f64 = 30.0;
/// Единиц пути на тайл и шаг между предметами на ленте (см. `ferrustry-sim::segment`).
const TILE_LEN: f64 = 256.0;
const ITEM_SPACING: f64 = 64.0;

fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.replace('.', ",")
}

impl ContentRegistry {
    fn stacks(&self, s: &[ItemStack]) -> String {
        if s.is_empty() {
            return "—".into();
        }
        s.iter()
            .map(|st| format!("{} `{}`", st.count, self.item(st.item).name))
            .collect::<Vec<_>>()
            .join(" + ")
    }

    fn category(kind: &BlockKind) -> &'static str {
        match kind {
            BlockKind::Conveyor { .. }
            | BlockKind::Splitter
            | BlockKind::Sorter
            | BlockKind::Bridge { .. }
            | BlockKind::Depot { .. } => "Логистика",
            BlockKind::Drill { .. } | BlockKind::Crafter { .. } | BlockKind::Core { .. } => {
                "Производство"
            }
            BlockKind::Turret { .. } | BlockKind::ArcTurret { .. } | BlockKind::Wall => "Оборона",
            BlockKind::Generator { .. }
            | BlockKind::PowerNode { .. }
            | BlockKind::Battery { .. }
            | BlockKind::HeatEngine { .. } => "Энергия",
            BlockKind::Pipe { .. }
            | BlockKind::Condenser { .. }
            | BlockKind::CoolingTower { .. }
            | BlockKind::HeatPipe { .. }
            | BlockKind::Radiator { .. } => "Жидкости и тепло",
        }
    }

    /// Человекочитаемое описание поведения постройки с числами.
    fn describe(&self, b: &BlockDef) -> String {
        match &b.kind {
            BlockKind::Conveyor { speed } => {
                let s = f64::from(*speed);
                format!(
                    "{} тайла/с, до {} предметов/с",
                    num(s * HZ / TILE_LEN),
                    num(s * HZ / ITEM_SPACING)
                )
            }
            BlockKind::Drill { base_ticks, buffer } => {
                let area = f64::from(b.size) * f64::from(b.size);
                format!(
                    "руда под всеми тайлами ({area}): {} предмета/с; медленнее пропорционально покрытию; буфер {buffer}",
                    num(HZ / f64::from(*base_ticks))
                )
            }
            BlockKind::Crafter {
                inputs,
                outputs,
                craft_ticks,
                power_use,
                buffer,
                heat,
                cooling,
                ..
            } => {
                let mut s = format!(
                    "{} → {} за {} с; энергия {power_use}/тик; буфер {buffer}",
                    self.stacks(inputs),
                    self.stacks(outputs),
                    num(f64::from(*craft_ticks) / HZ)
                );
                if *heat > 0 {
                    let _ = write!(
                        s,
                        "; тепло +{heat}/тик в работе, пассивное охлаждение {cooling}/тик"
                    );
                }
                s
            }
            BlockKind::Generator {
                power, fuel: None, ..
            } => format!("+{power} энергии/тик всегда"),
            BlockKind::Generator {
                power,
                fuel: Some(f),
                burn_ticks,
                buffer,
            } => format!(
                "+{power} энергии/тик, пока горит `{}` (1 шт. = {} с); буфер топлива {buffer}",
                self.item(*f).name,
                num(f64::from(*burn_ticks) / HZ)
            ),
            BlockKind::PowerNode { range } => {
                format!("соединяет энергопостройки в радиусе {range} тайлов")
            }
            BlockKind::Battery { capacity } => format!("хранит {capacity} энергии"),
            BlockKind::Core { capacity } => {
                format!(
                    "хранилище игрока: до {capacity} каждого предмета; разрушение последнего — поражение"
                )
            }
            BlockKind::Splitter => "раздаёт предметы по кругу на три другие стороны".into(),
            BlockKind::Sorter => {
                "предметы фильтра идут прямо, остальные — вбок (поочерёдно)".into()
            }
            BlockKind::Bridge {
                range,
                interval,
                buffer,
            } => format!(
                "перебрасывает предметы до {range} тайлов к следующему мосту; {} предмета/с; буфер {buffer}",
                num(HZ / f64::from(*interval))
            ),
            BlockKind::Depot { capacity } => format!(
                "склад на {capacity} предметов: принимает с трёх сторон, выдаёт вперёд до {} предметов/с",
                num(HZ)
            ),
            BlockKind::Turret {
                range,
                damage,
                reload,
                ammo,
                shots_per_item,
                projectile_speed,
                buffer,
            } => format!(
                "радиус {range}, урон {damage} за выстрел, {} выстр./с ({} урона/с); боеприпас `{}` — {shots_per_item} выстр. за штуку, буфер {buffer} шт.; снаряд {} тайла/с",
                num(HZ / f64::from(*reload)),
                num(f64::from(*damage) * HZ / f64::from(*reload)),
                self.item(*ammo).name,
                num(f64::from(*projectile_speed) * HZ / TILE_LEN)
            ),
            BlockKind::ArcTurret {
                range,
                damage,
                reload,
                power_use,
            } => format!(
                "радиус {range}, разряд {damage} мгновенно, **игнорирует броню**; при полном питании {} разр./с ({} урона/с); энергия {power_use}/тик во время перезарядки",
                num(HZ / f64::from(*reload)),
                num(f64::from(*damage) * HZ / f64::from(*reload))
            ),
            BlockKind::Wall => {
                "только прочность; враги предпочитают обходить постройки с большой прочностью"
                    .into()
            }
            BlockKind::Pipe { capacity } => {
                format!("объём {capacity}; соединяет жидкостные постройки")
            }
            BlockKind::Condenser {
                fluid,
                rate,
                power_use,
                capacity,
            } => format!(
                "делает `{}` {rate}/тик, энергия {power_use}/тик; объём {capacity}",
                self.fluid(*fluid).name
            ),
            BlockKind::CoolingTower {
                fluid,
                rate,
                cooling,
                heat_capacity,
            } => format!(
                "расходует `{}` {rate}/тик и снимает {} тепла/тик; теплоёмкость {heat_capacity}",
                self.fluid(*fluid).name,
                rate * cooling
            ),
            BlockKind::HeatPipe { heat_capacity } => {
                format!("соединяет тепловые постройки; теплоёмкость {heat_capacity}")
            }
            BlockKind::Radiator {
                dissipation,
                heat_capacity,
            } => format!("рассеивает {dissipation} тепла/тик; теплоёмкость {heat_capacity}"),
            BlockKind::HeatEngine {
                max_heat,
                power_per_heat_permille,
                heat_capacity,
            } => format!(
                "забирает до {max_heat} тепла/тик и даёт {}% от него энергией (со следующего тика); теплоёмкость {heat_capacity}",
                power_per_heat_permille / 10
            ),
        }
    }

    /// Справочник контента в Markdown (детерминированно: порядок как в файле контента).
    pub fn reference_markdown(&self) -> String {
        let mut s = String::new();
        let w = &mut s;
        let _ = writeln!(w, "# Справочник контента");
        let _ = writeln!(w);
        let _ = writeln!(
            w,
            "> Файл сгенерирован из `crates/ferrustry-content/data/base.ron` — не редактируйте руками.\n\
             > Обновить: `FERRUSTRY_BLESS=1 cargo test -p ferrustry-content reference`.\n\
             > Время — при 30 тиках/с. Описание механик — в [руководстве игрока](player-guide.md)."
        );

        let _ = writeln!(w, "\n## Предметы и жидкости\n");
        let _ = writeln!(w, "| Предмет | Добывается из |\n|---|---|");
        for (i, it) in self.items().iter().enumerate() {
            let ores: Vec<String> = self
                .ores()
                .iter()
                .filter(|o| usize::from(o.item.0) == i)
                .map(|o| format!("`{}`", o.name))
                .collect();
            let src = if ores.is_empty() {
                "производится".to_owned()
            } else {
                ores.join(", ")
            };
            let _ = writeln!(w, "| `{}` | {src} |", it.name);
        }
        let fluids: Vec<String> = self
            .fluids()
            .iter()
            .map(|f| format!("`{}`", f.name))
            .collect();
        let _ = writeln!(w, "\nЖидкости: {}.", fluids.join(", "));
        let floors: Vec<String> = self
            .floors()
            .iter()
            .map(|f| {
                format!(
                    "`{}`{}",
                    f.name,
                    if f.buildable {
                        ""
                    } else {
                        " (нельзя строить)"
                    }
                )
            })
            .collect();
        let _ = writeln!(w, "Поверхности: {}.", floors.join(", "));

        let _ = writeln!(w, "\n## Постройки");
        for cat in [
            "Логистика",
            "Производство",
            "Энергия",
            "Жидкости и тепло",
            "Оборона",
        ] {
            let _ = writeln!(w, "\n### {cat}\n");
            let _ = writeln!(
                w,
                "| Постройка | Размер | Стоимость | Прочность | Что делает |\n|---|---|---|---|---|"
            );
            for b in self
                .blocks()
                .iter()
                .filter(|b| Self::category(&b.kind) == cat)
            {
                let _ = writeln!(
                    w,
                    "| `{}` | {s}×{s} | {} | {} | {} |",
                    b.name,
                    self.stacks(&b.cost),
                    b.health,
                    self.describe(b),
                    s = b.size
                );
            }
        }

        let _ = writeln!(w, "\n## Враги\n");
        let _ = writeln!(
            w,
            "| Враг | Прочность | Броня | Скорость, тайл/с | Атака | Урон/с |\n|---|---|---|---|---|---|"
        );
        for u in self.units() {
            let attack = if u.range > 0 {
                format!(
                    "снаряд {} с {} тайлов, раз в {} с",
                    u.damage,
                    u.range,
                    num(f64::from(u.cooldown) / HZ)
                )
            } else {
                format!(
                    "ближний бой {}, раз в {} с",
                    u.damage,
                    num(f64::from(u.cooldown) / HZ)
                )
            };
            let _ = writeln!(
                w,
                "| `{}` | {} | {} | {} | {attack} | {} |",
                u.name,
                u.health,
                u.armor,
                num(f64::from(u.speed) * HZ / TILE_LEN),
                num(f64::from(u.damage) * HZ / f64::from(u.cooldown))
            );
        }
        let _ = writeln!(
            w,
            "\nБроня вычитается из урона снаряда, но урон не опускается ниже трети исходного. \
             Разряд дуговой турели броню игнорирует."
        );
        s
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::ContentRegistry;

    #[test]
    fn reference_doc_is_up_to_date() {
        let reg = ContentRegistry::base().unwrap();
        let fresh = reg.reference_markdown();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/content-reference.md");
        if std::env::var_os("FERRUSTRY_BLESS").is_some() {
            std::fs::write(&path, &fresh).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == fresh,
            "docs/content-reference.md is stale: run \
             `FERRUSTRY_BLESS=1 cargo test -p ferrustry-content reference`"
        );
    }

    /// Руководства не должны отставать от данных: каждый враг описан в руководстве игрока,
    /// каждый вид постройки из `base.ron` — в описании формата.
    #[test]
    fn guides_cover_all_content() {
        let reg = ContentRegistry::base().unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
        let guide = std::fs::read_to_string(root.join("player-guide.md")).unwrap();
        for u in reg.units() {
            assert!(
                guide.contains(&format!("`{}`", u.name)),
                "docs/player-guide.md does not describe enemy `{}`",
                u.name
            );
        }
        let format = std::fs::read_to_string(root.join("content-format.md")).unwrap();
        let data = include_str!("../data/base.ron");
        for kind in data.split("kind: ").skip(1) {
            let name: String = kind
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            assert!(
                format.contains(&format!("| `{name}` |")),
                "docs/content-format.md does not describe kind `{name}`"
            );
        }
    }

    #[test]
    fn reference_mentions_every_block_and_unit() {
        let reg = ContentRegistry::base().unwrap();
        let doc = reg.reference_markdown();
        for name in reg
            .blocks()
            .iter()
            .map(|b| &b.name)
            .chain(reg.units().iter().map(|u| &u.name))
        {
            assert!(doc.contains(&format!("`{name}`")), "{name} missing");
        }
    }
}
