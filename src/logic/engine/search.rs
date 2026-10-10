//! Busca: minimax paranoico (nós contra todos) com poda alfa-beta,
//! aprofundamento iterativo guiado por Best Node Search, tabela de
//! transposição, heurística de histórico e busca de quiescência quando as
//! cabeças estão perto.
//!
//! A busca é "anytime": respeita o `deadline` e devolve a melhor jogada da
//! última profundidade concluída.

use super::bitboard::{Bitboard, Move, MOVE_TABLE, N};
use super::eval;
use super::movegen::{allowed_moves, ordered_allowed_moves, ordered_limited_move_combinations, History, MoveCombos, MoveList};
use super::ttable::{self, Score};
use std::time::Instant;

const MAX_DEPTH: u8 = 200;
/// Profundidade máxima da quiescência (plies extras com cabeças próximas).
const QUIESCENCE_DEPTH: u8 = 5;

#[derive(Clone, Copy, Debug)]
pub struct SearchResult {
    pub mv: Move,
    pub score: Score,
    pub depth: u8,
    pub nodes: u64,
}

struct Ctx {
    deadline: Instant,
    nodes: u64,
    timed_out: bool,
    history: Box<History>,
}

impl Ctx {
    #[inline(always)]
    fn out_of_time(&mut self) -> bool {
        if !self.timed_out && self.nodes & 31 == 0 && Instant::now() >= self.deadline {
            self.timed_out = true;
        }
        self.timed_out
    }
}

pub fn search<const S: usize>(board: &Bitboard<S>, deadline: Instant) -> SearchResult {
    let mut ctx = Ctx { deadline, nodes: 0, timed_out: false, history: Box::new([[0; 4]; N]) };

    let my_allowed = ordered_allowed_moves(board, 0, &ctx.history);
    let mut enemy_moves = ordered_limited_move_combinations(board, 1, &ctx.history);
    if my_allowed.len() == 1 {
        return SearchResult { mv: my_allowed[0], score: 0, depth: 0, nodes: 0 };
    }

    let mut best_move = my_allowed[0];
    let mut best_score: Score = Score::MIN + 1;
    let mut depth: u8 = 1;
    let mut last_test: Score = 0;

    'outer: loop {
        let mut my_moves = my_allowed;
        let mut alpha = Score::MIN;
        let mut beta = Score::MAX;
        loop {
            let test = next_bns_guess(last_test, alpha, beta);
            let mut better = MoveList::new(Move::Up);
            for &mv in my_moves.iter() {
                match ab_min(&mut ctx, board, mv, &mut enemy_moves, depth, test - 1, test) {
                    Some(score) => {
                        if score >= test {
                            better.push(mv);
                        }
                    }
                    None => {
                        depth = depth.saturating_sub(1);
                        break 'outer; // acabou o tempo
                    }
                }
            }
            if better.is_empty() {
                beta = test;
            } else {
                alpha = test;
                my_moves = better;
            }
            if (beta as i32 - alpha as i32) < 2 || my_moves.len() == 1 {
                last_test = test;
                best_score = test;
                best_move = my_moves[0];
                break;
            }
        }
        if best_score > Score::MAX - 1000 || best_score < Score::MIN + 1000 || depth >= MAX_DEPTH {
            break; // resultado forçado (vitória/derrota): não precisa ir mais fundo
        }
        depth += 1;
    }

    SearchResult { mv: best_move, score: best_score, depth, nodes: ctx.nodes }
}

/// Próximo valor de teste do Best Node Search: reaproveita o palpite anterior
/// se ele ainda estiver dentro da janela, senão bissecta.
fn next_bns_guess(prev_guess: Score, alpha: Score, beta: Score) -> Score {
    if prev_guess > alpha && prev_guess < beta {
        return prev_guess;
    }
    let test = alpha / 2 + beta / 2;
    if test == beta {
        test - 1
    } else if test == alpha {
        test + 1
    } else {
        test
    }
}

