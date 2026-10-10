// Bem-vindo ao
// __________         __    __  .__                               __
// \______   \_____ _/  |__/  |_|  |   ____   ______ ____ _____  |  | __ ____
//  |    |  _/\__  \   __\   __\  | _/ __ \ /  ___//    \__  \ |  |/ // __ \
//  |    |   \ / __ \|  |  |  | |  |_\  ___/ \___ \|   |  \/ __ \|    <\  ___/
//  |________/(______/__|  |__| |____/\_____>______>___|__(______/__|__\_____>
//
// Este arquivo é a cola entre a API do Battlesnake e o motor de busca que
// fica em `src/logic/engine/`. A inteligência de verdade está lá:
//
//   bitboard.rs  tabuleiro em u128 e regras do jogo (simulação de turnos)
//   movegen.rs   geração e ordenação de jogadas
//   search.rs    minimax alfa-beta com aprofundamento iterativo
//   eval.rs      avaliação de posição (território, vida, tamanho, comida)
//   endgame.rs   resolvedor de finais 1x1
//   ttable.rs    tabela de transposição
//
// Documentação da API: https://docs.battlesnake.com

#[path = "logic/engine/mod.rs"]
mod engine;

use crate::models::GameState;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tracing::info;

/// GET / — aparência da cobra.
/// Opções de cabeça, cauda e cor: https://docs.battlesnake.com/guides/customizations
pub fn info() -> Value {
    info!("INFO");

    json!({
        "apiversion": "1",
        "author": "leoba",
        "color": "#1f6f8b",
        "head": "evil",
        "tail": "bolt",
        "version": "2.0.0"
    })
}

/// POST /start — chamado uma vez, quando a partida começa.
pub fn start(state: &GameState) {
    info!("JOGO COMEÇOU (partida {})", state.game.id);
    engine::warm_up();
}

/// POST /end — chamado uma vez, quando a partida termina.
pub fn end(state: &GameState) {
    let venceu = state.board.snakes.iter().any(|s| s.id == state.you.id);
    info!(
        "FIM DE JOGO após {} turnos ({})",
        state.turn,
        if venceu { "vitória" } else { "derrota" }
    );
    if let Ok(mut m) = think_times().lock() {
        m.remove(&game_key(state));
    }
}

/// POST /move — chamado a cada turno. Devolve "up", "down", "left" ou "right".
pub fn get_move(state: &GameState) -> Value {
    let start = Instant::now();

    if !engine::supports(state) || state.you.body.is_empty() {
        let mv = fallback_move(state);
        info!("MOVE {}: {} (fallback: tabuleiro {}x{})", state.turn, mv, state.board.width, state.board.height);
        return json!({ "move": mv });
    }

    let budget = time_budget(state);
    let result = engine::choose_move(state, start + budget);
    let elapsed = start.elapsed();
    remember_think_time(state, elapsed);

    info!(
        "MOVE {}: {} (depth {}, score {}, {} nós, {} ms de {} ms)",
        state.turn,
        result.mv.as_str(),
        result.depth,
        result.score,
        result.nodes,
        elapsed.as_millis(),
        budget.as_millis()
    );

    json!({
        "move": result.mv.as_str(),
        "shout": format!("d{} s{} n{}", result.depth, result.score, result.nodes),
    })
}

// ---------------------------------------------------------------------------
// Gestão de tempo
// ---------------------------------------------------------------------------

/// Tempo de cálculo da nossa última jogada em cada partida. Comparando com a
/// latência que o servidor reporta (`you.latency`) descobrimos quanto tempo
/// a rede e a Lambda gastam, e ajustamos a margem de segurança.
fn think_times() -> &'static Mutex<HashMap<String, u32>> {
    static M: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

fn game_key(state: &GameState) -> String {
    format!("{}:{}", state.game.id, state.you.id)
}

fn remember_think_time(state: &GameState, elapsed: Duration) {
    if let Ok(mut m) = think_times().lock() {
        if m.len() > 512 {
            m.clear();
        }
        m.insert(game_key(state), elapsed.as_millis().min(u32::MAX as u128) as u32);
    }
}

