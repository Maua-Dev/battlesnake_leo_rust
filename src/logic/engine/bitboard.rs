//! Tabuleiro em bitboard para o modo standard 11x11.
//!
//! Cada casa do tabuleiro é um bit de um `u128` (121 casas cabem em 128 bits).
//! O índice de uma casa é `y * 11 + x`, então andar para a direita é `+1`,
//! para a esquerda `-1`, para cima `+11` e para baixo `-11`.
//!
//! A representação é derivada da Shapeshifter (Jonathan Arns), simplificada
//! para um único modo de jogo: assim o tabuleiro inteiro é `Copy`, sem
//! alocação e sem despacho dinâmico, o que deixa a busca bem mais rápida.

use crate::models::{Battlesnake, GameState};

pub const W: usize = 11;
pub const H: usize = 11;
pub const N: usize = W * H;

pub type Bits = u128;

/// Todas as casas do tabuleiro.
pub const FULL: Bits = (1u128 << N) - 1;

/// Casas que NÃO estão na coluna da direita (podem andar para a direita).
pub const NOT_RIGHT_EDGE: Bits = {
    let mut m = 0u128;
    let mut i = 0;
    while i < N {
        if i % W != W - 1 {
            m |= 1u128 << i;
        }
        i += 1;
    }
    m
};

/// Casas que NÃO estão na coluna da esquerda (podem andar para a esquerda).
pub const NOT_LEFT_EDGE: Bits = {
    let mut m = 0u128;
    let mut i = 0;
    while i < N {
        if i % W != 0 {
            m |= 1u128 << i;
        }
        i += 1;
    }
    m
};

/// Padrão de tabuleiro de xadrez (casas "pretas"). Uma cobra alterna de cor a
/// cada passo, então uma região só é totalmente aproveitável se tiver um
/// número parecido de casas de cada cor.
pub const CHECKER: Bits = {
    let mut m = 0u128;
    let mut i = 0;
    while i < N {
        if i % 2 == 0 {
            m |= 1u128 << i;
        }
        i += 1;
    }
    m
};

/// Para cada casa, o destino de cada uma das 4 direções (ou `None` se sair
/// do tabuleiro). Índice da direção = `Move as usize`.
pub const MOVE_TABLE: [[Option<u8>; 4]; N] = {
    let mut t = [[None; 4]; N];
    let mut pos = 0;
    while pos < N {
        if pos < W * (H - 1) {
            t[pos][0] = Some((pos + W) as u8);
        }
        if pos >= W {
            t[pos][1] = Some((pos - W) as u8);
        }
        if pos % W < W - 1 {
            t[pos][2] = Some((pos + 1) as u8);
        }
        if pos % W > 0 {
            t[pos][3] = Some((pos - 1) as u8);
        }
        pos += 1;
    }
    t
};