/// Nó MIN: nós já escolhemos `mv`; os inimigos escolhem a combinação que
/// mais nos prejudica.
fn ab_min<const S: usize>(
    ctx: &mut Ctx,
    board: &Bitboard<S>,
    mv: Move,
    enemy_moves: &mut MoveCombos<S>,
    depth: u8,
    mut alpha: Score,
    mut beta: Score,
) -> Option<Score> {
    if ctx.out_of_time() {
        return None;
    }
    let mut tt_move: Option<[Move; S]> = None;
    let tt_key = board.hash_with_move(mv);
    if enemy_moves.len() > 1 {
        if let Some(entry) = ttable::get(tt_key) {
            if entry.depth() >= depth {
                let tt_score = entry.score();
                if entry.is_lower_bound() {
                    alpha = alpha.max(tt_score);
                } else if entry.is_upper_bound() {
                    beta = beta.min(tt_score);
                } else {
                    return Some(tt_score);
                }
                if alpha >= beta {
                    return Some(tt_score);
                }
            }
            if let Some(mvs) = entry.best_moves::<S>() {
                if legal_enemy_moves(board, &mvs) {
                    tt_move = Some(mvs);
                }
            }
        }
    }

    let mut best_score = Score::MAX;
    let mut best_moves = [Move::Up; S];
    let mut seen = MoveCombos::<S>::new([Move::Up; S]);
    let n = enemy_moves.len();
    for k in 0..n + 1 {
        let mut mvs = if k == 0 {
            match tt_move {
                Some(m) => m,
                None => continue,
            }
        } else {
            enemy_moves[k - 1]
        };
        mvs[0] = mv;
        if seen.contains(&mvs) {
            continue;
        }
        if seen.len() < 4 {
            seen.push(mvs);
        }
        let score = ab_max(ctx, board, &mvs, depth, alpha, beta)?;
        if score < alpha {
            best_score = score;
            best_moves = mvs;
            break;
        }
        if score < best_score {
            best_score = score;
            best_moves = mvs;
            if score < beta {
                beta = score;
            }
        }
    }
    ttable::insert(tt_key, best_score, best_score >= beta, best_score <= alpha, depth, best_moves);
    for i in 1..S {
        if board.snakes[i].is_alive() {
            ctx.history[board.snakes[i].head as usize][best_moves[i].to_int() as usize] += depth as u32;
        }
    }
    Some(best_score)
}

/// Nó MAX: aplica a combinação de jogadas e escolhe a nossa melhor resposta.
fn ab_max<const S: usize>(
    ctx: &mut Ctx,
    board: &Bitboard<S>,
    moves: &[Move; S],
    mut depth: u8,
    mut alpha: Score,
    mut beta: Score,
) -> Option<Score> {
    let mut child = *board;
    child.apply_moves(moves);
    ctx.nodes += 1;

    if child.is_terminal() {
        return Some(eval::eval_terminal(&child));
    } else if depth == 1 && is_stable(&child) {
        return Some(eval::eval(&child));
    }

    let tt_key = child.hash();
    let mut tt_move: Option<Move> = None;
    let my_moves = ordered_allowed_moves(&child, 0, &ctx.history);
    if my_moves.len() > 1 {
        if let Some(entry) = ttable::get(tt_key) {
            if entry.depth() >= depth {
                let tt_score = entry.score();
                if entry.is_lower_bound() {
                    alpha = alpha.max(tt_score);
                } else if entry.is_upper_bound() {
                    beta = beta.min(tt_score);
                } else {
                    return Some(tt_score);
                }
                if alpha >= beta {
                    return Some(tt_score);
                }
            }
            if let Some(m) = entry.best_moves::<1>() {
                if MOVE_TABLE[child.snakes[0].head as usize][m[0].to_int() as usize].is_some() {
                    tt_move = Some(m[0]);
                }
            }
        }
    }

    let mut next_enemy_moves = ordered_limited_move_combinations(&child, 1, &ctx.history);

    // extensão: sequências forçadas (uma única continuação) não gastam profundidade
    if my_moves.len() * next_enemy_moves.len() <= 1 {
        depth += 1;
    }

    let mut best_score = Score::MIN;
    let mut best_move = Move::Up;
    let mut seen = MoveList::new(Move::Up);
    let n = my_moves.len();
    for k in 0..n + 1 {
        let mv = if k == 0 {
            match tt_move {
                Some(m) => m,
                None => continue,
            }
        } else {
            my_moves[k - 1]
        };
        if seen.contains(&mv) {
            continue;
        }
        if seen.len() < 4 {
            seen.push(mv);
        }
        let score = if depth == 1 {
            quiescence(ctx, &child, mv, &mut next_enemy_moves, QUIESCENCE_DEPTH, alpha, beta)?
        } else {
            ab_min(ctx, &child, mv, &mut next_enemy_moves, depth - 1, alpha, beta)?
        };
        if score > beta {
            best_score = score;
            best_move = mv;
            break;
        }
        if score > best_score {
            best_score = score;
            best_move = mv;
            if score > alpha {
                alpha = score;
            }
        }
    }
    if best_score > Score::MIN {
        ttable::insert(tt_key, best_score, best_score >= beta, best_score <= alpha, depth, [best_move; 1]);
        ctx.history[child.snakes[0].head as usize][best_move.to_int() as usize] += depth as u32;
    }
    Some(best_score)
}

