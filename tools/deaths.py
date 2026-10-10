"""Lê as partidas salvas em arena_out/ e explica como a cobra "Me" morreu."""
import json, sys, glob, os

def load(path):
    lines = [l for l in open(path, encoding="utf-8").read().splitlines() if l.strip()]
    turns = [json.loads(l) for l in lines[1:-1]]
    return turns, json.loads(lines[-1])

def snake(turn, name):
    for s in turn["board"]["snakes"]:
        if s["name"] == name:
            return s
    return None

def explain(path):
    turns, result = load(path)
    # último turno em que "Me" aparece viva
    last_alive = None
    for i, t in enumerate(turns):
        if snake(t, "Me"):
            last_alive = i
    if last_alive is None:
        return f"{os.path.basename(path)}: Me nunca apareceu"
    t = turns[last_alive]
    me = snake(t, "Me"); opp = snake(t, "Opp")
    nxt = turns[last_alive + 1] if last_alive + 1 < len(turns) else None
    head = me["head"]
    cause = "?"
    if nxt:
        opp_n = snake(nxt, "Opp")
        # onde a cabeça foi: inferir pelo shout? não; comparar com oponente
        if me["health"] <= 1:
            cause = "fome"
        elif opp_n and abs(opp_n["head"]["x"] - head["x"]) + abs(opp_n["head"]["y"] - head["y"]) <= 2 and opp_n["length"] >= me["length"]:
            cause = "provável colisão de cabeças (oponente maior/igual)"
        else:
            cause = "colisão com corpo/parede ou fechada sem espaço"
    else:
        cause = "último turno do arquivo"
    w, h = t["board"]["width"], t["board"]["height"]
    # espaço livre em volta da cabeça
    occ = {(c["x"], c["y"]) for s in t["board"]["snakes"] for c in s["body"]}
    free = [(head["x"]+dx, head["y"]+dy) for dx, dy in ((1,0),(-1,0),(0,1),(0,-1))
            if 0 <= head["x"]+dx < w and 0 <= head["y"]+dy < h and (head["x"]+dx, head["y"]+dy) not in occ]
    return (f"{os.path.basename(path)}: morreu no turno {t['turn']+1} | vida {me['health']} | "
            f"len Me {me['length']} vs Opp {opp['length'] if opp else '?'} | cabeça {head} | livres {free} | "
            f"shout '{me.get('shout','')}' | {cause} | vencedor {result.get('winnerName')}")

for p in sorted(glob.glob(os.path.join(os.path.dirname(__file__), "..", "arena_out", "*.jsonl"))):
    print(explain(p))