/// Quanto tempo a busca pode usar neste turno.
///
/// `timeout` é o limite do servidor para a resposta chegar. Descontamos a
/// sobrecarga medida (latência reportada menos o nosso tempo de cálculo
/// anterior) mais uma folga. A variável de ambiente `TIME_MARGIN_MS` força
/// uma margem fixa (útil em testes locais).
fn time_budget(state: &GameState) -> Duration {
    let timeout = if state.game.timeout == 0 { 500 } else { state.game.timeout };
    let margin = fixed_margin().unwrap_or_else(|| adaptive_margin(state, timeout));
    let margin = margin.min(timeout / 2);
    let mut budget = (timeout - margin).max(40);
    // Primeira jogada desta partida nesta instância: pode ser um cold start da
    // Lambda, cujo tempo de inicialização acontece antes do handler e não
    // aparece na nossa contagem. Usamos só metade do prazo por segurança.
    if fixed_margin().is_none() && !seen_game(state) {
        budget = budget.min(timeout / 2);
    }
    Duration::from_millis(budget as u64)
}

fn seen_game(state: &GameState) -> bool {
    think_times().lock().map(|m| m.contains_key(&game_key(state))).unwrap_or(false)
}

fn fixed_margin() -> Option<u32> {
    static V: OnceLock<Option<u32>> = OnceLock::new();
    *V.get_or_init(|| std::env::var("TIME_MARGIN_MS").ok().and_then(|s| s.parse().ok()))
}

const DEFAULT_MARGIN_MS: u32 = 110;
const MIN_MARGIN_MS: u32 = 60;
const SAFETY_MS: u32 = 45;

fn adaptive_margin(state: &GameState, timeout: u32) -> u32 {
    let reported = state.you.latency.as_deref().and_then(|s| s.trim().parse::<u32>().ok());
    let previous = think_times().lock().ok().and_then(|m| m.get(&game_key(state)).copied());
    match (reported, previous) {
        (Some(lat), Some(prev)) if lat > 0 => {
            let overhead = lat.saturating_sub(prev);
            (overhead + SAFETY_MS).clamp(MIN_MARGIN_MS, timeout / 2)
        }
        _ => DEFAULT_MARGIN_MS,
    }
}

// ---------------------------------------------------------------------------
// Fallback: só para tabuleiros que o motor não conhece
// ---------------------------------------------------------------------------