/// Expande um conjunto de casas para incluir todos os vizinhos ortogonais.
#[inline(always)]
pub fn expand(s: Bits) -> Bits {
    (s | ((s & NOT_RIGHT_EDGE) << 1) | ((s & NOT_LEFT_EDGE) >> 1) | (s << W) | (s >> W)) & FULL
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Move {
    #[default]
    Up = 0,
    Down = 1,
    Right = 2,
    Left = 3,
}

impl Move {
    pub const ALL: [Move; 4] = [Move::Up, Move::Down, Move::Right, Move::Left];

    #[inline(always)]
    pub const fn to_int(self) -> u8 {
        self as u8
    }

    #[inline(always)]
    pub const fn from_int(x: u8) -> Move {
        match x & 3 {
            0 => Move::Up,
            1 => Move::Down,
            2 => Move::Right,
            _ => Move::Left,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Move::Up => "up",
            Move::Down => "down",
            Move::Right => "right",
            Move::Left => "left",
        }
    }

    /// Deslocamento no índice linear ao andar nessa direção.
    #[inline(always)]
    pub const fn delta(self) -> i16 {
        match self {
            Move::Up => W as i16,
            Move::Down => -(W as i16),
            Move::Right => 1,
            Move::Left => -1,
        }
    }

    #[inline(always)]
    pub const fn delta_from_int(x: u8) -> i16 {
        Move::from_int(x).delta()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Snake {
    pub head: u8,
    pub tail: u8,
    pub length: u8,
    pub health: i8,
    /// Segmentos empilhados na cauda (depois de comer, a cauda "fica parada").
    pub curled: u8,
}

impl Snake {
    #[inline(always)]
    pub fn is_alive(&self) -> bool {
        self.health > 0
    }

    #[inline(always)]
    pub fn is_dead(&self) -> bool {
        self.health <= 0
    }
}

/// Estado completo da partida. `S` é o número de cobras; a cobra de índice 0
/// é sempre a nossa.
#[derive(Clone, Copy, Debug)]
pub struct Bitboard<const S: usize> {
    /// `bodies[0]`: casas ocupadas por corpos (sem a cauda, se ela vai sair).
    /// `bodies[1]` e `bodies[2]`: bits da direção de cada segmento até o
    /// próximo segmento no sentido da cabeça (`bit0 | bit1 << 1` = `Move`).
    pub bodies: [Bits; 3],
    pub snakes: [Snake; S],
    pub food: Bits,
    pub hazard: Bits,
    pub hazard_dmg: i8,
    pub turn: u16,
}

#[inline(always)]
fn mix(h: u64, v: u64) -> u64 {
    (h.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95)
}

impl<const S: usize> Bitboard<S> {
    pub fn empty() -> Self {
        Bitboard {
            bodies: [0; 3],
            snakes: [Snake { head: 0, tail: 0, length: 0, health: 0, curled: 0 }; S],
            food: 0,
            hazard: 0,
            hazard_dmg: 14,
            turn: 0,
        }
    }

    /// Monta o tabuleiro a partir do estado da API. `snakes` deve vir com a
    /// nossa cobra na posição 0 e ter exatamente `S` elementos.
    pub fn from_state(state: &GameState, snakes: &[&Battlesnake]) -> Self {
        debug_assert_eq!(snakes.len(), S);
        let mut board = Self::empty();
        board.turn = state.turn.clamp(0, u16::MAX as i32) as u16;

        if let Some(settings) = state.game.ruleset.get("settings") {
            if let Some(dmg) = settings.get("hazardDamagePerTurn").and_then(|v| v.as_i64()) {
                board.hazard_dmg = dmg.clamp(0, 100) as i8;
            }
        }
        for f in &state.board.food {
            if let Some(i) = idx(f.x, f.y) {
                board.food |= 1u128 << i;
            }
        }
        for hz in &state.board.hazards {
            if let Some(i) = idx(hz.x, hz.y) {
                board.hazard |= 1u128 << i;
            }
        }

        for (n, snake) in snakes.iter().enumerate().take(S) {
            let body: Vec<u8> = snake.body.iter().filter_map(|c| idx(c.x, c.y)).collect();
            if body.is_empty() {
                continue;
            }
            let s = &mut board.snakes[n];
            s.health = snake.health.clamp(0, 100) as i8;
            s.length = snake.length.clamp(0, 255) as u8;
            s.head = body[0];
            s.tail = *body.last().unwrap();
            board.bodies[0] |= 1u128 << s.head;

            let mut prev = s.head;
            for &pos in &body[1..] {
                if pos == prev {
                    s.curled += 1;
                    continue;
                }
                board.bodies[0] |= 1u128 << pos;
                // direção de `pos` para `prev` (o segmento mais próximo da cabeça)
                if prev == pos.wrapping_sub(1) || prev == pos.wrapping_sub(W as u8) {
                    // andou para a esquerda (3) ou para baixo (1): bit0 ligado
                    board.bodies[1] |= 1u128 << pos;
                }
                if prev == pos.wrapping_sub(1) || prev == pos + 1 {
                    // andou na horizontal (2 ou 3): bit1 ligado
                    board.bodies[2] |= 1u128 << pos;
                }
                prev = pos;
            }
            // Se a API disser que a cobra é maior do que o corpo que recebemos
            // (coordenadas inválidas, por exemplo), tratamos a diferença como
            // segmentos empilhados: a cauda fica parada e nada é liberado.
            s.curled += s.length.saturating_sub(body.len() as u8);
            if s.curled == 0 {
                // A cauda vai sair no próximo turno, então a casa é livre.
                board.bodies[0] &= !(1u128 << s.tail);
            }
        }
        board
    }

    /// Verdadeiro se nós morremos ou se somos a única cobra viva.
    #[inline]
    pub fn is_terminal(&self) -> bool {
        if self.snakes[0].is_dead() {
            return true;
        }
        for i in 1..S {
            if self.snakes[i].is_alive() {
                return false;
            }
        }
        true
    }

    #[inline(always)]
    pub fn distance(a: u8, b: u8) -> u16 {
        let (ax, ay) = ((a as usize % W) as i16, (a as usize / W) as i16);
        let (bx, by) = ((b as usize % W) as i16, (b as usize / W) as i16);
        ((ax - bx).abs() + (ay - by).abs()) as u16
    }

    /// Próximo segmento do corpo (no sentido da cabeça). Não confere se `pos`
    /// realmente está em cima de uma cobra.
    #[inline(always)]
    pub fn next_body_segment(&self, pos: u8) -> u8 {
        let mv = ((self.bodies[1] >> pos) & 1) as u8 | (((self.bodies[2] >> pos) & 1) as u8) << 1;
        (pos as i16 + Move::delta_from_int(mv)) as u8
    }

    pub fn kill_snake(&mut self, i: usize) {
        self.snakes[i].health = -1;
        self.remove_snake_body(i);
    }

    fn remove_snake_body(&mut self, i: usize) {
        if S <= 2 || self.snakes[0].is_dead() {
            return; // estado terminal, o corpo não importa mais
        }
        let snake = self.snakes[i];
        let mut pos = snake.tail;
        let mut guard = 0;
        while pos != snake.head && guard < N {
            guard += 1;
            let next = self.next_body_segment(pos);
            let clear = !(1u128 << pos);
            self.bodies[0] &= clear;
            self.bodies[1] &= clear;
            self.bodies[2] &= clear;
            pos = next;
        }
        self.bodies[0] &= !(1u128 << snake.head);
    }

    /// Hash do estado para a tabela de transposição. Não inclui o turno, para
    /// que as entradas continuem úteis de um turno para o outro.
    pub fn hash(&self) -> u64 {
        let mut h = 0x9e37_79b9_7f4a_7c15u64;
        for b in self.bodies {
            h = mix(h, b as u64);
            h = mix(h, (b >> 64) as u64);
        }
        h = mix(h, self.food as u64);
        h = mix(h, (self.food >> 64) as u64);
        h = mix(h, self.hazard as u64 ^ (self.hazard >> 64) as u64);
        for s in &self.snakes {
            if s.is_alive() {
                let v = s.head as u64
                    | (s.tail as u64) << 8
                    | (s.length as u64) << 16
                    | (s.health as u8 as u64) << 24
                    | (s.curled as u64) << 32;
                h = mix(h, v);
            }
        }
        h
    }

    pub fn hash_with_move(&self, mv: Move) -> u64 {
        mix(self.hash(), 0xabcd_0000 | mv.to_int() as u64)
    }

    // ----------------------------------------------------------------------
    // Regras do jogo (modo standard). Mesma ordem do servidor oficial:
    // mover cabeças, reduzir vida / comer, mover caudas, colisões.
    // ----------------------------------------------------------------------

    pub fn apply_moves(&mut self, moves: &[Move; S]) {
        self.turn = self.turn.wrapping_add(1);
        self.move_heads(moves);
        self.update_health();
        self.move_tails();
        self.perform_collisions();
        self.finish_head_movement();
        self.finish_tail_movement();
    }

    #[inline]
    fn move_heads(&mut self, moves: &[Move; S]) {
        for i in 0..S {
            if self.snakes[i].is_dead() {
                continue;
            }
            let mv = moves[i].to_int();
            let pos = self.snakes[i].head;
            // registra a direção do novo segmento (a cabeça antiga vira corpo)
            let bit = 1u128 << pos;
            if mv & 1 != 0 {
                self.bodies[1] |= bit;
            } else {
                self.bodies[1] &= !bit;
            }
            if mv >> 1 != 0 {
                self.bodies[2] |= bit;
            } else {
                self.bodies[2] &= !bit;
            }
            match MOVE_TABLE[pos as usize][mv as usize] {
                Some(new_head) => self.snakes[i].head = new_head,
                None => self.kill_snake(i), // saiu do tabuleiro
            }
        }
    }

    #[inline]
    fn update_health(&mut self) {
        let mut eaten: Bits = 0;
        for i in 0..S {
            if self.snakes[i].is_dead() {
                continue;
            }
            let head_bit = 1u128 << self.snakes[i].head;
            let dmg = 1 + if self.hazard & head_bit != 0 { self.hazard_dmg } else { 0 };
            self.snakes[i].health -= dmg;
            if self.food & head_bit != 0 {
                let s = &mut self.snakes[i];
                s.health = 100;
                s.curled += 1;
                s.length = s.length.saturating_add(1);
                eaten |= head_bit;
            }
            if self.snakes[i].health <= 0 {
                self.kill_snake(i); // morreu de fome
            }
        }
        self.food &= !eaten;
    }

    #[inline]
    fn move_tails(&mut self) {
        for i in 0..S {
            let snake = self.snakes[i];
            if snake.is_dead() {
                continue;
            }
            if snake.curled == 0 || (snake.curled == 1 && snake.health == 100) {
                let tail = snake.tail;
                let mv = ((self.bodies[1] >> tail) & 1) as u8 | (((self.bodies[2] >> tail) & 1) as u8) << 1;
                let clear = !(1u128 << tail);
                self.bodies[0] &= clear;
                self.bodies[1] &= clear;
                self.bodies[2] &= clear;
                self.snakes[i].tail = (tail as i16 + Move::delta_from_int(mv)) as u8;
            } else {
                self.snakes[i].curled -= 1;
            }
        }
    }

    #[inline]
    fn perform_collisions(&mut self) {
        let mut dead = [false; S];
        'outer: for i in 0..S {
            if self.snakes[i].is_dead() {
                continue;
            }
            let head = self.snakes[i].head;
            if self.bodies[0] & (1u128 << head) != 0 {
                dead[i] = true;
                continue;
            }
            for j in 0..S {
                if i != j
                    && self.snakes[j].is_alive()
                    && self.snakes[j].head == head
                    && self.snakes[i].length <= self.snakes[j].length
                {
                    dead[i] = true;
                    continue 'outer;
                }
            }
        }
        for i in 0..S {
            if dead[i] {
                self.kill_snake(i);
            }
        }
    }

    #[inline]
    fn finish_head_movement(&mut self) {
        for i in 0..S {
            if self.snakes[i].is_alive() {
                self.bodies[0] |= 1u128 << self.snakes[i].head;
            }
        }
    }

    #[inline]
    fn finish_tail_movement(&mut self) {
        for i in 0..S {
            if self.snakes[i].is_alive() && self.snakes[i].curled == 0 {
                self.bodies[0] &= !(1u128 << self.snakes[i].tail);
            }
        }
    }

    /// Casas que a cobra `i` pode ocupar com cada direção (destinos válidos).
    #[inline(always)]
    pub fn moves_from(pos: u8) -> &'static [Option<u8>; 4] {
        &MOVE_TABLE[pos as usize]
    }
}

/// Índice linear de uma coordenada, ou `None` se estiver fora do tabuleiro.
#[inline]
pub fn idx(x: i32, y: i32) -> Option<u8> {
    if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
        None
    } else {
        Some((y as usize * W + x as usize) as u8)
    }
}

impl<const S: usize> std::fmt::Display for Bitboard<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for row in (0..H).rev() {
            for col in 0..W {
                let i = row * W + col;
                let bit = 1u128 << i;
                let mut c = '.';
                if self.food & bit != 0 {
                    c = 'f';
                }
                if self.hazard & bit != 0 {
                    c = '~';
                }
                if self.bodies[0] & bit != 0 {
                    c = 'x';
                }
                for (n, s) in self.snakes.iter().enumerate() {
                    if s.is_alive() && s.head as usize == i {
                        c = if n == 0 { '@' } else { 'E' };
                    }
                }
                write!(f, "{c} ")?;
            }
            writeln!(f)?;
        }
        for s in &self.snakes {
            writeln!(
                f,
                "head {} tail {} len {} hp {} curled {}",
                s.head, s.tail, s.length, s.health, s.curled
            )?;
        }
        writeln!(f, "turn {}", self.turn)
    }
}