/// Posição "calma" o suficiente para ser avaliada: nenhuma cabeça inimiga a
/// menos de 3 casas da nossa. Perto disso, uma colisão de cabeças pode mudar
/// tudo em um turno, então a busca continua (quiescência).
#[inline]
fn is_stable<const S: usize>(board: &Bitboard<S>) -> bool {
    for s in &board.snakes[1..] {
        if s.is_alive() && Bitboard::<S>::distance(board.snakes[0].head, s.head) < 3 {
            return false;
        }
    }
    true
}

/// Quiescência: continua a busca (sem tabela de transposição, janela atual)
/// enquanto a posição estiver instável, até `depth` plies extras.
fn quiescence<const S: usize>(
    ctx: &mut Ctx,
    board: &Bitboard<S>,
    mv: Move,
    enemy_moves: &mut MoveCombos<S>,
    depth: u8,
    alpha: Score,
    mut beta: Score,
) -> Option<Score> {
    if ctx.out_of_time() {
        return None;
    }
    let mut best_score = Score::MAX;
    let mut best_moves = [Move::Up; S];
    for k in 0..enemy_moves.len() {
        let mut mvs = enemy_moves[k];
        mvs[0] = mv;
        // nó MAX embutido
        let score = {
            let mut ialpha = alpha;
            let ibeta = beta;
            let mut ibest_score = Score::MIN;
            let mut ibest_move = Move::Up;
            let mut child = *board;
            child.apply_moves(&mvs);
            ctx.nodes += 1;

            if child.is_terminal() {
                eval::eval_terminal(&child)
            } else if depth == 1 || is_stable(&child) {
                eval::eval(&child)
            } else {
                let mut next_enemy_moves = ordered_limited_move_combinations(&child, 1, &ctx.history);
                let my_moves = ordered_allowed_moves(&child, 0, &ctx.history);
                for &imv in my_moves.iter() {
                    let iscore = quiescence(ctx, &child, imv, &mut next_enemy_moves, depth - 1, ialpha, ibeta)?;
                    if iscore > ibeta {
                        ibest_score = iscore;
                        ibest_move = imv;
                        break;
                    }
                    if iscore > ibest_score {
                        ibest_score = iscore;
                        ibest_move = imv;
                        if iscore > ialpha {
                            ialpha = iscore;
                        }
                    }
                }
                ctx.history[child.snakes[0].head as usize][ibest_move.to_int() as usize] += depth as u32;
                ibest_score
            }
        };
        if score < alpha {
            best_score = score;
            best_moves = mvs;
            break;
        }
        if score < best_score {
            best_score = score;
            best_moves = mvs;
            if score < beta {
                beta = score;
            }
        }
    }
    for i in 1..S {
        if board.snakes[i].is_alive() {
            ctx.history[board.snakes[i].head as usize][best_moves[i].to_int() as usize] += depth as u32;
        }
    }
    Some(best_score)
}

#[inline]
fn legal_enemy_moves<const S: usize>(board: &Bitboard<S>, mvs: &[Move; S]) -> bool {
    for i in 1..S {
        if board.snakes[i].is_alive() && MOVE_TABLE[board.snakes[i].head as usize][mvs[i].to_int() as usize].is_none() {
            return false;
        }
    }
    true
}

/// Jogada de emergência caso a busca não rode (nunca deve acontecer).
pub fn any_safe_move<const S: usize>(board: &Bitboard<S>) -> Move {
    allowed_moves(board, 0)[0]
}
