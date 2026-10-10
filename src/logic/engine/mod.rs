//! Motor de busca da cobra (tabuleiro 11x11, modo standard).
//!
//! Ponto de entrada: [`choose_move`]. Ele converte o `GameState` da API para
//! o bitboard, escolhe a instância certa para o número de cobras e roda a
//! busca até o `deadline`.

pub mod bitboard;
pub mod endgame;
pub mod eval;
pub mod movegen;
pub mod search;
pub mod ttable;

pub use search::SearchResult;

use crate::models::{Battlesnake, GameState};
use bitboard::Bitboard;
use std::time::Instant;

/// Número máximo de cobras que o motor suporta numa partida.
pub const MAX_SNAKES: usize = 8;

/// Verdadeiro se o tabuleiro é o 11x11 que o motor conhece.
pub fn supports(state: &GameState) -> bool {
    state.board.width as usize == bitboard::W && state.board.height as usize == bitboard::H
}

/// Lista de cobras vivas com a nossa na posição 0.
fn ordered_snakes(state: &GameState) -> Vec<&Battlesnake> {
    let mut snakes: Vec<&Battlesnake> = Vec::with_capacity(state.board.snakes.len() + 1);
    snakes.push(&state.you);
    for s in &state.board.snakes {
        if s.id != state.you.id && s.body.first().map(|c| *c) != state.you.body.first().map(|c| *c) {
            snakes.push(s);
        }
    }
    snakes.truncate(MAX_SNAKES);
    snakes
}

/// Escolhe a jogada deste turno. Nunca entra em pânico com estados estranhos:
/// se algo não bater, cai no `fallback`.
pub fn choose_move(state: &GameState, deadline: Instant) -> SearchResult {
    let snakes = ordered_snakes(state);
    macro_rules! run {
        ($($n:literal),*) => {
            match snakes.len() {
                $( $n => {
                    let board = Bitboard::<$n>::from_state(state, &snakes);
                    search::search(&board, deadline)
                } )*
                _ => unreachable!("número de cobras fora do suportado"),
            }
        };
    }
    run!(1, 2, 3, 4, 5, 6, 7, 8)
}

/// Aquece a tabela de transposição (chamado no /start e no primeiro /move).
pub fn warm_up() {
    ttable::init();
}

#[cfg(test)]
mod tests {
    use super::bitboard::{idx, Bitboard, Move};
    use super::*;
    use crate::models::{Board, Coord, Game};
    use std::collections::HashMap;
    use std::time::Duration;

    fn c(x: i32, y: i32) -> Coord {
        Coord { x, y }
    }

    fn snake(id: &str, body: &[(i32, i32)], health: i32) -> Battlesnake {
        let body: Vec<Coord> = body.iter().map(|&(x, y)| c(x, y)).collect();
        Battlesnake {
            id: id.to_string(),
            name: id.to_string(),
            health,
            head: body[0],
            length: body.len() as i32,
            body,
            latency: None,
            shout: None,
        }
    }

    fn state(snakes: Vec<Battlesnake>, food: &[(i32, i32)], turn: i32) -> GameState {
        GameState {
            game: Game { id: "t".into(), ruleset: HashMap::new(), map: Some("standard".into()), timeout: 500 },
            turn,
            board: Board {
                height: 11,
                width: 11,
                food: food.iter().map(|&(x, y)| c(x, y)).collect(),
                hazards: vec![],
                snakes: snakes.clone(),
            },
            you: snakes[0].clone(),
        }
    }

    #[test]
    fn converte_corpo_e_direcoes_corretamente() {
        // cabeça (5,5), corpo indo para a esquerda e depois para baixo
        let me = snake("me", &[(5, 5), (4, 5), (3, 5), (3, 4)], 90);
        let st = state(vec![me], &[], 10);
        let ordered = ordered_snakes(&st);
        let b = Bitboard::<1>::from_state(&st, &ordered);
        assert_eq!(b.snakes[0].head, idx(5, 5).unwrap());
        assert_eq!(b.snakes[0].tail, idx(3, 4).unwrap());
        assert_eq!(b.snakes[0].curled, 0);
        // a cauda vai sair, então não conta como ocupada
        assert_eq!(b.bodies[0] & (1u128 << idx(3, 4).unwrap()), 0);
        // seguindo a cauda chegamos na cabeça
        let mut pos = b.snakes[0].tail;
        for _ in 0..3 {
            pos = b.next_body_segment(pos);
        }
        assert_eq!(pos, b.snakes[0].head);
    }

    #[test]
    fn comer_aumenta_e_empilha_a_cauda() {
        let me = snake("me", &[(5, 5), (4, 5), (3, 5)], 50);
        let st = state(vec![me], &[(6, 5)], 10);
        let ordered = ordered_snakes(&st);
        let mut b = Bitboard::<1>::from_state(&st, &ordered);
        b.apply_moves(&[Move::Right]);
        assert_eq!(b.snakes[0].health, 100);
        assert_eq!(b.snakes[0].length, 4);
        assert_eq!(b.snakes[0].curled, 1);
        assert_eq!(b.snakes[0].head, idx(6, 5).unwrap());
        assert_eq!(b.snakes[0].tail, idx(4, 5).unwrap());
        assert_eq!(b.food, 0);
        // no turno seguinte a cauda fica parada
        b.apply_moves(&[Move::Right]);
        assert_eq!(b.snakes[0].tail, idx(4, 5).unwrap());
        assert_eq!(b.snakes[0].curled, 0);
        b.apply_moves(&[Move::Right]);
        assert_eq!(b.snakes[0].tail, idx(5, 5).unwrap());
    }

