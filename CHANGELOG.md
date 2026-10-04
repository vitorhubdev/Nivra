# Changelog

## Nivra 1.0.11

Esta versão corrige o vídeo que travava o aplicativo, publica a suíte dos 3 pilares de voz com números medidos, fecha os itens da auditoria da 1.0.10 e entrega as cinco funções aprovadas pelo dono: Push to Mute, ícone de voz na bandeja, layout compacto, prévia HEIC e enquetes.

### Vídeo

- Tocar um vídeo não trava mais o Nivra. A causa era um re-lock do contexto do egui dentro do `widget_info` (deadlock): o texto agora é calculado antes, e um watchdog registra em `nivra-freeze.log` qualquer intervalo de quadros acima de 2 s, sem nunca bloquear a UI.
- Clipes sem áudio (ou com áudio mais curto que o vídeo) tocam até o fim: o relógio de mídia passou a ancorar no relógio monotônico e não joga mais tempo fora em leituras lentas.
- O cartão abre mostrando o primeiro quadro (poster) e o Play inicia a reprodução; falha de decodificação mostra o motivo com "Tentar de novo", "Baixar vídeo" e "Abrir no original".
- Workers e logs de mídia limitados por prazo, primeiro quadro com deadline e primeiro byte real medido (#73); vídeo preso em "Carregando" agora termina em erro com retry, e arrastar a barra confirma o salto ao soltar.

### Os 3 pilares de voz

Pilar 1 — supressão de ruído (orçamento de 10 ms por quadro; aprovado com p99 < 5000 µs e zero estouros):

| Modo | p50 (µs) | p99 (µs) | Estouros | Maior salto |
| --- | --- | --- | --- | --- |
| Desligado | 0 | 0 | 0 | 0.1382 |
| Leve (WebRtc) | 50 | 58 | 0 | 0.0475 |
| Padrão (RNNoise) | 55 | 90 | 0 | 0.0619 |
| Máxima (DeepFilterNet, worker sintético) | 97 | 138 | 0 | 0.1105 |
| Troca de modo no meio | — | — | — | 0.0836 (sem estalo) |

No nível Máxima o teste usa um worker sintético (400 µs fixos, sem carregar o modelo real); os outros três usam o processamento real. A prova é o orçamento e a continuidade do pipeline, não o tempo do modelo real.

- Pilar 2 — falar e ouvir: dois clientes sintéticos trocam áudio decodificado com DAVE/MLS ligado (tons de 450/650 Hz, pelo menos 60 quadros por sentido, tom alvo > 10× o outro); um blackout de UDP de 10 s volta sozinho sem rejoin; troca de dispositivo com a chamada aberta; estresse de 50 ciclos de entrar/sair e 20 trocas de canal.
- Pilar 3 — avisar tudo: cada `VOICE_STATE_UPDATE` pinta o ícone certo em um frame (teste com egui_kittest); sons de entrar/sair, mutar/desmutar e ensurdecer passam por filas ordenadas, com deduplicação por identidade, que mantêm os avisos mais recentes quando a fila enche (o item mais antigo é descartado de propósito).

### Correções da auditoria

- UDP de voz não suspende mais o loop da chamada (`try_send` com falha injetável); perda em rajada salta para o próximo pacote do jitter em vez de limpar a fila inteira; tick atrasado descarta o backlog do mixer; o rekey do DAVE ganhou prazo de 30 s e o prazo é desarmado após sucesso; a fila de sons mantém o aviso mais novo.
- Exportação HTML: a inicial do avatar é escapada e `href`/`src` só aceitam http(s); `javascript:` vira texto inerte.
- READY: guildas descartadas contam para o teto de entradas; uma guilda malformada derruba só a própria entrada e o login continua com o aviso.
- Anexos com link de cinco segmentos (`/attachments/{canal}/{mensagem}/{anexo}/{arquivo}`) voltam a resolver; mute de DM observado limpa o timer velho sem ressuscitar no desmutar; textos de atualização traduzidos nos 3 idiomas.
- Instância única: teste de corrida prova que duas aberturas simultâneas têm um único vencedor.

### Funções aprovadas

- Push to Mute nos botões laterais do mouse (4/5): ação desatribuída por padrão, funciona com a janela focada e, no Windows, global via poller; mudar o estado acorda a UI na hora.
- Ícone de voz na bandeja (Windows): mutado, ensurdecido, ativo e fora de chamada, com os quatro estados cobertos por teste e prévias em PNG; no macOS e Linux a bandeja mantém o comportamento anterior.
- Layout compacto da timeline (hora | autor | texto) em Configurações > Aparência, cobrindo linhas normais, starter de thread e mensagens pendentes.
- Prévia de HEIC do iPhone via decodificador do Windows (WIC), com o orçamento de pixels respeitado, orientação EXIF aplicada e fallback "Baixar" onde não houver codec.
- Enquetes do Discord: ver, votar, remover o voto, resultado ao vivo e enquete encerrada, no lugar do marcador antigo. Limitação honesta: o voto usa a rota de usuário `PUT /channels/{canal}/polls/{mensagem}/answers/@me`, que é não oficial e não foi validada em conta real.

### English

1.0.11 fixes the video freeze, ships the measured voice-pillar suite, closes the 1.0.10 audit items and delivers the five approved features. Playing a video no longer deadlocks the app (egui context re-lock inside `widget_info`); audio-less clips play to the end with a monotonic media clock; the card shows the first frame before Play; decode failures offer retry/download/open-original; and a render-stall watchdog writes a bounded `nivra-freeze.log` after 2 s without blocking the UI.

Voice pillars, measured: noise suppression stays under a 5000 µs p99 per 10 ms frame with zero budget blowouts (Off 0/0, Light/WebRtc 50/58, Standard/RNNoise 55/90, Maximum/DeepFilter 97/138 µs p50/p99 — the Maximum figure measures the synthetic worker path, not the real model; 0.0836 maximum crossfade step); two synthetic clients exchange decoded DAVE-encrypted audio (450/650 Hz, at least 60 frames per direction), survive a 10 s UDP blackout without rejoining, follow a device switch during a call, and pass a 50× join/leave and 20× channel-switch stress; every voice-state update paints the right icon in one frame with ordered, identity-deduplicated cue queues that keep the newest bounded set when full.

Audit fixes: non-blocking voice UDP, burst-loss jitter recovery, stale-mix drop, a 30 s DAVE rekey deadline disarmed after success, escaped HTML export (no `javascript:` URLs), bounded READY entries with per-guild login survival, five-segment attachment links, video stall and drag-seek fixes, the observed DM mute timer, translated update strings, and a race test proving a single instance-lock winner.

Approved features: Push to Mute on mouse 4/5 (unassigned by default, global on Windows), the Windows tray voice-state icon (muted, deafened, connected, idle) with PNG proofs (macOS and Linux keep the previous tray behavior), compact timeline layout including thread starters and pending rows, Windows WIC HEIC preview with EXIF orientation and download fallback, and Discord polls (view, vote, unvote, live results, closed state). Honest limitation: poll voting uses the unofficial normal-user route and has not been validated against a real account.

## Nivra 1.0.10

Esta versão traz estabilidade aprimorada para chamadas de voz, nova experiência de download e seleção de mensagens, refinamento visual e correções importantes:

- **Supressão de ruído sem picote (Pilar 1 de Voz):** O modo Máximo (DeepFilterNet) e os modos Padrão/Médio (RNNoise) receberam crossfade contínuo de 10 ms e rampa de atenuação no Echo, eliminando qualquer estalo ou corte na troca de modos. O modelo agora opera com piso acústico de -26 dB preservando a clareza de consoantes e cauda de fonemas, e conta com controle deslizante de intensidade máxima ajustável (0-100%) nas configurações de voz.
- **Chamadas estáveis e saída de áudio unificada (Pilar 2 de Voz):** A saída configurada em Configurações > Voz passa a ser utilizada para absolutamente todos os sons do aplicativo (voz, notificações, toques, mídias e vídeos). Sessões de chamada substituídas por outro dispositivo agora são encerradas e liberadas com segurança sem travar o cliente. Suíte de estresse automatizada com 50 ciclos de entrar/sair e 20 trocas de canal.
- **Avisos de chamada garantidos (Pilar 3 de Voz):** Sons de entrada, saída, microfone mutado e ensurdecimento agora são acionados diretamente pelo fluxo de eventos com ritmo mínimo de 120 ms, garantindo que saídas e entradas rápidas toquem os dois sons em ordem sem descartes na fila.
- **Pré-visualização e conclusão inteligente de downloads:** A pré-visualização de arquivos de texto e Markdown agora segue redirecionamentos dos servidores do Discord e renova links expirados automaticamente. O cartão de anexo exibe o status "Baixado" e oferece botões imediatos para "Abrir" ou "Mostrar na pasta", lembrando arquivos salvos localmente via banco SQLite leve.
- **Embeds sem caixas pretas vazias:** Imagens de embeds de links externos (como posts do X/Twitter) que falham ao carregar não mostram mais caixas pretas com "Couldn't load"; o espaço colapsa de forma limpa como se não houvesse imagem.
- **Seleção e exportação em lote completas:** Barra de ações dinâmica que exibe apenas as opções relevantes ao conteúdo selecionado (Download apenas se houver mídias, Copiar/Salvar apenas se houver texto, Excluir apenas para mensagens próprias com contagem explícita). Download de mídias de qualquer autor liberado, menu "Selecionar…" com filtros rápidos por tipo e quantidade (até 20+ mensagens) e exportação unificada em HTML estilo Telegram com SHA-256 no rodapé.
- **Painel do usuário centralizado:** Alinhamento vertical do avatar, nome de usuário e botões de controle de áudio perfeitamente alinhados na mesma linha de base do campo de mensagem, com rótulos de presença ("Online", "Ausente", "Não perturbe", "Invisível") e "Celular" traduzidos.
- **Atualizador e proteções em segundo plano:** Novo layout consistente nas configurações de atualização, verificação periódica inteligente e travamento de reinício de atualização caso uma chamada de voz esteja ativa ou haja envio de arquivos em andamento.
- **Links externos e proteção contra links falsos:** Diálogo de confirmação de links externos traduzido com opção de "Não perguntar de novo para este site", chave global em Privacidade e verificação de segurança estrita contra links disfarçados ou encurtadores.
- **Instância única e conformidade de licenças:** Bloqueio de instância dupla via lockfile com verificação de processo ativo para evitar corrupção do banco de dados SQLite e atribuição explícita aos gráficos Twemoji sob licença CC-BY 4.0 na seção Legal.

## Nivra 1.0.9

Vídeo toca dentro do app. O anexo `.mp4` do Discord parou de mostrar "vídeo indisponível", e o vídeo de embed (X, GIF, link direto) abre no player do próprio Nivra, sem navegador: o WebView ficou só para o login do Discord. YouTube e Vimeo continuam abrindo no navegador do sistema, com um botão só.

A barra de vídeo ficou melhor: arrastar a linha do tempo mostra a prévia e só salta uma vez, ao soltar, e a imagem e o som continuam juntos depois do salto. Quando um vídeo não abre, o app diz o motivo (formato ou codec não suportado, arquivo grande demais, link expirado), em português ou espanhol, e oferece "Tentar de novo" e "Baixar vídeo".

A janela mudou: os dois circles laranja ao lado do minimizar viraram um ícone de sinal com verde, amarelo ou vermelho conforme a conexão, e o mouse explica "Conectado · ping 180 ms". A versão saiu dali e ficou em Configurações > Sobre.

A interface fala português do Brasil e espanhol, inclusive a primeira tela (boas-vindas, login, token de sessão, avisos) e as mensagens de erro. O texto de cada idioma agora mora em um arquivo só, dentro de `crates/ui/locales`: acrescentar outro idioma é acrescentar um arquivo, sem mexer em nenhuma tela.

## Nivra 1.0.7

The local database keeps a 2 MiB page cache, which already holds the file. A write past the 256 MiB cap returns an error. Saving a channel at that cap drops the oldest cached channel to make room, then retries. Text, Markdown and code previews detect UTF-8 and UTF-16, read a legacy Windows text file, and show a long file in steps. Video stays on the operating system's decoder. There is no VLC and no bundled FFmpeg. A PDF attachment is still a file card.

## Nivra 1.0.6

A call stays up when someone joins or leaves, through a short resume, and while the PC is stalled. Global shortcuts are off until they are turned on under Settings, Keybinds. Chat can be exported as text or Markdown. The Windows build remains one executable, without a notification install script, and these builds are unsigned.

## Nivra 1.0.5

SereinExt is now Nivra.

The project retains its existing version history and Git history.
This release introduces the new Nivra identity and begins the
migration of application identifiers from SereinExt/Serein.

Previous project name:
SereinExt

Original upstream:
Serein by the Serein contributors / ViceVerse-cz.

Existing installations are migrated where applicable.
