#!/usr/bin/env bash
# Сквозной тест интерфейса на настоящем окне (Linux, Xvfb + xdotool), только мышью:
# главное меню -> MAP EDITOR -> SAVE (файл .fmap) -> BACK -> NEW GAME -> START -> клик по слоту ленты -> протяжка ленты -> Esc -> QUIT.
# Проверяет, что игра закрылась через меню и что на карте появились постройки.
# Координаты рассчитаны на окно 1280x720 и масштаб интерфейса 1 (см. Game::menu_rects/hotbar_rect).
# Использование: scripts/e2e-ui.sh path/to/ferrustry [out_dir]
set -euo pipefail
BIN=$1; OUT=${2:-.}
mkdir -p "$OUT"
rm -f ferrustry-quicksave.fsave ferrustry-map.fmap
Xvfb :91 -screen 0 1280x720x24 >/dev/null 2>&1 & XV=$!
trap 'kill $XV 2>/dev/null || true' EXIT
sleep 1; export DISPLAY=:91
"$BIN" > "$OUT/e2e-ui.log" 2>&1 & APP=$!
for _ in $(seq 1 40); do xdotool search --name Ferrustry >/dev/null 2>&1 && break; sleep 0.5; done
sleep 2
WID=$(xdotool search --name Ferrustry | head -1)
xdotool windowmove "$WID" 0 0; xdotool windowfocus "$WID" || true; sleep 0.5
click() { xdotool mousemove "$1" "$2"; sleep 0.3; xdotool mousedown 1; sleep 0.15; xdotool mouseup 1; sleep 0.5; }
shot() { xwd -root -silent | convert xwd:- "$OUT/$1.png" 2>/dev/null || true; }
shot e2e-menu
click 640 375                     # MAP EDITOR (2-я кнопка главного меню)
click 561 684                     # SAVE (Game::editor_layout: 6 кнопок по 150+8 px внизу)
shot e2e-editor
if [ ! -s ferrustry-map.fmap ]; then echo "FAIL: map editor did not save ferrustry-map.fmap"; kill $APP; exit 1; fi
click 1035 684                    # BACK
click 640 311                     # NEW GAME
shot e2e-newgame
click 640 504                     # START (Game::new_game_layout)
sleep 1
click 465 648                     # слот 1: лента (вкладка LOGISTICS: 6 слотов по 64+6 px, по центру)
xdotool mousemove 300 450; sleep 0.3; xdotool mousedown 1; sleep 0.2
xdotool mousemove 500 450; sleep 0.2; xdotool mousemove 700 450; sleep 0.2
xdotool mouseup 1; sleep 0.8
shot e2e-playing
xdotool key Escape; sleep 0.2; xdotool key Escape; sleep 0.6   # снять инструмент, открыть меню
shot e2e-pause
click 640 483                     # QUIT (5-я кнопка меню паузы)
for _ in $(seq 1 20); do kill -0 $APP 2>/dev/null || break; sleep 0.5; done
if kill -0 $APP 2>/dev/null; then echo "FAIL: game did not quit via the pause menu"; kill $APP; exit 1; fi
line=$(grep "exit at tick" "$OUT/e2e-ui.log" || true)
echo "$line"
n=$(echo "$line" | sed -E 's/.*buildings ([0-9]+).*/\1/')
# на новой карте только ядро; протяжка добавляет ~13 лент
if [ -z "$n" ] || [ "$n" -le 10 ]; then echo "FAIL: expected new buildings, got '${n}'"; exit 1; fi
echo "PASS: saved a map in the editor, built with the mouse, quit via menu ($n buildings)"
