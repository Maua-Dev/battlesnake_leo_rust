# 🦀 Battlesnake Rust Template

Template de [Battlesnake](https://play.battlesnake.com) em **Rust**, rodando em
**AWS Lambda** com **API Gateway**. O deploy é automático: você programa, dá
push, e o GitHub Actions devolve a URL da sua cobra.

---

## 📦 Pré-requisitos

- **Rust** (edição 2021, versão estável)
  ```bash
  # Windows, Mac ou Linux
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
  No Windows você também pode usar o instalador de [rustup.rs](https://rustup.rs).
  Confira com `cargo --version`.

- Noções básicas de **Rust**, **API** e **Lambda**
- **Disposição, competitividade e força de vontade!**

Você **não** precisa instalar Terraform nem AWS CLI: quem cuida do deploy é o CD.

---

## 🚀 Como começar

1. Vá até o repositório [**devmaua_setup**](https://github.com/Maua-Dev/devmaua_setup),
   abra uma **issue** e escolha:
   - **project_name**: `battlesnake_rust_{seu nome}`
   - **project template**: `battlesnake_rust_template`
   - marque o repositório como **público**

2. Aguarde cerca de **1 minuto** e confira em
   [Repositórios da organização](https://github.com/orgs/Maua-Dev/repositories).

3. Clone, compile e rode os testes:
   ```bash
   git clone https://github.com/Maua-Dev/Nome_Do_Seu_Repositorio
   cd Nome_Do_Seu_Repositorio
   cargo build
   cargo test
   ```

   O `cargo test` tem que passar antes de você mexer em qualquer coisa — é o
   mesmo comando que o GitHub Actions roda, e é ele que libera o deploy.

4. Abra [`src/logic.rs`](src/logic.rs) e comece a programar sua cobra 🐍

> 🚀 **O deploy acontece por push na branch `dev`.** Push em qualquer outra
> branch roda só os testes e a compilação, sem tocar na AWS.

---

## 🧠 Como esta cobra pensa

A lógica em `src/logic/engine/` é um motor de busca derivado da
[Shapeshifter](https://github.com/JonathanArns/shapeshifter) (campeã mundial
de 2022), reescrito para o tabuleiro 11x11 standard e para rodar dentro de uma
Lambda:

| Arquivo | O que faz |
|---|---|
| `bitboard.rs` | Tabuleiro em `u128` (uma casa por bit) e simulação exata das regras do servidor: cabeças, fome, comida, caudas empilhadas, colisões. Todo o estado é `Copy`: clonar uma posição custa o mesmo que copiar 80 bytes. |
| `movegen.rs` | Jogadas legais, ordenadas por heurística de histórico e por "saídas" da casa de destino. Gera combinações de jogadas inimigas cobrindo cada jogada de cada inimigo. |
| `search.rs` | Minimax paranoico (nós contra todos) com poda alfa-beta, aprofundamento iterativo guiado por *Best Node Search*, tabela de transposição, extensão de sequências forçadas e quiescência quando as cabeças estão perto. Respeita o prazo e devolve a melhor jogada da última profundidade concluída. |
| `eval.rs` | Avaliação por controle de território (Voronoi em bitboard com desempate pela cobra maior), vida, diferença de tamanho (logarítmica), comida controlada, distância da comida e acesso às caudas. Pesos interpolados pelo turno. **Diferença em relação à Shapeshifter:** o Voronoi sabe que cada casa de corpo fica livre quando a cauda passa por ela, então regiões "fechadas" pelo próprio corpo contam certo. Na arena isso levou a taxa de vitória contra a Shapeshifter de 55% para 77%. |
| `endgame.rs` | Resolvedor de finais 1x1: quando cada cobra está na sua região, estima quem fica sem espaço primeiro. **Diferença em relação à Shapeshifter:** o veredito não é tratado como terminal; a busca continua aprofundando e a simulação real prevalece quando a heurística erra (antes, um veredito errado travava a busca na profundidade 1). |
| `ttable.rs` | Tabela de transposição lock-free (64 MB) que sobrevive entre turnos. |

O `src/logic.rs` só faz a cola: gestão de tempo e conversão para JSON.

**Gestão de tempo.** A cobra usa `timeout - margem`. A margem é adaptativa:
compara a latência que o servidor reporta (`you.latency`) com o tempo que a
busca gastou no turno anterior e descobre quanto a rede e a Lambda custam.
`TIME_MARGIN_MS` força uma margem fixa (útil em testes locais).

**Lambda.** A memória está em 1024 MB (cerca de 0,6 vCPU, quatro vezes o padrão de
256 MB do template); a cobra também foi testada em 256 MB e funciona bem. A
tabela de transposição tem 16 MB para não custar tempo numa instância fria, e a
primeira jogada de cada partida numa instância nova usa só metade do prazo,
porque o cold start acontece antes do handler e não entra na nossa contagem.

### Testando localmente contra outra cobra

Sem o linker da Microsoft instalado, o jeito mais simples é compilar dentro
do Docker (a imagem `rust:slim` já serve):

```bash
docker run --rm -v "$PWD:/app" -v cargo-registry:/usr/local/cargo/registry -w /app rust:slim   cargo build --release --bins --target-dir /app/target-docker

docker run -d --name leo-snake -p 8082:8080 -e TIME_MARGIN_MS=60   -v "$PWD/target-docker/release:/snake:ro" rust:slim /snake/local
```

`src/bin/local.rs` é um servidor HTTP mínimo (só biblioteca padrão) que expõe
a mesma lógica da Lambda. Com as duas cobras no ar:

```bash
python tools/arena.py --me http://localhost:8082 --opp http://localhost:8081 -n 20 -j 3
```

O script roda `2n` partidas em pares com a ordem trocada, imprime a taxa de
vitória com intervalo de confiança (Wilson 95%) e guarda as derrotas em
`arena_out/` para você rever. `tools/deaths.py` resume a causa de cada morte.
Para experimentar pesos da avaliação sem recompilar, use `EVAL_WEIGHTS`
(veja `eval.rs`); `TAIL_AWARE=0` desliga o Voronoi ciente das caudas e
`ENEMY_TAIL_DELAY=0` tira o atraso aplicado às caudas inimigas.

Partidas gravadas em `arena_runs/` servem de teste de regressão das regras:
`cargo test --release -- --ignored replay` reproduz cada turno na nossa
simulação e confere que bate com o servidor oficial.

Resultados medidos (40 partidas, 500 ms, Shapeshifter em Docker):

| Versão | V | D | E | Taxa |
|---|---|---|---|---|
| Porta pura da Shapeshifter | 21 | 17 | 2 | 55% |
| + Voronoi ciente das caudas (semente 2000) | 30 | 9 | 1 | 77% |
| + Voronoi ciente das caudas (semente 3000) | 29 | 10 | 1 | 74% |
| + veredito do resolvedor não terminal (semente 2000) | 31 | 9 | 0 | 78% |
| + cauda inimiga com 1 turno de atraso (semente 2000) | 33 | 7 | 0 | 83% |

Contra as 19 cobras de 2026 da organização (10 partidas cada, antes da última
correção): 185 vitórias, 4 derrotas, 1 empate.

---

## 📂 Estrutura do projeto

```
.
├── Cargo.toml                  # dependências e configuração do build
├── src
│   ├── logic.rs                # 👈 É AQUI QUE VOCÊ PROGRAMA
│   ├── models.rs               # structs do estado do jogo (leitura recomendada)
│   └── main.rs                 # ponte com a Lambda — não precisa mexer
├── terraform
│   ├── bootstrap/              # bucket de estado do Terraform
│   └── app/                    # Lambda + API Gateway
└── .github/workflows/CD.yaml   # testes + deploy automático
```

**Você só precisa de `src/logic.rs`.** Os outros arquivos existem para levar o
estado do jogo até as suas quatro funções.

---

## ⭐ Onde implementar sua snake

**Você só precisa editar `src/logic.rs`.** Os outros arquivos existem para
levar o estado do jogo até as suas quatro funções.

### O que você deve alterar:
- `src/logic.rs` — **este é o seu arquivo principal**

### O que você normalmente NÃO precisa alterar:
- `src/main.rs` — ponte com a Lambda
- `src/models.rs` — structs do estado do jogo
- Infraestrutura (Terraform)
- GitHub Actions

---

## 🧠 As quatro funções

Todas ficam em `src/logic.rs` e recebem um `&GameState` (definido em `models.rs`):

| Função | Rota | Quando é chamada | O que devolve |
|---|---|---|---|
| `info()` | `GET /` | ao cadastrar a cobra e no início de cada partida | aparência (cor, cabeça, cauda) |
| `start(state)` | `POST /start` | uma vez, no começo da partida | nada |
| `get_move(state)` | `POST /move` | **a cada turno** | `{"move": "up" \| "down" \| "left" \| "right"}` |
| `end(state)` | `POST /end` | uma vez, no fim da partida | nada |

A cobra já vem com a lógica que **impede ela de andar para trás**. A partir daí,
os `TODO` em `get_move` marcam os próximos passos:

1. não sair do tabuleiro
2. não bater no próprio corpo
3. não bater nas cobras adversárias
4. ir atrás da comida em vez de sortear a direção

Documentação oficial da API: <https://docs.battlesnake.com/api>

> ⏱️ Você tem cerca de **500 ms** por jogada. Se estourar, o servidor escolhe
> uma direção qualquer por você — normalmente para a morte.

---

## 🧪 Testando

```bash
cargo test
```

O template já vem com testes que garantem que a sua cobra **sempre devolve uma
direção válida** e **nunca volta por cima do próprio pescoço**. Escreva mais
testes conforme for implementando os passos acima.

> 🚨 Os testes rodam no GitHub Actions **antes** do deploy. Se algum falhar, o
> deploy não acontece e a URL da sua cobra não é atualizada.

Vale rodar também:

```bash
cargo clippy   # aponta problemas comuns de Rust
cargo fmt      # formata o código
```

### Rodando um servidor local (opcional)

Se quiser bater na sua cobra com `curl` antes de subir, instale o
[cargo-lambda](https://www.cargo-lambda.info/):

```bash
pip install cargo-lambda
cargo lambda watch
```

Ele sobe um emulador da Lambda na porta 9000. A sua função fica em
`http://localhost:9000/lambda-url/battlesnake` (o `battlesnake` do final é o
nome do pacote no `Cargo.toml`). Em outro terminal:

```bash
curl http://localhost:9000/lambda-url/battlesnake/
```

```bash
curl -X POST http://localhost:9000/lambda-url/battlesnake/move -H 'Content-Type: application/json' -d '{"turn":1,"game":{"id":"1","ruleset":{},"timeout":500},"board":{"width":11,"height":11,"food":[],"snakes":[]},"you":{"id":"s1","name":"eu","health":100,"body":[{"x":5,"y":4},{"x":4,"y":4}],"head":{"x":5,"y":4},"length":2}}'
```

Um JSON completo de exemplo está em <https://docs.battlesnake.com/api/example-move>.

---

## ☁️ Deploy

O deploy é disparado por push na branch **`dev`**:

```bash
git add .
git commit -m "minha cobra agora desvia das paredes"
git push origin dev
```

O que o CD faz, nessa ordem:

1. **ExecuteTests** — roda `cargo test`
2. **Bootstrap** — garante o bucket S3 que guarda o estado do Terraform
3. **build_rust** — compila para `x86_64-unknown-linux-musl` e empacota o
   binário `bootstrap` num zip
4. **deploy_app** — `terraform apply`, criando a Lambda e o API Gateway

No fim, o resumo da execução mostra a **URL da sua cobra** e um link para os
logs no CloudWatch. Você também encontra a URL no output `api_url_base` do
passo *Terraform Apply*.

