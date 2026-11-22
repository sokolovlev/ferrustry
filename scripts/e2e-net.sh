#!/usr/bin/env bash
# Сквозной тест сетевой игры на настоящих процессах (Linux, Xvfb + xdotool):
# выделенный сервер -> окно игры -> JOIN -> ввод адреса и имени с клавиатуры -> Enter ->
# сервер видит игрока, игра показывает ONLINE. Координаты — для окна 1280x720 (Game::menu_rects).
# Использование: scripts/e2e-net.sh path/to/ferrustry path/to/ferrustry-server [out_dir]
set -euo pipefail
BIN=$1; SERVER=$2; OUT=${3:-.}
mkdir -p "$OUT"
rm -f ferrustry-server.txt
PORT=27971
"$SERVER" --port $PORT --size small --seed 3 > "$OUT/e2e-net-server.log" 2>&1 & SRV=$!
Xvfb :92 -screen 0 1280x720x24 >/dev/null 2>&1 & XV=$!
trap 'kill $XV $SRV 2>/dev/null || true' EXIT
sleep 1; export DISPLAY=:92
"$BIN" > "$OUT/e2e-net-client.log" 2>&1 & APP=$!
for _ in $(seq 1 40); do xdotool search --name Ferrustry >/dev/null 2>&1 && break; sleep 0.5; done
sleep 2
WID=$(xdotool search --name Ferrustry | head -1)
xdotool windowmove "$WID" 0 0; xdotool windowfocus "$WID" || true; sleep 0.5
click() { xdotool mousemove "$1" "$2"; sleep 0.3; xdotool mousedown 1; sleep 0.15; xdotool mouseup 1; sleep 0.5; }
shot() { xwd -root -silent | convert xwd:- "$OUT/$1.png" 2>/dev/null || true; }
click 640 493                     # JOIN (4-я кнопка главного меню)
for _ in $(seq 1 12); do xdotool key BackSpace; done
xdotool type --delay 40 "127.0.0.1:$PORT"
xdotool key Tab
for _ in $(seq 1 8); do xdotool key BackSpace; done
xdotool type --delay 40 "e2e"
shot e2e-net-join
xdotool key Return
sleep 4
shot e2e-net-online
grep -q "player 1 \`e2e\` joined" "$OUT/e2e-net-server.log" || { echo "FAIL: the server did not see the player"; cat "$OUT/e2e-net-server.log"; exit 1; }
grep -q "^127.0.0.1:$PORT$" ferrustry-server.txt || { echo "FAIL: the server address was not remembered"; exit 1; }
xdotool key Escape; sleep 0.6   # меню паузы; партия продолжается
click 640 420                     # MAIN MENU (4-я кнопка меню паузы) — отключение
sleep 1.5
grep -q "player 1 left" "$OUT/e2e-net-server.log" || { echo "FAIL: leaving did not disconnect"; exit 1; }
kill $APP 2>/dev/null || true
echo "PASS: joined the dedicated server through the JOIN screen and left via the menu"
