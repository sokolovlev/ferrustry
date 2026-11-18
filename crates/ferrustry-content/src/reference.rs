//! Справочник контента в Markdown, генерируемый из реестра, на двух языках.
//!
//! Файлы `docs/content-reference.md` (русский) и `docs/en/content-reference.md` (английский) не
//! пишутся руками: тест `reference_doc_is_up_to_date` сравнивает их с выводом
//! [`ContentRegistry::reference_markdown`] и падает при расхождении.
//! Обновить: `FERRUSTRY_BLESS=1 cargo test -p ferrustry-content reference`.

use std::fmt::Write as _;

use crate::{BlockDef, BlockKind, ContentRegistry, ItemStack};

/// Язык документации.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocLang {
    /// Русский (`docs/`).
    Ru,
    /// Английский (`docs/en/`).
    En,
}

/// Тиков в секунду (D-04). Только для перевода в секунды в справочнике.
const HZ: f64 = 30.0;
/// Единиц пути на тайл и шаг между предметами на ленте (см. `ferrustry-sim::segment`).
const TILE_LEN: f64 = 256.0;
const ITEM_SPACING: f64 = 64.0;

const CATEGORIES: [(&str, &str); 5] = [
    ("Логистика", "Logistics"),
    ("Производство", "Production"),
    ("Энергия", "Power"),
    ("Жидкости и тепло", "Fluids and heat"),
    ("Оборона", "Defense"),
];

