//! Função de avaliação: dá uma nota para uma posição não terminal do ponto
//! de vista da nossa cobra. Quanto maior, melhor para nós.
//!
//! A base é o controle de território (Voronoi em bitboard): quais casas cada
//! cobra alcança antes da outra. Em cima disso entram vida, diferença de
//! tamanho, comida e acesso às caudas, com pesos que mudam ao longo do jogo.

use super::bitboard::{expand, Bitboard, Bits, CHECKER, FULL, W};
use super::endgame;
use super::ttable::Score;
use std::sync::OnceLock;

/// Pesos da avaliação. Cada par é (peso no início, peso no fim); o valor
/// usado é interpolado pelo turno entre `early_end` e `late_start`.
#[derive(Clone, Copy, Debug)]
pub struct Weights {
    pub early_end: i16,
    pub late_start: i16,
    pub health: (i32, i32),
    pub lowest_enemy_health: (i32, i32),
    pub being_longer: (i32, i32),
    pub controlled_food: (i32, i32),
    pub area: (i32, i32),
    pub food_distance: (i32, i32),
    pub controlled_tails: (i32, i32),
}

impl Weights {
    pub const DEFAULT: Weights = Weights {
        early_end: 0,
        late_start: 632,
        health: (1, 0),
        lowest_enemy_health: (-2, 0),
        being_longer: (9, 0),
        controlled_food: (0, 3),
        area: (1, 7),
        food_distance: (7, 0),
        controlled_tails: (6, 20),
    };

    /// Lê pesos de uma string "early_end,late_start,h0,h1,e0,e1,l0,l1,f0,f1,a0,a1,d0,d1,t0,t1".
    /// Usado só para experimentos na arena, via a variável `EVAL_WEIGHTS`.
    pub fn parse(s: &str) -> Option<Weights> {
        let v: Vec<i32> = s.split(',').map(|x| x.trim().parse().ok()).collect::<Option<Vec<_>>>()?;
        if v.len() != 16 {
            return None;
        }
        Some(Weights {
            early_end: v[0] as i16,
            late_start: v[1] as i16,
            health: (v[2], v[3]),
            lowest_enemy_health: (v[4], v[5]),
            being_longer: (v[6], v[7]),
            controlled_food: (v[8], v[9]),
            area: (v[10], v[11]),
            food_distance: (v[12], v[13]),
            controlled_tails: (v[14], v[15]),
        })
    }
}

static WEIGHTS: OnceLock<Weights> = OnceLock::new();

/// Voronoi que considera que cada segmento de corpo libera a sua casa quando
/// a cauda passa por ela. Ligado por padrão: na arena contra a Shapeshifter
/// subiu a taxa de vitória de 55% para 77%. `TAIL_AWARE=0` desliga.
pub fn tail_aware() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| std::env::var("TAIL_AWARE").map(|v| v != "0").unwrap_or(true))
}

/// Número de passos à frente para os quais calculamos liberação de casas.
const RELEASE_STEPS: usize = 32;

/// Atraso (em passos) aplicado à liberação das casas dos corpos INIMIGOS.
/// Um inimigo que come segura a cauda por um turno, então confiar que a cauda
/// dele sai "no tempo certo" é otimista demais: a cobra entrava em corredores
/// que fechavam quando o adversário comia. Padrão 1; `ENEMY_TAIL_DELAY` muda.
fn enemy_tail_delay() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| std::env::var("ENEMY_TAIL_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(1))
}

/// `freed[t]` = casas de corpo que ficam livres exatamente no passo `t`
/// (supondo que ninguém come). Casas que demoram mais ficam como parede.
fn body_release<const S: usize>(board: &Bitboard<S>) -> [Bits; RELEASE_STEPS] {
    let mut freed = [0u128; RELEASE_STEPS];
    let delay = enemy_tail_delay();
    for (i, s) in board.snakes.iter().enumerate() {
        if s.is_dead() {
            continue;
        }
        let mut pos = s.tail;
        let mut step = s.curled as usize + 1 + if i > 0 { delay } else { 0 };
        let mut guard = 0;
        loop {
            if step >= RELEASE_STEPS {
                break;
            }
            freed[step] |= 1u128 << pos;
            if pos == s.head || guard > 128 {
                break;
            }
            pos = board.next_body_segment(pos);
            step += 1;
            guard += 1;
        }
    }
    freed
}

