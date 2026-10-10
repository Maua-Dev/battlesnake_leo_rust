//! Geração de jogadas: quais direções uma cobra pode tomar e quais
//! combinações de jogadas inimigas vale a pena considerar na busca.

use super::bitboard::{Bitboard, Move, N};

/// Vetor de tamanho fixo (até `C` elementos) que vive na pilha.
#[derive(Clone, Copy, Debug)]
pub struct SmallVec<T: Copy, const C: usize> {
    len: usize,
    data: [T; C],
}

impl<T: Copy + PartialEq, const C: usize> SmallVec<T, C> {
    /// Cria um vetor vazio. `fill` só preenche as posições não usadas.
    #[inline(always)]
    pub fn new(fill: T) -> Self {
        SmallVec { len: 0, data: [fill; C] }
    }

    #[inline(always)]
    pub fn push(&mut self, v: T) {
        debug_assert!(self.len < C);
        self.data[self.len] = v;
        self.len += 1;
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[T] {
        &self.data[..self.len]
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data[..self.len]
    }

    #[inline(always)]
    pub fn contains(&self, v: &T) -> bool {
        self.as_slice().contains(v)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.as_slice().iter()
    }
}

impl<T: Copy + PartialEq, const C: usize> std::ops::Index<usize> for SmallVec<T, C> {
    type Output = T;
    #[inline(always)]
    fn index(&self, i: usize) -> &T {
        &self.data[i]
    }
}

impl<T: Copy + PartialEq, const C: usize> std::ops::IndexMut<usize> for SmallVec<T, C> {
    #[inline(always)]
    fn index_mut(&mut self, i: usize) -> &mut T {
        &mut self.data[i]
    }
}

pub type MoveList = SmallVec<Move, 4>;
pub type MoveCombos<const S: usize> = SmallVec<[Move; S], 4>;

/// Heurística de histórico: quantas vezes cada (casa, direção) foi a melhor
/// jogada na busca. Usada para ordenar jogadas.
pub type History = [[u32; 4]; N];

/// Direções que não matam a cobra `i` imediatamente (sem contar colisões de
/// cabeça). Se nenhuma for segura, devolve uma direção legal qualquer, para a
/// busca sempre ter o que explorar.
pub fn allowed_moves<const S: usize>(board: &Bitboard<S>, i: usize) -> MoveList {
    let mut moves = MoveList::new(Move::Up);
    let pos = board.snakes[i].head;
    let survives_hazard = board.snakes[i].health > board.hazard_dmg;
    let mut some_legal = Move::Up;
    let mut some_better = None;

    for (mv_int, dest) in Bitboard::<S>::moves_from(pos).iter().enumerate() {
        if let Some(dest) = *dest {
            let mv = Move::from_int(mv_int as u8);
            some_legal = mv;
            let bit = 1u128 << dest;
            if survives_hazard || board.hazard & bit == 0 || board.food & bit != 0 {
                some_better = Some(mv);
                if board.bodies[0] & bit == 0 {
                    moves.push(mv);
                }
            }
        }
    }
    if moves.is_empty() {
        moves.push(some_better.unwrap_or(some_legal));
    }
    moves
}

/// Como `allowed_moves`, mas ordenado da jogada mais promissora para a menos:
/// primeiro pelo histórico, depois por quantas saídas a casa de destino tem.
pub fn ordered_allowed_moves<const S: usize>(board: &Bitboard<S>, i: usize, history: &History) -> MoveList {
    let mut moves = allowed_moves(board, i);
    let head = board.snakes[i].head;
    let mut keys = [0u64; 4];
    for k in 0..moves.len() {
        let mv = moves[k];
        let dest = Bitboard::<S>::moves_from(head)[mv.to_int() as usize].unwrap_or(head);
        let mut options: u64 = 1;
        for next in Bitboard::<S>::moves_from(dest).iter().flatten() {
            let bit = 1u128 << *next;
            let free = board.bodies[0] & bit == 0 && (board.hazard_dmg < 90 || board.hazard & bit == 0);
            options += free as u64;
        }
        for s in &board.snakes {
            if s.is_alive() && s.tail == dest {
                options += 1;
            }
        }
        keys[k] = (history[head as usize][mv.to_int() as usize] as u64) * 16 + options;
    }
    // ordenação por inserção (no máximo 4 elementos), decrescente
    let n = moves.len();
    let slice = moves.as_mut_slice();
    for a in 1..n {
        let mut b = a;
        while b > 0 && keys[b - 1] < keys[b] {
            slice.swap(b - 1, b);
            keys.swap(b - 1, b);
            b -= 1;
        }
    }
    moves
}

/// Gera até 4 combinações de jogadas inimigas de forma que cada jogada de
/// cada inimigo apareça pelo menos uma vez. As cobras com índice `< skip`
/// recebem `Up` (a nossa jogada é preenchida depois pela busca).
///
/// Com um único inimigo isso cobre todas as jogadas dele, então a busca 1x1
/// é um minimax completo. Com vários inimigos é uma aproximação que mantém o
/// fator de ramificação baixo.
pub fn ordered_limited_move_combinations<const S: usize>(
    board: &Bitboard<S>,
    skip: usize,
    history: &History,
) -> MoveCombos<S> {
    let mut combos = MoveCombos::<S>::new([Move::Up; S]);
    combos.push([Move::Up; S]);
    for i in skip..S {
        if board.snakes[i].is_dead() {
            continue;
        }
        let moves = ordered_allowed_moves(board, i, history);
        let n = moves.len().max(combos.len());
        for j in 0..n {
            if combos.len() <= j {
                let first = combos[0];
                combos.push(first);
            }
            combos[j][i] = moves[j.min(moves.len() - 1)];
        }
    }
    combos
}
