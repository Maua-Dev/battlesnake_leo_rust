"""Arena local: roda partidas 1x1 entre duas cobras com a CLI oficial do
Battlesnake e mede a taxa de vitória com intervalo de confiança.

As partidas rodam em pares com a ordem das cobras trocada, para anular a
vantagem da posição inicial. As derrotas ficam salvas em `arena_out/` para
regressão.

Uso:
    python tools/arena.py --me http://localhost:8082 --opp http://localhost:8081 -n 20 -j 3

Requer a CLI `battlesnake` (baixada automaticamente para `tools/` se faltar).
"""

import argparse
import concurrent.futures
import json
import math
import os
import queue
import shutil
import subprocess
import sys
import tarfile
import urllib.request
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
OUT_DIR = os.path.join(HERE, "..", "arena_out")


def download_cli():
    name = "battlesnake.exe" if os.name == "nt" else "battlesnake"
    path = os.path.join(HERE, name)
    if os.path.exists(path):
        return path
    found = shutil.which("battlesnake")
    if found:
        return found
    print("Baixando a CLI do Battlesnake...")
    if os.name == "nt":
        url = "https://github.com/BattlesnakeOfficial/rules/releases/latest/download/battlesnake_windows_amd64.zip"
        tmp = os.path.join(HERE, "cli.zip")
        urllib.request.urlretrieve(url, tmp)
        with zipfile.ZipFile(tmp) as z:
            z.extract(name, HERE)
    else:
        url = "https://github.com/BattlesnakeOfficial/rules/releases/latest/download/battlesnake_linux_amd64.tar.gz"
        tmp = os.path.join(HERE, "cli.tar.gz")
        urllib.request.urlretrieve(url, tmp)
        with tarfile.open(tmp) as t:
            t.extract(name, HERE)
        os.chmod(path, 0o755)
    os.remove(tmp)
    return path


def wilson(wins, n, z=1.96):
    if n == 0:
        return 0.0, 0.0
    p = wins / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    s = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return c - s, c + s


# Cada partida simultânea pega um "slot" exclusivo. Com --me/--opp contendo
# várias URLs separadas por vírgula (réplicas), o slot i usa a i-ésima réplica,
# como numa Lambda, onde cada requisição simultânea ganha sua própria instância.
SLOTS = queue.Queue()
ME_URLS = []
OPP_URLS = []


def run_match(args):
    cli, match_id, swap, timeout, seed = args
    slot = SLOTS.get()
    try:
        return _run_match(cli, ME_URLS[slot % len(ME_URLS)], OPP_URLS[slot % len(OPP_URLS)], match_id, swap, timeout, seed)
    finally:
        SLOTS.put(slot)


def _run_match(cli, me, opp, match_id, swap, timeout, seed):
    out = os.path.join(OUT_DIR, f"match_{match_id}.jsonl")
    snakes = [("Me", me), ("Opp", opp)]
    if swap:
        snakes.reverse()
    cmd = [cli, "play", "-W", "11", "-H", "11", "-g", "standard", "-t", str(timeout), "--output", out]
    if seed is not None:
        cmd += ["--seed", str(seed + match_id)]
    for name, url in snakes:
        cmd += ["--name", name, "--url", url]
    subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    winner, turns = None, 0
    try:
        with open(out, encoding="utf-8") as f:
            lines = [l for l in f.read().splitlines() if l.strip()]
        last = json.loads(lines[-1])
        if "winnerName" in last:
            winner = "draw" if last.get("isDraw") else last["winnerName"]
        if len(lines) >= 2:
            turns = json.loads(lines[-2]).get("turn", 0)
    except Exception as e:  # noqa: BLE001
        print(f"\nerro lendo a partida {match_id}: {e}")
    if winner == "Me":
        os.remove(out)
    return match_id, winner, turns, swap


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--me", required=True, help="URL (ou várias, separadas por vírgula)")
    p.add_argument("--opp", required=True, help="URL (ou várias, separadas por vírgula)")
    p.add_argument("-n", type=int, default=20, help="pares de partidas (total = 2n)")
    p.add_argument("-j", type=int, default=3, help="partidas simultâneas")
    p.add_argument("-t", type=int, default=500, help="timeout por jogada (ms)")
    p.add_argument("--seed", type=int, default=None, help="semente base (reprodutível)")
    a = p.parse_args()

    os.makedirs(OUT_DIR, exist_ok=True)
    cli = download_cli()
    ME_URLS.extend(u.strip() for u in a.me.split(",") if u.strip())
    OPP_URLS.extend(u.strip() for u in a.opp.split(",") if u.strip())
    for slot in range(a.j):
        SLOTS.put(slot)
    tasks = []
    for i in range(a.n):
        tasks.append((cli, 2 * i, False, a.t, a.seed))
        tasks.append((cli, 2 * i + 1, True, a.t, a.seed))

    wins = losses = draws = errors = 0
    turns_total = 0
    print(f"{len(tasks)} partidas, {a.j} por vez...")
    with concurrent.futures.ThreadPoolExecutor(max_workers=a.j) as ex:
        for match_id, winner, turns, swap in ex.map(run_match, tasks):
            turns_total += turns
            if winner == "Me":
                wins += 1
            elif winner == "Opp":
                losses += 1
            elif winner == "draw":
                draws += 1
            else:
                errors += 1
            done = wins + losses + draws + errors
            print(f"\r{done}/{len(tasks)}  V {wins}  D {losses}  E {draws}  err {errors}", end="", flush=True)

    print("\n\n=== Resultado ===")
    dec = wins + losses
    if dec:
        lo, hi = wilson(wins, dec)
        print(f"Taxa de vitória: {wins / dec:.1%}  (IC 95%: {lo:.1%} a {hi:.1%})")
    print(f"Vitórias {wins}  Derrotas {losses}  Empates {draws}  Erros {errors}")
    if len(tasks):
        print(f"Turnos por partida (média): {turns_total / len(tasks):.0f}")
    print(f"Derrotas salvas em {os.path.abspath(OUT_DIR)}")
    return 0 if wins > losses else 1


if __name__ == "__main__":
    sys.exit(main())