pub fn weights() -> &'static Weights {
    WEIGHTS.get_or_init(|| {
        std::env::var("EVAL_WEIGHTS")
            .ok()
            .and_then(|s| Weights::parse(&s))
            .unwrap_or(Weights::DEFAULT)
    })
}

/// Avaliação de uma posição não terminal.
pub fn eval<const S: usize>(board: &Bitboard<S>) -> Score {
    let w = weights();
    let me = board.snakes[0];
    let (my_area, enemy_area, food_dist) = area_control(board);
    let my_size = checkered_area_size(my_area);
    let enemy_size = checkered_area_size(enemy_area);

    if let Some(score) = endgame::solver(board, my_area, enemy_area, my_size, enemy_size, food_dist) {
        return score;
    }

    let (my_area, enemy_area, my_size, enemy_size) = if tail_aware() {
        let (a, b, _) = area_control_with(board, Some(&body_release(board)));
        (a, b, checkered_area_size(a), checkered_area_size(b))
    } else {
        (my_area, enemy_area, my_size, enemy_size)
    };

    let p = turn_progression(board.turn, w.early_end, w.late_start);
    let feats: [((i32, i32), i32); 7] = [
        (w.health, me.health as i32),
        (w.lowest_enemy_health, lowest_enemy_health(board) as i32),
        (w.being_longer, being_longer(board) as i32),
        (w.controlled_food, controlled_food_diff(board, my_area, enemy_area) as i32),
        (w.area, (my_size - enemy_size) as i32),
        (w.food_distance, W as i32 - food_dist as i32),
        (w.controlled_tails, controlled_tail_diff(board, my_area, enemy_area) as i32),
    ];
    let mut early: i32 = 0;
    let mut late: i32 = 0;
    for ((w0, w1), f) in feats {
        early += w0 * f;
        late += w1 * f;
    }
    let v = (early as f64 * (1.0 - p) + late as f64 * p).floor();
    v.clamp(-20000.0, 20000.0) as Score
}

/// Avaliação de um estado terminal (alguém morreu). Quanto mais cedo a
/// vitória, melhor; quanto mais tarde a derrota, menos ruim.
pub fn eval_terminal<const S: usize>(board: &Bitboard<S>) -> Score {
    if board.snakes[0].is_dead() {
        for s in &board.snakes[1..] {
            if s.is_alive() {
                return Score::MIN + board.turn as Score;
            }
        }
        // empate (todo mundo morreu): quase tão ruim quanto perder
        -10000 + board.turn as Score
    } else {
        Score::MAX - board.turn as Score
    }
}

fn turn_progression(turn: u16, early_end: i16, late_start: i16) -> f64 {
    let span = (late_start - early_end).max(1) as f64;
    (((turn as i16 - early_end) as f64) / span).clamp(0.0, 1.0)
}

fn lowest_enemy_health<const S: usize>(board: &Bitboard<S>) -> Score {
    let mut lowest = 100i8;
    for s in &board.snakes[1..] {
        if s.is_alive() && s.health < lowest {
            lowest = s.health;
        }
    }
    lowest as Score
}

fn largest_enemy_length<const S: usize>(board: &Bitboard<S>) -> Score {
    let mut largest = 0u8;
    for s in &board.snakes[1..] {
        if s.is_alive() && s.length > largest {
            largest = s.length;
        }
    }
    largest as Score
}

fn length_diff<const S: usize>(board: &Bitboard<S>) -> Score {
    W as Score * (board.snakes[0].length as Score - largest_enemy_length(board))
}

/// Vantagem de tamanho com retorno decrescente (logarítmico): ser 1 maior
/// importa muito, ser 10 maior não importa 10x mais.
fn being_longer<const S: usize>(board: &Bitboard<S>) -> Score {
    let d = length_diff(board);
    if d > 0 {
        (((d + 1) as f64).log(1.5) * W as f64) as Score
    } else {
        -((((-d + 1) as f64).log(1.5) * W as f64) as Score)
    }
}

