//! Resolvedor de finais 1x1: quando as duas cobras estão separadas em regiões
//! próprias, dá para decidir quem vence contando casas e esperando a cauda.
//!
//! Devolve `Some(score)` quando acha que consegue determinar um vencedor;
//! `None` quando o resultado ainda depende do jogo.
//!
//! O veredito é uma heurística (ignora, por exemplo, que a cauda do próprio
//! inimigo pode abrir caminho), então o score fica abaixo do de um estado
//! terminal de verdade: assim a busca continua aprofundando e, se a simulação
//! mostrar uma saída, ela prevalece. Com o score no nível terminal a busca
//! parava na profundidade 1 e escolhia entre jogadas "perdidas" sem olhar.

use super::bitboard::{expand, Bitboard, Bits};
use super::ttable::Score;

/// Magnitude do veredito: acima de qualquer avaliação normal (limitada a
/// ±20000), abaixo de vitória/derrota reais (≈ ±32000).
const SOLVER_SCORE: Score = 26000;

pub fn solver<const S: usize>(
    board: &Bitboard<S>,
    my_area: Bits,
    enemy_area: Bits,
    my_area_size: Score,
    enemy_area_size: Score,
    my_food_distance: Score,
) -> Option<Score> {
    match loser(board, my_area, enemy_area, my_area_size, enemy_area_size, my_food_distance) {
        Some(0) => Some(-SOLVER_SCORE + board.turn as Score + my_area_size),
        Some(_) => Some(SOLVER_SCORE - board.turn as Score - enemy_area_size),
        None => None,
    }
}

/// Índice da cobra que perde, se der para determinar.
fn loser<const S: usize>(
    board: &Bitboard<S>,
    my_area: Bits,
    enemy_area: Bits,
    my_area_size: Score,
    enemy_area_size: Score,
    my_food_distance: Score,
) -> Option<usize> {
    if S != 2 || board.turn < 50 {
        return None;
    }
    let me = board.snakes[0];
    let enemy = board.snakes[1];

    // Região de cada um expandida em uma casa: as caudas que tocam essa
    // região "chegam" para a cobra seguir.
    let em_fill = expand(my_area);
    let ee_fill = expand(enemy_area);

    let tail_in_my_reach = |pos: u8| em_fill & (1u128 << pos) != 0;
    let tail_in_enemy_reach = |pos: u8| ee_fill & (1u128 << pos) != 0;

    // Em quantos turnos a MINHA cauda passa perto de mim / do inimigo.
    let mut counter: Score = 1;
    let mut my_tail_dist = 0;
    let mut enemy_tail_dist = 0;
    if tail_in_my_reach(me.tail) && !(enemy.length > me.length && enemy_area & (1u128 << me.tail) != 0) {
        my_tail_dist = counter;
    }
    if tail_in_enemy_reach(me.tail) && !(me.length > enemy.length && my_area & (1u128 << me.tail) != 0) {
        enemy_tail_dist = counter;
    }
    let mut pos = board.next_body_segment(me.tail);
    let mut guard = 0;
    while (my_tail_dist == 0 || enemy_tail_dist == 0) && pos != me.head && guard < 256 {
        guard += 1;
        if my_tail_dist == 0 && tail_in_my_reach(pos) {
            my_tail_dist = counter;
        }
        if enemy_tail_dist == 0 && tail_in_enemy_reach(pos) {
            enemy_tail_dist = counter;
        }
        counter += 1;
        pos = board.next_body_segment(pos);
    }
    if my_tail_dist == 0 {
        my_tail_dist = counter;
    }
    if enemy_tail_dist == 0 {
        enemy_tail_dist = counter;
    }

    // Mesma coisa para a cauda do INIMIGO.
    counter = 1;
    let mut my_etail_dist = 0;
    let mut enemy_etail_dist = 0;
    if tail_in_my_reach(enemy.tail) && !(enemy.length > me.length && enemy_area & (1u128 << enemy.tail) != 0) {
        my_etail_dist = counter;
    }
    if tail_in_enemy_reach(enemy.tail) && !(me.length > enemy.length && my_area & (1u128 << enemy.tail) != 0) {
        enemy_etail_dist = counter;
    }
    pos = board.next_body_segment(enemy.tail);
    guard = 0;
    while (counter < my_tail_dist || counter < enemy_tail_dist)
        && (my_etail_dist == 0 || enemy_etail_dist == 0)
        && pos != enemy.head
        && guard < 256
    {
        guard += 1;
        if my_etail_dist == 0 && tail_in_my_reach(pos) {
            my_etail_dist = counter;
        }
        if enemy_etail_dist == 0 && tail_in_enemy_reach(pos) {
            enemy_etail_dist = counter;
        }
        counter += 1;
        pos = board.next_body_segment(pos);
    }
    if my_etail_dist == 0 {
        my_etail_dist = counter;
    }
    if enemy_etail_dist == 0 {
        enemy_etail_dist = counter;
    }
    my_tail_dist = my_tail_dist.min(my_etail_dist);
    enemy_tail_dist = enemy_tail_dist.min(enemy_etail_dist);

    // Decide quem está preso sem saída (região menor que o tempo até uma
    // cauda liberar espaço) enquanto o outro sobrevive tempo suficiente.
    let me_trapped = my_area_size < my_tail_dist && enemy.health as Score > my_area_size;
    let enemy_trapped = enemy_area_size < enemy_tail_dist && me.health as Score > enemy_area_size;
    if me_trapped {
        if enemy_trapped {
            if my_area_size < enemy_area_size {
                Some(0)
            } else if enemy_area_size < my_area_size {
                Some(1)
            } else {
                None
            }
        } else {
            Some(0)
        }
    } else if enemy_trapped {
        Some(1)
    } else if (me.health as Score) < my_food_distance {
        Some(0)
    } else {
        None
    }
}
