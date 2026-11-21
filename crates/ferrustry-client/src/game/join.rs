//! Экран «JOIN»: адрес сервера и имя игрока, подключение к сетевой партии.

use ferrustry_render::atlas::palette;
use ferrustry_render::{Atlas, Hud, SpriteTable, rgba};

use super::{Game, Request, Screen};
use crate::ui::{Rect, Ui, text_width};

/// Наибольшая длина адреса и имени.
const MAX_ADDR: usize = 64;
const MAX_NAME: usize = 16;

/// Поля экрана подключения.
#[derive(Debug, Clone)]
pub struct JoinForm {
    /// Адрес сервера.
    pub addr: String,
    /// Имя игрока.
    pub name: String,
    /// Поле с фокусом: 0 — адрес, 1 — имя.
    pub focus: usize,
}

impl Default for JoinForm {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1".into(),
            name: "player".into(),
            focus: 0,
        }
    }
}

impl JoinForm {
    /// Ввод символа в поле с фокусом (лишние символы и сверх длины — отбрасываются).
    pub fn type_char(&mut self, c: char) {
        let (field, max, ok) = if self.focus == 0 {
            (
                &mut self.addr,
                MAX_ADDR,
                c.is_ascii_alphanumeric() || ".:-_[]".contains(c),
            )
        } else {
            (
                &mut self.name,
                MAX_NAME,
                c.is_ascii_alphanumeric() || "-_".contains(c),
            )
        };
        if ok && field.chars().count() < max {
            field.push(c);
        }
    }

    /// Стереть последний символ.
    pub fn backspace(&mut self) {
        if self.focus == 0 {
            self.addr.pop();
        } else {
            self.name.pop();
        }
    }
}

/// Раскладка экрана подключения.
#[derive(Debug, Clone, Copy)]
pub struct JoinLayout {
    /// Поле адреса.
    pub addr: Rect,
    /// Поле имени.
    pub name: Rect,
    /// CONNECT.
    pub connect: Rect,
    /// BACK.
    pub back: Rect,
}

impl Game {
    /// Файл с последним сервером и именем.
    pub fn server_file() -> &'static str {
        "ferrustry-server.txt"
    }

    /// Открыть экран подключения.
    pub fn open_join(&mut self) {
        self.join.focus = 0;
        self.screen = Screen::Join;
    }

    /// Подставить последний сервер и имя (из файла настроек).
    pub fn set_join_defaults(&mut self, addr: &str, name: &str) {
        if !addr.is_empty() {
            self.join.addr = addr.chars().take(MAX_ADDR).collect();
        }
        if !name.is_empty() {
            self.join.name = name.chars().take(MAX_NAME).collect();
        }
    }

    /// Раскладка экрана подключения.
    pub fn join_layout(&self) -> JoinLayout {
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let (w, h) = (480.0 * u, 48.0 * u);
        let x = (vw - w) / 2.0;
        let top = vh * 0.34;
        let r = self.menu_rects(2, top + 200.0 * u);
        JoinLayout {
            addr: [x, top + 30.0 * u, w, h],
            name: [x, top + 120.0 * u, w, h],
            connect: r[0],
            back: r[1],
        }
    }

    /// Подключиться с данными формы.
    pub fn join_connect(&mut self) {
        let (addr, name) = (
            self.join.addr.trim().to_owned(),
            self.join.name.trim().to_owned(),
        );
        if addr.is_empty() {
            self.notify("ENTER THE SERVER ADDRESS");
            return;
        }
        let name = if name.is_empty() {
            "player".to_owned()
        } else {
            name
        };
        self.requests.push(Request::RememberServer {
            addr: addr.clone(),
            name: name.clone(),
        });
        self.connect(&addr, &name);
    }

    pub(super) fn join_screen(
        &mut self,
        ui: &mut Ui,
        atlas: &Atlas,
        table: &SpriteTable,
        hud: &mut Hud,
    ) {
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let white = table.white;
        hud.rect(atlas, white, [0.0, 0.0, vw, vh], [0.0, 0.0, 0.0, 0.93]);
        let l = self.join_layout();
        let t = "JOIN A GAME";
        hud.text(
            atlas,
            (vw - text_width(t, 5.0 * u)) / 2.0,
            l.addr[1] - 150.0 * u,
            5.0 * u,
            t,
            rgba(palette::FORGE_ORANGE),
        );
        let hint = "TAB: NEXT FIELD   ENTER: CONNECT   DEFAULT PORT 27960";
        hud.text(
            atlas,
            (vw - text_width(hint, 2.0 * u)) / 2.0,
            l.addr[1] - 80.0 * u,
            2.0 * u,
            hint,
            rgba(palette::TITANIUM),
        );
        let (addr, name, focus) = (
            self.join.addr.clone(),
            self.join.name.clone(),
            self.join.focus,
        );
        if ui.text_field(
            hud,
            atlas,
            white,
            l.addr,
            "SERVER ADDRESS",
            &addr,
            focus == 0,
            2.0 * u,
        ) {
            self.join.focus = 0;
        }
        if ui.text_field(
            hud,
            atlas,
            white,
            l.name,
            "YOUR NAME",
            &name,
            focus == 1,
            2.0 * u,
        ) {
            self.join.focus = 1;
        }
        if ui.button(hud, atlas, white, l.connect, "CONNECT", 3.0 * u, true) {
            self.join_connect();
        }
        if ui.button(hud, atlas, white, l.back, "BACK", 3.0 * u, true) {
            self.screen = Screen::MainMenu;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_filters_and_limits() {
        let mut f = JoinForm {
            addr: String::new(),
            name: String::new(),
            focus: 0,
        };
        for c in "192.168.1.10:27960 x!".chars() {
            f.type_char(c);
        }
        assert_eq!(f.addr, "192.168.1.10:27960x", "space and ! dropped");
        f.backspace();
        assert_eq!(f.addr, "192.168.1.10:27960");
        f.focus = 1;
        for c in "Anna-B:1234567890123".chars() {
            f.type_char(c);
        }
        assert_eq!(f.name, "Anna-B1234567890", "colon dropped, 16 chars max");
    }
}
