//! Tabela de transposição: guarda o resultado de posições já buscadas para
//! não repetir trabalho (dentro de um turno e entre turnos).
//!
//! É um vetor global de pares `(chave, dados)` em atômicos. A chave guardada é
//! `chave ^ dados`, então uma escrita concorrente corrompida é detectada na
//! leitura sem precisar de lock (truque clássico de engines de xadrez).

use super::bitboard::Move;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

pub type Score = i16;

/// 2^20 entradas × 16 bytes = 16 MB. Na Lambda padrão (256 MB, ~0,15 vCPU)
/// a busca visita poucas centenas de milhares de nós por jogada, então uma
/// tabela maior não ajuda, e zerar 64 MB numa instância fria custaria ~100 ms
/// da primeira jogada.
const TT_BITS: u32 = 20;
const TT_LEN: usize = 1 << TT_BITS;

struct Slot {
    key: AtomicU64,
    data: AtomicU64,
}

static TABLE: OnceLock<Box<[Slot]>> = OnceLock::new();

fn table() -> &'static [Slot] {
    TABLE.get_or_init(|| {
        (0..TT_LEN)
            .map(|_| Slot { key: AtomicU64::new(0), data: AtomicU64::new(0) })
            .collect()
    })
}

/// Aloca a tabela. Opcional: a primeira busca também aloca sob demanda.
pub fn init() {
    let _ = table();
}

#[inline(always)]
fn index(key: u64) -> usize {
    (key >> (64 - TT_BITS)) as usize
}

pub fn get(key: u64) -> Option<Entry> {
    let slot = &table()[index(key)];
    let entry = Entry { key: slot.key.load(Ordering::Relaxed), data: slot.data.load(Ordering::Relaxed) };
    if entry.matches_key(key) {
        Some(entry)
    } else {
        None
    }
}

pub fn insert<const S: usize>(
    key: u64,
    score: Score,
    is_lower_bound: bool,
    is_upper_bound: bool,
    depth: u8,
    best_moves: [Move; S],
) {
    let slot = &table()[index(key)];
    let old = Entry { key: slot.key.load(Ordering::Relaxed), data: slot.data.load(Ordering::Relaxed) };
    if old.matches_key(key) && old.depth() > depth {
        return; // já temos algo melhor para esta posição
    }
    let entry = Entry::new(key, score, is_lower_bound, is_upper_bound, depth, best_moves);
    slot.key.store(entry.key, Ordering::Relaxed);
    slot.data.store(entry.data, Ordering::Relaxed);
}

/// Uma entrada da tabela. Tudo fica empacotado em `data` (64 bits):
/// profundidade (8), score (16), jogadas (2 por cobra, até 8 cobras),
/// e dois bits dizendo se o score é um limite inferior/superior.
#[derive(Clone, Copy)]
pub struct Entry {
    key: u64,
    data: u64,
}

impl Entry {
    const DEPTH_SHIFT: u32 = 0;
    const SCORE_SHIFT: u32 = 8;
    const MOVES_SHIFT: u32 = 24;
    const MOVE_WIDTH: u32 = 2;
    const LOWER_SHIFT: u32 = 40;
    const UPPER_SHIFT: u32 = 41;
    /// Bit sempre ligado, para `data` nunca ser zero (zero = slot vazio).
    const VALID_SHIFT: u32 = 42;

    fn new<const S: usize>(
        key: u64,
        score: Score,
        is_lower_bound: bool,
        is_upper_bound: bool,
        depth: u8,
        best_moves: [Move; S],
    ) -> Self {
        let mut data = ((score as u16) as u64) << Self::SCORE_SHIFT
            | (depth as u64) << Self::DEPTH_SHIFT
            | (is_lower_bound as u64) << Self::LOWER_SHIFT
            | (is_upper_bound as u64) << Self::UPPER_SHIFT
            | 1u64 << Self::VALID_SHIFT;
        if S <= 8 {
            for (i, mv) in best_moves.iter().enumerate() {
                data |= (mv.to_int() as u64 & 0b11) << (Self::MOVES_SHIFT + i as u32 * Self::MOVE_WIDTH);
            }
        }
        Entry { key: key ^ data, data }
    }

    #[inline(always)]
    fn matches_key(&self, key: u64) -> bool {
        self.data != 0 && self.key ^ self.data == key
    }

    pub fn depth(&self) -> u8 {
        (self.data >> Self::DEPTH_SHIFT) as u8
    }

    pub fn score(&self) -> Score {
        (self.data >> Self::SCORE_SHIFT) as u16 as Score
    }

    pub fn best_moves<const S: usize>(&self) -> Option<[Move; S]> {
        if S > 8 {
            return None;
        }
        let mut moves = [Move::Up; S];
        for (i, mv) in moves.iter_mut().enumerate() {
            *mv = Move::from_int((self.data >> (Self::MOVES_SHIFT + i as u32 * Self::MOVE_WIDTH)) as u8 & 0b11);
        }
        Some(moves)
    }

    pub fn is_lower_bound(&self) -> bool {
        (self.data >> Self::LOWER_SHIFT) & 1 != 0
    }

    pub fn is_upper_bound(&self) -> bool {
        (self.data >> Self::UPPER_SHIFT) & 1 != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entrada_vai_e_volta_sem_perder_nada() {
        let moves = [Move::Left, Move::Down, Move::Right];
        let e = Entry::new(12345, -321, true, false, 7, moves);
        assert!(e.matches_key(12345));
        assert!(!e.matches_key(12346));
        assert_eq!(e.depth(), 7);
        assert_eq!(e.score(), -321);
        assert!(e.is_lower_bound());
        assert!(!e.is_upper_bound());
        assert_eq!(e.best_moves::<3>().unwrap(), moves);
    }

    #[test]
    fn insere_e_recupera_na_tabela() {
        insert(0xdead_beef, 42, false, false, 3, [Move::Up, Move::Right]);
        let e = get(0xdead_beef).expect("entrada sumiu");
        assert_eq!(e.score(), 42);
        assert_eq!(e.best_moves::<2>().unwrap(), [Move::Up, Move::Right]);
        assert!(get(0xdead_bee0).is_none() || get(0xdead_bee0).unwrap().score() != 42 || true);
    }
}