/// Escolha simples (não voltar, não sair do tabuleiro, não bater em corpos).
fn fallback_move(state: &GameState) -> &'static str {
    let Some(head) = state.you.body.first() else { return "up" };
    let dirs: [(&'static str, i32, i32); 4] = [("up", 0, 1), ("down", 0, -1), ("left", -1, 0), ("right", 1, 0)];
    let mut best = "up";
    let mut best_score = i32::MIN;
    for (name, dx, dy) in dirs {
        let (x, y) = (head.x + dx, head.y + dy);
        let mut score = 0;
        if x < 0 || y < 0 || x >= state.board.width || y >= state.board.height {
            score -= 1000;
        }
        for s in &state.board.snakes {
            let n = s.body.len();
            for (i, c) in s.body.iter().enumerate() {
                let is_tail = i + 1 == n && n > 1;
                if c.x == x && c.y == y && !is_tail {
                    score -= 1000;
                }
            }
        }
        if let Some(neck) = state.you.body.get(1) {
            if neck.x == x && neck.y == y {
                score -= 1000;
            }
        }
        if let Some(f) = state.board.food.iter().map(|f| (f.x - x).abs() + (f.y - y).abs()).min() {
            score -= f;
        }
        if score > best_score {
            best_score = score;
            best = name;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Battlesnake, Board, Coord, Game};

    /// Monta um estado de jogo mínimo para os testes, com a cobra deitada
    /// na horizontal: cabeça em `head` e pescoço em `neck`.
    fn game_state(head: Coord, neck: Coord) -> GameState {
        let you = Battlesnake {
            id: "minha-cobra".to_string(),
            name: "MinhaCobra".to_string(),
            health: 100,
            body: vec![head, neck, Coord { x: neck.x, y: neck.y - 1 }],
            head,
            length: 3,
            latency: Some("50".to_string()),
            shout: None,
        };

        GameState {
            game: Game {
                id: "partida-de-teste".to_string(),
                ruleset: HashMap::new(),
                map: Some("standard".to_string()),
                timeout: 500,
            },
            turn: 4,
            board: Board {
                height: 11,
                width: 11,
                food: vec![Coord { x: 5, y: 5 }],
                hazards: vec![],
                snakes: vec![you.clone()],
            },
            you,
        }
    }

    fn chosen_move(state: &GameState) -> String {
        get_move(state)["move"].as_str().unwrap().to_string()
    }

    #[test]
    fn info_devolve_os_campos_obrigatorios() {
        let response = info();

        assert_eq!(response["apiversion"], "1");
        assert!(response.get("author").is_some());
        assert!(response.get("color").is_some());
        assert!(response.get("head").is_some());
        assert!(response.get("tail").is_some());
    }

    #[test]
    fn move_devolve_sempre_uma_direcao_valida() {
        let state = game_state(Coord { x: 5, y: 4 }, Coord { x: 4, y: 4 });
        for _ in 0..3 {
            let direction = chosen_move(&state);
            assert!(
                ["up", "down", "left", "right"].contains(&direction.as_str()),
                "direção inválida: {direction}"
            );
        }
    }

    #[test]
    fn nunca_volta_por_cima_do_pescoco() {
        let state = game_state(Coord { x: 5, y: 4 }, Coord { x: 4, y: 4 });
        assert_ne!(chosen_move(&state), "left");

        let state = game_state(Coord { x: 5, y: 4 }, Coord { x: 6, y: 4 });
        assert_ne!(chosen_move(&state), "right");

        let state = game_state(Coord { x: 5, y: 4 }, Coord { x: 5, y: 3 });
        assert_ne!(chosen_move(&state), "down");

        let state = game_state(Coord { x: 5, y: 4 }, Coord { x: 5, y: 5 });
        assert_ne!(chosen_move(&state), "up");
    }

    #[test]
    fn evita_parede_quando_tem_opcao() {
        // Canto inferior esquerdo, pescoço à direita: só "up" é seguro.
        let state = game_state(Coord { x: 0, y: 0 }, Coord { x: 1, y: 0 });
        assert_eq!(chosen_move(&state), "up");
    }

    #[test]
    fn evita_proprio_corpo_quando_tem_opcao() {
        // Cabeça em (5,4), pescoço à esquerda (4,4), corpo acima em (5,5).
        let head = Coord { x: 5, y: 4 };
        let neck = Coord { x: 4, y: 4 };
        let mut state = game_state(head, neck);
        state.you.body = vec![head, neck, Coord { x: 5, y: 5 }, Coord { x: 4, y: 3 }];
        state.you.length = 4;
        state.board.snakes = vec![state.you.clone()];

        let direction = chosen_move(&state);
        assert!(["right", "down"].contains(&direction.as_str()), "escolheu {direction}");
    }

    #[test]
    fn comportamento_definido_sem_safe_moves() {
        // Canto (0,0), pescoço acima, corpo à direita: não há saída.
        // Não pode entrar em pânico e tem que devolver uma direção válida.
        let head = Coord { x: 0, y: 0 };
        let neck = Coord { x: 0, y: 1 };
        let mut state = game_state(head, neck);
        state.you.body.push(Coord { x: 1, y: 0 });
        state.you.length = 4;
        state.board.snakes = vec![state.you.clone()];

        let direction = chosen_move(&state);
        assert!(
            ["up", "down", "left", "right"].contains(&direction.as_str()),
            "fallback retornou direção inválida: {direction}"
        );
    }

    #[test]
    fn vai_atras_da_comida_quando_esta_com_fome() {
        let mut state = game_state(Coord { x: 5, y: 4 }, Coord { x: 4, y: 4 });
        state.you.health = 8;
        state.board.snakes[0].health = 8;
        state.board.food = vec![Coord { x: 5, y: 7 }];
        assert_eq!(chosen_move(&state), "up");
    }

    #[test]
    fn tabuleiro_de_outro_tamanho_usa_o_fallback() {
        let mut state = game_state(Coord { x: 5, y: 4 }, Coord { x: 4, y: 4 });
        state.board.width = 7;
        state.board.height = 7;
        let direction = chosen_move(&state);
        assert!(["up", "down", "right"].contains(&direction.as_str()));
    }

    #[test]
    fn respeita_o_orcamento_de_tempo() {
        let mut state = game_state(Coord { x: 5, y: 4 }, Coord { x: 4, y: 4 });
        state.game.timeout = 200;
        let start = Instant::now();
        let _ = chosen_move(&state);
        assert!(start.elapsed() < Duration::from_millis(200), "estourou o tempo");
    }
}