> Rust não tem runtime gerenciado na Lambda. Por isso compilamos um binário
> estático chamado `bootstrap` e subimos no runtime `provided.al2023`. O
> resultado é um cold start bem mais rápido que Java ou Python.

---

## 🎯 Cadastrando na Arena Mauá

1. Acesse [arena.devmaua.com](https://arena.devmaua.com)
2. Faça login com sua conta Mauá
3. No campo **URL**, cole a URL gerada pelo deploy
   (algo como `https://abc123.execute-api.us-east-1.amazonaws.com/dev`)
4. Salve e participe das partidas e do ranking!

Se quiser testar primeiro no site oficial:
1. Entre em [play.battlesnake.com](https://play.battlesnake.com)
2. **My Battlesnakes** → **Create Battlesnake**
3. No campo **URL**, cole a URL do deploy
4. Salve e mande ver!

Se o site reclamar da URL, teste antes no terminal:

```bash
curl https://SUA_URL_AQUI/
```

Deve responder o JSON do `info()`.

---

## 📈 Progressão pedagógica

| Nível | Nome | O que implementar |
|---|---|---|
| 0 | **Random** | movimento aleatório (já vem pronto) |
| 1 | **Don't Die** | não voltar, não bater na parede, não bater em si mesmo |
| 2 | **Food** | procurar comida |
| 3 | **Space** | avaliar espaço disponível, evitar becos |
| 4 | **Opponents** | considerar outras cobras, head-to-head |
| 5 | **Advanced** | BFS, flood fill, A*, avaliação de território |

Comece do Nível 1 implementando os `TODO`s em `src/logic.rs`.

---

## 📌 Observações

- Toda a lógica da partida vive em `get_move()`.
- **Evite adicionar dependências pesadas.** O zip da Lambda tem limite de 50 MB
  e cada dependência aumenta o tempo de compilação no CI.
- Os logs ficam no **CloudWatch**, com retenção de 14 dias. Use `tracing::info!`
  (já importado em `logic.rs`) em vez de `println!` para que eles apareçam
  formatados.
- A branch de deploy é **`dev`**. Push em outras branches roda só os testes.

---

## 🛠 Ferramentas úteis

- [Battlesnake Docs](https://docs.battlesnake.com/) — documentação da API
- [The Rust Book](https://doc.rust-lang.org/book/) — o livro oficial de Rust
- [Rust by Example](https://doc.rust-lang.org/rust-by-example/) — Rust na prática
- [serde_json](https://docs.rs/serde_json/) — manipulação de JSON
- [cargo-lambda](https://www.cargo-lambda.info/) — rodar a Lambda localmente

---

## 📞 Fale com a gente

Dúvidas? Chama no [Discord](https://discord.gg/Yr2VPgAmcb) da Dev. Community Mauá.