/// Число с не более чем двумя знаками после запятой, в принятой для языка записи.
fn num(lang: DocLang, v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    match lang {
        DocLang::Ru => s.replace('.', ","),
        DocLang::En => s.to_owned(),
    }
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

    /// Индекс категории в [`CATEGORIES`].
    fn category(kind: &BlockKind) -> usize {
        match kind {
            BlockKind::Conveyor { .. }
            | BlockKind::Splitter
            | BlockKind::Sorter
            | BlockKind::Bridge { .. }
            | BlockKind::Depot { .. } => 0,
            BlockKind::Drill { .. } | BlockKind::Crafter { .. } | BlockKind::Core { .. } => 1,
            BlockKind::Generator { .. }
            | BlockKind::PowerNode { .. }
            | BlockKind::Battery { .. }
            | BlockKind::HeatEngine { .. } => 2,
            BlockKind::Pipe { .. }
            | BlockKind::Condenser { .. }
            | BlockKind::CoolingTower { .. }
            | BlockKind::HeatPipe { .. }
            | BlockKind::Radiator { .. } => 3,
            BlockKind::Turret { .. } | BlockKind::ArcTurret { .. } | BlockKind::Wall => 4,
        }
    }

    /// Человекочитаемое описание поведения постройки с числами.
    fn describe(&self, lang: DocLang, b: &BlockDef) -> String {
        let en = lang == DocLang::En;
        let n = |v: f64| num(lang, v);
        let sec = |ticks: u32| n(f64::from(ticks) / HZ);
        let per_s = |ticks: u32| n(HZ / f64::from(ticks));
        match &b.kind {
            BlockKind::Conveyor { speed } => {
                let s = f64::from(*speed);
                let (tiles, items) = (n(s * HZ / TILE_LEN), n(s * HZ / ITEM_SPACING));
                if en {
                    format!("{tiles} tiles/s, up to {items} items/s")
                } else {
                    format!("{tiles} тайла/с, до {items} предметов/с")
                }
            }
            BlockKind::Drill { base_ticks, buffer } => {
                let area = u32::from(b.size) * u32::from(b.size);
                let rate = per_s(*base_ticks);
                if en {
                    format!(
                        "ore under all tiles ({area}): {rate} items/s; proportionally slower with less ore; buffer {buffer}"
                    )
                } else {
                    format!(
                        "руда под всеми тайлами ({area}): {rate} предмета/с; медленнее пропорционально покрытию; буфер {buffer}"
                    )
                }
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
                let (i, o, t) = (self.stacks(inputs), self.stacks(outputs), sec(*craft_ticks));
                let mut s = if en {
                    format!("{i} → {o} in {t} s; power {power_use}/tick; buffer {buffer}")
                } else {
                    format!("{i} → {o} за {t} с; энергия {power_use}/тик; буфер {buffer}")
                };
                if *heat > 0 {
                    let _ = if en {
                        write!(
                            s,
                            "; heat +{heat}/tick while working, passive cooling {cooling}/tick"
                        )
                    } else {
                        write!(
                            s,
                            "; тепло +{heat}/тик в работе, пассивное охлаждение {cooling}/тик"
                        )
                    };
                }
                s
            }
            BlockKind::Generator {
                power, fuel: None, ..
            } => {
                if en {
                    format!("+{power} power/tick, always")
                } else {
                    format!("+{power} энергии/тик всегда")
                }
            }
            BlockKind::Generator {
                power,
                fuel: Some(f),
                burn_ticks,
                buffer,
            } => {
                let (fuel, t) = (&self.item(*f).name, sec(*burn_ticks));
                if en {
                    format!(
                        "+{power} power/tick while `{fuel}` burns (1 item = {t} s); fuel buffer {buffer}"
                    )
                } else {
                    format!(
                        "+{power} энергии/тик, пока горит `{fuel}` (1 шт. = {t} с); буфер топлива {buffer}"
                    )
                }
            }
            BlockKind::PowerNode { range } => {
                if en {
                    format!("links power buildings within {range} tiles")
                } else {
                    format!("соединяет энергопостройки в радиусе {range} тайлов")
                }
            }
            BlockKind::Battery { capacity } => {
                if en {
                    format!("stores {capacity} power")
                } else {
                    format!("хранит {capacity} энергии")
                }
            }
            BlockKind::Core { capacity } => {
                if en {
                    format!(
                        "player storage: up to {capacity} of each item; losing the last one loses the game"
                    )
                } else {
                    format!(
                        "хранилище игрока: до {capacity} каждого предмета; разрушение последнего — поражение"
                    )
                }
            }
            BlockKind::Splitter => if en {
                "hands out items to its three other sides in turn"
            } else {
                "раздаёт предметы по кругу на три другие стороны"
            }
            .into(),
            BlockKind::Sorter => if en {
                "filter items go straight, the rest alternate sideways"
            } else {
                "предметы фильтра идут прямо, остальные — вбок (поочерёдно)"
            }
            .into(),
            BlockKind::Bridge {
                range,
                interval,
                buffer,
            } => {
                let rate = per_s(*interval);
                if en {
                    format!(
                        "throws items up to {range} tiles to the next bridge; {rate} items/s; buffer {buffer}"
                    )
                } else {
                    format!(
                        "перебрасывает предметы до {range} тайлов к следующему мосту; {rate} предмета/с; буфер {buffer}"
                    )
                }
            }
            BlockKind::Depot { capacity } => {
                let rate = n(HZ);
                if en {
                    format!(
                        "stores {capacity} items: accepts from three sides, outputs forward up to {rate} items/s"
                    )
                } else {
                    format!(
                        "склад на {capacity} предметов: принимает с трёх сторон, выдаёт вперёд до {rate} предметов/с"
                    )
                }
            }
            BlockKind::Turret {
                range,
                damage,
                reload,
                ammo,
                shots_per_item,
                projectile_speed,
                buffer,
            } => {
                let shots = per_s(*reload);
                let dps = n(f64::from(*damage) * HZ / f64::from(*reload));
                let ammo = &self.item(*ammo).name;
                let speed = n(f64::from(*projectile_speed) * HZ / TILE_LEN);
                if en {
                    format!(
                        "range {range}, {damage} damage per shot, {shots} shots/s ({dps} damage/s); ammo `{ammo}` — {shots_per_item} shots per item, buffer {buffer} items; projectile {speed} tiles/s"
                    )
                } else {
                    format!(
                        "радиус {range}, урон {damage} за выстрел, {shots} выстр./с ({dps} урона/с); боеприпас `{ammo}` — {shots_per_item} выстр. за штуку, буфер {buffer} шт.; снаряд {speed} тайла/с"
                    )
                }
            }
            BlockKind::ArcTurret {
                range,
                damage,
                reload,
                power_use,
            } => {
                let shots = per_s(*reload);
                let dps = n(f64::from(*damage) * HZ / f64::from(*reload));
                if en {
                    format!(
                        "range {range}, instant {damage} damage discharge, **ignores armor**; at full power {shots} discharges/s ({dps} damage/s); power {power_use}/tick while recharging"
                    )
                } else {
                    format!(
                        "радиус {range}, разряд {damage} мгновенно, **игнорирует броню**; при полном питании {shots} разр./с ({dps} урона/с); энергия {power_use}/тик во время перезарядки"
                    )
                }
            }
            BlockKind::Wall => if en {
                "health only; enemies prefer to go around high-health buildings"
            } else {
                "только прочность; враги предпочитают обходить постройки с большой прочностью"
            }
            .into(),
            BlockKind::Pipe { capacity } => {
                if en {
                    format!("volume {capacity}; connects fluid buildings")
                } else {
                    format!("объём {capacity}; соединяет жидкостные постройки")
                }
            }
            BlockKind::Condenser {
                fluid,
                rate,
                power_use,
                capacity,
            } => {
                let fluid = &self.fluid(*fluid).name;
                if en {
                    format!(
                        "makes `{fluid}` {rate}/tick, power {power_use}/tick; volume {capacity}"
                    )
                } else {
                    format!(
                        "делает `{fluid}` {rate}/тик, энергия {power_use}/тик; объём {capacity}"
                    )
                }
            }
            BlockKind::CoolingTower {
                fluid,
                rate,
                cooling,
                heat_capacity,
            } => {
                let (fluid, removed) = (&self.fluid(*fluid).name, rate * cooling);
                if en {
                    format!(
                        "uses `{fluid}` {rate}/tick and removes {removed} heat/tick; heat capacity {heat_capacity}"
                    )
                } else {
                    format!(
                        "расходует `{fluid}` {rate}/тик и снимает {removed} тепла/тик; теплоёмкость {heat_capacity}"
                    )
                }
            }
            BlockKind::HeatPipe { heat_capacity } => {
                if en {
                    format!("connects heat buildings; heat capacity {heat_capacity}")
                } else {
                    format!("соединяет тепловые постройки; теплоёмкость {heat_capacity}")
                }
            }
            BlockKind::Radiator {
                dissipation,
                heat_capacity,
            } => {
                if en {
                    format!("dissipates {dissipation} heat/tick; heat capacity {heat_capacity}")
                } else {
                    format!("рассеивает {dissipation} тепла/тик; теплоёмкость {heat_capacity}")
                }
            }
            BlockKind::HeatEngine {
                max_heat,
                power_per_heat_permille,
                heat_capacity,
            } => {
                let pct = power_per_heat_permille / 10;
                if en {
                    format!(
                        "takes up to {max_heat} heat/tick and returns {pct}% of it as power (from the next tick); heat capacity {heat_capacity}"
                    )
                } else {
                    format!(
                        "забирает до {max_heat} тепла/тик и даёт {pct}% от него энергией (со следующего тика); теплоёмкость {heat_capacity}"
                    )
                }
            }
        }
    }

    /// Справочник контента в Markdown (детерминированно: порядок как в файле контента).
    pub fn reference_markdown(&self, lang: DocLang) -> String {
        let en = lang == DocLang::En;
        let t = |ru: &'static str, eng: &'static str| if en { eng } else { ru };
        let n = |v: f64| num(lang, v);
        let mut s = String::new();
        let w = &mut s;
        let _ = writeln!(w, "# {}", t("Справочник контента", "Content reference"));
        let _ = writeln!(w);
        let _ = writeln!(
            w,
            "{}",
            t(
                "> Файл сгенерирован из `crates/ferrustry-content/data/base.ron` — не редактируйте руками.\n\
                 > Обновить: `FERRUSTRY_BLESS=1 cargo test -p ferrustry-content reference`.\n\
                 > Время — при 30 тиках/с. Описание механик — в [руководстве игрока](player-guide.md).",
                "> Generated from `crates/ferrustry-content/data/base.ron` — do not edit by hand.\n\
                 > Update: `FERRUSTRY_BLESS=1 cargo test -p ferrustry-content reference`.\n\
                 > Times assume 30 ticks/s. Mechanics are explained in the [player guide](player-guide.md).",
            )
        );

        let _ = writeln!(w, "\n## {}\n", t("Предметы и жидкости", "Items and fluids"));
        let _ = writeln!(
            w,
            "| {} | {} |\n|---|---|",
            t("Предмет", "Item"),
            t("Добывается из", "Mined from")
        );
        for (i, it) in self.items().iter().enumerate() {
            let ores: Vec<String> = self
                .ores()
                .iter()
                .filter(|o| usize::from(o.item.0) == i)
                .map(|o| format!("`{}`", o.name))
                .collect();
            let src = if ores.is_empty() {
                t("производится", "produced").to_owned()
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
        let _ = writeln!(w, "\n{}: {}.", t("Жидкости", "Fluids"), fluids.join(", "));
        let floors: Vec<String> = self
            .floors()
            .iter()
            .map(|f| {
                let note = if f.buildable {
                    ""
                } else {
                    t(" (нельзя строить)", " (not buildable)")
                };
                format!("`{}`{note}", f.name)
            })
            .collect();
        let _ = writeln!(w, "{}: {}.", t("Поверхности", "Floors"), floors.join(", "));

        let _ = writeln!(w, "\n## {}", t("Постройки", "Buildings"));
        for (c, (ru, eng)) in CATEGORIES.iter().enumerate() {
            let _ = writeln!(w, "\n### {}\n", t(ru, eng));
            let _ = writeln!(
                w,
                "{}\n|---|---|---|---|---|",
                t(
                    "| Постройка | Размер | Стоимость | Прочность | Что делает |",
                    "| Building | Size | Cost | Health | What it does |"
                )
            );
            for b in self
                .blocks()
                .iter()
                .filter(|b| Self::category(&b.kind) == c)
            {
                let _ = writeln!(
                    w,
                    "| `{}` | {s}×{s} | {} | {} | {} |",
                    b.name,
                    self.stacks(&b.cost),
                    b.health,
                    self.describe(lang, b),
                    s = b.size
                );
            }
        }

        let _ = writeln!(w, "\n## {}\n", t("Враги", "Enemies"));
        let _ = writeln!(
            w,
            "{}\n|---|---|---|---|---|---|",
            t(
                "| Враг | Прочность | Броня | Скорость, тайл/с | Атака | Урон/с |",
                "| Enemy | Health | Armor | Speed, tiles/s | Attack | Damage/s |"
            )
        );
        for u in self.units() {
            let every = n(f64::from(u.cooldown) / HZ);
            let attack = match (u.range > 0, en) {
                (true, true) => format!(
                    "projectile {} from {} tiles, every {every} s",
                    u.damage, u.range
                ),
                (true, false) => {
                    format!("снаряд {} с {} тайлов, раз в {every} с", u.damage, u.range)
                }
                (false, true) => format!("melee {}, every {every} s", u.damage),
                (false, false) => format!("ближний бой {}, раз в {every} с", u.damage),
            };
            let _ = writeln!(
                w,
                "| `{}` | {} | {} | {} | {attack} | {} |",
                u.name,
                u.health,
                u.armor,
                n(f64::from(u.speed) * HZ / TILE_LEN),
                n(f64::from(u.damage) * HZ / f64::from(u.cooldown))
            );
        }
        let _ = writeln!(
            w,
            "\n{}",
            t(
                "Броня вычитается из урона снаряда, но урон не опускается ниже трети исходного. \
                 Разряд дуговой турели броню игнорирует.",
                "Armor is subtracted from projectile damage, but damage never drops below a third \
                 of the original. The arc tower's discharge ignores armor."
            )
        );

        const SAMPLE: [u32; 4] = [1, 5, 10, 20];
        let _ = writeln!(w, "\n## {}\n", t("Волны", "Waves"));
        let mut head = t(
            "| Враг | С волны | Вначале | Прирост |",
            "| Enemy | From wave | At first | Growth |",
        )
        .to_owned();
        for n in SAMPLE {
            let _ = write!(head, " {} {n} |", t("Волна", "Wave"));
        }
        let _ = writeln!(
            w,
            "{head}\n|---|---|---|---|{}",
            "---|".repeat(SAMPLE.len())
        );
        for r in self.waves() {
            let (g, e) = (r.grow, r.every);
            let growth = match (g, e, en) {
                (0, _, _) => "—".to_owned(),
                (_, 1, true) => format!("+{g} each wave"),
                (_, 1, false) => format!("+{g} каждую волну"),
                (_, _, true) => format!("+{g} every {e} waves"),
                (_, _, false) => {
                    // 2–4, 22–24… волны; 5–20, 25… волн
                    let few = (2..=4).contains(&(e % 10)) && !(12..=14).contains(&(e % 100));
                    format!("+{g} каждые {e} {}", if few { "волны" } else { "волн" })
                }
            };
            let mut row = format!(
                "| `{}` | {} | {} | {growth} |",
                self.unit(r.unit).name,
                r.from_wave,
                r.count
            );
            for n in SAMPLE {
                let _ = write!(row, " {} |", r.count_in(n));
            }
            let _ = writeln!(w, "{row}");
        }
        let mut total = t("| **Всего** | | | |", "| **Total** | | | |").to_owned();
        for n in SAMPLE {
            let sum: u32 = self.waves().iter().map(|r| r.count_in(n)).sum();
            let _ = write!(total, " **{sum}** |");
        }
        let _ = writeln!(w, "{total}");
        s
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::DocLang;
    use crate::ContentRegistry;

    fn docs() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs")
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    #[test]
    fn reference_doc_is_up_to_date() {
        let reg = ContentRegistry::base().unwrap();
        for (lang, path) in [
            (DocLang::Ru, docs().join("content-reference.md")),
            (DocLang::En, docs().join("en/content-reference.md")),
        ] {
            let fresh = reg.reference_markdown(lang);
            if std::env::var_os("FERRUSTRY_BLESS").is_some() {
                std::fs::write(&path, &fresh).unwrap();
                continue;
            }
            // git на Windows может выдать файл с CRLF: сравниваем текст, а не концы строк
            let committed = std::fs::read_to_string(&path)
                .unwrap_or_default()
                .replace("\r\n", "\n");
            assert!(
                committed == fresh,
                "{} is stale: run `FERRUSTRY_BLESS=1 cargo test -p ferrustry-content reference`",
                path.display()
            );
        }
    }

    /// Руководства не должны отставать от данных: каждый враг описан в руководстве игрока,
    /// каждый вид постройки из `base.ron` — в описании формата. На обоих языках.
    #[test]
    fn guides_cover_all_content() {
        let reg = ContentRegistry::base().unwrap();
        let data = include_str!("../data/base.ron");
        for dir in [docs(), docs().join("en")] {
            let guide = read(&dir.join("player-guide.md"));
            for u in reg.units() {
                assert!(
                    guide.contains(&format!("`{}`", u.name)),
                    "{}/player-guide.md does not describe enemy `{}`",
                    dir.display(),
                    u.name
                );
            }
            let format = read(&dir.join("content-format.md"));
            for kind in data.split("kind: ").skip(1) {
                let name: String = kind
                    .chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect();
                assert!(
                    format.contains(&format!("| `{name}` |")),
                    "{}/content-format.md does not describe kind `{name}`",
                    dir.display()
                );
            }
        }
    }

    /// Число разделов (строк, начинающихся с `#`, вне блоков кода).
    fn sections(text: &str) -> usize {
        let mut in_code = false;
        text.lines()
            .filter(|l| {
                if l.starts_with("```") {
                    in_code = !in_code;
                }
                !in_code && l.starts_with('#')
            })
            .count()
    }

    fn markdown_files(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "en") {
                    markdown_files(&p, base, out);
                }
            } else if p.extension().is_some_and(|x| x == "md") {
                out.push(p.strip_prefix(base).unwrap().to_path_buf());
            }
        }
    }

    /// Английская документация зеркалит русскую: те же файлы и то же число разделов.
    /// Смысл текста тест не сравнивает — это задача ревью.
    #[test]
    fn english_docs_mirror_russian() {
        let root = docs().join("..");
        let mut pairs: Vec<(PathBuf, PathBuf)> = vec![
            (root.join("README.md"), root.join("README.en.md")),
            (root.join("CHANGELOG.md"), root.join("CHANGELOG.en.md")),
        ];
        let mut ru = Vec::new();
        markdown_files(&docs(), &docs(), &mut ru);
        let mut en = Vec::new();
        markdown_files(&docs().join("en"), &docs().join("en"), &mut en);
        ru.sort();
        en.sort();
        assert_eq!(ru, en, "docs/ and docs/en/ must contain the same documents");
        pairs.extend(
            ru.iter()
                .map(|p| (docs().join(p), docs().join("en").join(p))),
        );
        for (r, e) in pairs {
            assert_eq!(
                sections(&read(&r)),
                sections(&read(&e)),
                "{} and {} have a different number of sections",
                r.display(),
                e.display()
            );
        }
    }

    #[test]
    fn reference_mentions_every_block_and_unit() {
        let reg = ContentRegistry::base().unwrap();
        for lang in [DocLang::Ru, DocLang::En] {
            let doc = reg.reference_markdown(lang);
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
}