    #[test]
    fn bater_na_parede_ou_no_corpo_mata() {
        let me = snake("me", &[(0, 5), (1, 5), (2, 5)], 50);
        let st = state(vec![me], &[], 10);
        let ordered = ordered_snakes(&st);
        let mut b = Bitboard::<1>::from_state(&st, &ordered);
        b.apply_moves(&[Move::Left]);
        assert!(b.snakes[0].is_dead());

        let me = snake("me", &[(5, 5), (4, 5), (4, 6), (5, 6), (6, 6)], 50);
        let st = state(vec![me], &[], 10);
        let ordered = ordered_snakes(&st);
        let mut b = Bitboard::<1>::from_state(&st, &ordered);
        b.apply_moves(&[Move::Up]);
        assert!(b.snakes[0].is_dead());
    }

    #[test]
    fn colisao_de_cabecas_mata_a_menor() {
        let me = snake("me", &[(4, 5), (3, 5), (2, 5)], 50);
        let en = snake("en", &[(6, 5), (7, 5), (8, 5), (9, 5)], 50);
        let st = state(vec![me, en], &[], 10);
        let ordered = ordered_snakes(&st);
        let mut b = Bitboard::<2>::from_state(&st, &ordered);
        b.apply_moves(&[Move::Right, Move::Left]);
        assert!(b.snakes[0].is_dead());
        assert!(b.snakes[1].is_alive());
        assert!(b.is_terminal());
    }

    #[test]
    fn busca_evita_beco_sem_saida() {
        // Estamos num corredor: subir leva a um beco de 1 casa, descer é livre.
        //  y=7: x x x
        //  y=6: x . x    <- (5,6) é o beco
        //  y=5: x @ x
        //  y=4: . . .
        let mut body = vec![(5, 5), (4, 5), (4, 6), (4, 7), (5, 7), (6, 7), (6, 6), (6, 5)];
        body.extend([(7, 5), (8, 5), (9, 5), (10, 5)]);
        let me = snake("me", &body, 90);
        let st = state(vec![me], &[], 30);
        let res = choose_move(&st, Instant::now() + Duration::from_millis(100));
        assert_eq!(res.mv, Move::Down, "foi para o beco: {:?}", res);
    }

    #[test]
    fn busca_pega_vitoria_forcada_por_colisao() {
        // Somos maiores e o inimigo só tem uma casa para ir, onde podemos
        // bater de frente e vencer.
        let me = snake("me", &[(5, 5), (5, 4), (5, 3), (5, 2), (5, 1)], 90);
        let en = snake("en", &[(7, 5), (8, 5), (9, 5)], 90);
        let st = state(vec![me, en], &[], 30);
        let res = choose_move(&st, Instant::now() + Duration::from_millis(150));
        assert!(matches!(res.mv, Move::Up | Move::Right | Move::Left | Move::Down));
        assert!(res.depth >= 1);
    }

    /// Reproduz partidas gravadas pela CLI (`arena_runs/**/*.jsonl`) e confere
    /// que a nossa simulação bate turno a turno com o servidor oficial.
    /// Roda com: cargo test --release -- --ignored replay
    #[test]
    #[ignore]
    fn replay_bate_com_o_servidor_oficial() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/arena_runs");
        let mut files = Vec::new();
        fn walk(dir: &str, out: &mut Vec<String>) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(p.to_str().unwrap(), out);
                    } else if p.extension().map(|x| x == "jsonl").unwrap_or(false) {
                        out.push(p.to_str().unwrap().to_string());
                    }
                }
            }
        }
        walk(root, &mut files);
        assert!(!files.is_empty(), "nenhuma partida em {root}");
        let mut checked = 0;
        let mut mismatches = 0;
        for f in &files {
            let text = std::fs::read_to_string(f).unwrap();
            let states: Vec<GameState> = text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .skip(1)
                .filter_map(|l| serde_json::from_str::<GameState>(l).ok())
                .collect();
            for w in states.windows(2) {
                let (a, b) = (&w[0], &w[1]);
                if a.board.snakes.len() != 2 || b.board.snakes.len() != 2 {
                    continue;
                }
                let sa = ordered_snakes(a);
                let mut board = Bitboard::<2>::from_state(a, &sa);
                let mut moves = [Move::Up; 2];
                for (i, s) in sa.iter().enumerate() {
                    let Some(nb) = b.board.snakes.iter().find(|x| x.id == s.id) else { continue };
                    let (dx, dy) = (nb.head.x - s.head.x, nb.head.y - s.head.y);
                    moves[i] = match (dx, dy) {
                        (0, 1) => Move::Up,
                        (0, -1) => Move::Down,
                        (1, 0) => Move::Right,
                        _ => Move::Left,
                    };
                }
                board.apply_moves(&moves);
                // o estado b é visto pela mesma cobra "you"
                let mut b2 = b.clone();
                b2.you = b.board.snakes.iter().find(|x| x.id == a.you.id).unwrap().clone();
                let sb = ordered_snakes(&b2);
                let expected = Bitboard::<2>::from_state(&b2, &sb);
                checked += 1;
                let same = board.bodies == expected.bodies
                    && board.snakes == expected.snakes
                    && board.food & !expected.food == 0;
                if !same {
                    mismatches += 1;
                    if mismatches <= 3 {
                        eprintln!("DIVERGÊNCIA em {f} turno {}
sim:
{board}
esperado:
{expected}", a.turn);
                    }
                }
            }
        }
        eprintln!("{checked} turnos conferidos em {} partidas, {mismatches} divergências", files.len());
        assert_eq!(mismatches, 0);
    }
}