fn controlled_food_diff<const S: usize>(board: &Bitboard<S>, my_area: Bits, enemy_area: Bits) -> Score {
    (my_area & board.food).count_ones() as Score - (enemy_area & board.food).count_ones() as Score
}

/// Tamanho útil de uma região: uma cobra alterna de cor a cada passo, então
/// o excesso de casas de uma cor sobre a outra só conta uma vez.
pub fn checkered_area_size(area: Bits) -> Score {
    let x = (area & CHECKER).count_ones();
    let y = (area & !CHECKER).count_ones();
    let over = x.max(y) - x.min(y);
    (x + y - over + over.min(1)) as Score
}

fn controlled_tail_diff<const S: usize>(board: &Bitboard<S>, my_area: Bits, enemy_area: Bits) -> Score {
    let mut res = 0;
    for s in &board.snakes {
        if s.is_dead() {
            continue;
        }
        let bit = 1u128 << s.tail;
        if my_area & bit != 0 {
            res += 1;
        } else if enemy_area & bit != 0 {
            res -= 1;
        }
    }
    res
}

/// Voronoi em bitboard: expande simultaneamente a partir da nossa cabeça e
/// das cabeças inimigas. Casas alcançadas no mesmo passo ficam com a cobra
/// maior (1x1) ou com ninguém.
///
/// Devolve `(minha_area, area_inimiga, distancia_ate_comida)`.
pub fn area_control<const S: usize>(board: &Bitboard<S>) -> (Bits, Bits, Score) {
    area_control_with(board, None)
}

pub fn area_control_with<const S: usize>(
    board: &Bitboard<S>,
    release: Option<&[Bits; RELEASE_STEPS]>,
) -> (Bits, Bits, Score) {
    let mut me: Bits = 1u128 << board.snakes[0].head;
    let mut enemies: Bits = 0;
    for s in &board.snakes[1..] {
        if s.is_alive() {
            enemies |= 1u128 << s.head;
        }
    }
    let mut walkable = if board.hazard_dmg > 95 {
        !board.hazard & !board.bodies[0] & FULL
    } else {
        !board.bodies[0] & FULL
    };

    let longer = if S == 2 {
        let e = largest_enemy_length(board);
        let m = board.snakes[0].length as Score;
        if m > e {
            Some(true)
        } else if m < e {
            Some(false)
        } else {
            None
        }
    } else {
        None
    };

    // casas de corpo que ainda vão ser liberadas em algum passo futuro
    let mut pending: Bits = match release {
        Some(r) => r.iter().fold(0, |acc, b| acc | b),
        None => 0,
    };
    let mut food_dist: Option<Score> = None;
    let mut step: Score = 0;
    loop {
        step += 1;
        if let Some(r) = release {
            if (step as usize) < RELEASE_STEPS {
                walkable |= r[step as usize];
                pending &= !r[step as usize];
            } else {
                pending = 0;
            }
        }
        let me_next = expand(me);
        let en_next = expand(enemies);
        let (new_me, new_en) = match longer {
            None => (me | (walkable & me_next & !en_next), enemies | (walkable & en_next & !me_next)),
            Some(true) => {
                let x = enemies | (walkable & en_next & !me_next);
                (me | (walkable & me_next & !x), x)
            }
            Some(false) => {
                let x = me | (walkable & me_next & !en_next);
                (x, enemies | (walkable & en_next & !x))
            }
        };
        if food_dist.is_none() && new_me & board.food != 0 {
            food_dist = Some(step);
        }
        if new_me == me && new_en == enemies && pending == 0 {
            return (me, enemies, food_dist.unwrap_or(W as Score));
        }
        me = new_me;
        enemies = new_en;
        if step > 2 * (W as Score) * 2 {
            // nunca deve acontecer (o tabuleiro tem 121 casas), só por segurança
            return (me, enemies, food_dist.unwrap_or(W as Score));
        }
    }
}
