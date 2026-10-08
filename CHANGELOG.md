# Changelog

## Nivra 1.0.14

Esta versão é uma auditoria completa do código: 48 achados levantados e tratados, dos quais os graves eram falhas que podiam **derrubar o aplicativo** ou **apagar a conversa na tela**. Nada de recurso novo — o objetivo é o que já existia parar de falhar.

### Travamentos

- **O relatório de erro não derruba mais o aplicativo.** A linha do log era cortada por bytes; um corte no meio de um caractere acentuado ou emoji fazia o próprio tratamento de pânico entrar em pânico — e aí o erro não era gravado. O corte agora respeita o limite do caractere.
- **A redação de segredos não vaza e não quebra.** O borrão de e-mails usava o tamanho da linha original para recortar o texto já encurtado, então o **segundo** endereço da mesma linha ficava visível no log; em linha com acento o recorte podia quebrar. Corrigido.
- **Sincronizar a lista de membros não derruba a conexão.** Uma faixa de membros inválida chegava até o cálculo do intervalo e quebrava o processo. Agora é validada na entrada e no redirecionamento.

### A conversa não some mais

- **Enquete com voto desconhecido.** Um único voto para uma resposta que não estava no cartão carregado era tratado como estado corrompido e limpava a conversa inteira. Agora esse voto é ignorado.
- **Exclusão em massa fora do limite.** Um lote com mais de 100 mensagens apagadas aplicava efeitos na tela e **depois** se declarava inválido, caindo na mesma limpeza. Agora o lote é descartado antes de qualquer efeito.

### Chamadas e conexão

- **Comando de voz perdido deixava a chamada presa.** Se o envio falhasse no meio, o pedido era descartado sem aviso — e como o estado já tinha sido marcado, tentar entrar de novo respondia "chamada já ativa". Agora a falha é reportada e o estado não fica cunhado.
- **Limite de taxa (429) com resposta inválida** deixava de ser tratado como limite e o pedido seguinte saía sem esperar. Agora usa uma espera padrão.
- **Falha de capacidade** encerrava a tarefa de conexão sem avisar ninguém, deixando a janela em "carregando" para sempre. Agora o aplicativo sabe e mostra o motivo.
- **Busca e perfil não morrem mais junto com a voz.** Uma oscilação de rede na chamada abortava silenciosamente uma busca ou um perfil em andamento, que ficavam carregando indefinidamente. O mesmo valia para paginar o histórico. Corrigido.
- **Pré-visualização de anexo** deixava o download antigo rodando até o fim depois de você abrir outro. Agora o anterior é cancelado, e o cliente HTTP é reaproveitado.

### Estado e interface

- **Cargos e permissões** deixavam de ser aprendidos quando o servidor ainda não tinha mandado a lista de cargos ou o membro, e a decisão de permissão ficava desatualizada.
- **Rascunhos:** o teto de `/msg` podia ser furado, e o limite era global em vez de por conta — uma conta cheia bloqueava as outras.
- **Trocar de chamada** era cancelado em silêncio; agora diz por quê. E uma fase de conexão atrasada não faz mais a chamada "regredir" na tela.
- **Canais de palco (stage)** entram como ouvinte, sem microfone nem câmera, como no Discord.
- **Canais de voz** aparecem com o rótulo correto, e a lista de participantes deixou de ser recalculada duas vezes por quadro.

### Robustez de dados e plataforma

- **Texto e nomes vindos do servidor ganharam teto**, junto com as listas de canais, cargos e membros — antes uma resposta hostil podia ocupar memória sem limite.
- **Contabilidade de bytes** de fórum, busca e reações estava subestimando ou contando em dobro; validadores de enquete, figurinha, emoji e avatar aceitavam lixo.
- **Banco local:** linhas de conta danificadas passam a ser isoladas em vez de "consertadas" silenciosamente; domínio inválido passa a ser recusado de verdade; a evicção do cache de fórum deixou de ser aleatória dentro do mesmo segundo.
- **Migração de dados** confere se os caminhos são diretórios, a varredura de DLL do sistema não diz mais "limpo" quando não conseguiu ler a pasta, e a exportação de log tem teto de tamanho.
- **Layout da conversa** deixou de refazer a resolução de menções de todas as linhas a cada mudança de estado (era O(n²) por causa de uma varredura da conversa inteira por menção não resolvida): cerca de **-36% no custo por linha** nesse caminho.

### Limites conhecidos

- Os pacotes não têm assinatura digital.
- Interoperabilidade ao vivo com o Discord e comportamento de microfone/alto-falante continuam **não verificados**: a verificação é sintética e offline.

### English

1.0.14 is a full code audit: 48 findings raised and handled, no new features. The severe ones were failures that could **take the application down** or **wipe the conversation on screen**. The crash-report writer no longer panics while trimming an over-long log line (it cut by bytes, so an accented or emoji character on the boundary panicked the panic handler itself, and the report was lost), and the secret redactor no longer uses the original line length to cut the already-shortened buffer — the second e-mail address on a line used to stay visible in the log. An invalid member-list subscription no longer reaches the span arithmetic that killed the process; it is validated on entry and on retarget.

The timeline is no longer cleared by malformed service input: a vote for a poll answer that is not in the loaded card, and a bulk delete above the local limit (which applied its screen effects and only then declared itself invalid), both used to be treated as corrupt state and dropped the whole resident window.

Calls and connection: a voice command lost to a send timeout used to be discarded silently after the call state had already been stamped, so rejoining answered "call already active" — the failure is now reported and the state is not wedged; a 429 with a malformed body is still a rate limit instead of skipping the cooldown; a capacity failure now reaches the application instead of leaving a dead connection task and a window stuck loading; and a voice reconnect no longer silently aborts an in-flight search, profile fetch or history page, which left those panes loading forever.

Interface and data: roles and member roles are learned even when a snapshot arrives without the role list or without the member, and a stage channel joins as a muted listener without camera, as in the official client. The `/msg` draft cap can no longer be bypassed and is enforced per account, switching calls reports why it was cancelled, a stale setup phase no longer regresses a live call, and the voice roster snapshot is accumulated in one pass instead of quadratic. Inbound text, nonces and channel names, plus guild channel, role, voice-state and member lists, are bounded before they are retained; forum, search and reaction byte accounting was undercounting or double counting; poll, sticker, emoji and avatar validators accepted invalid values. Local storage rows damaged outside the client are quarantined instead of silently normalized, an invalid allowed domain is rejected, forum-cache eviction is deterministic within one second, data-directory migration verifies directories, a DLL scan that cannot read its directory no longer reports the machine as clean, and the log export is capped.

Performance: the timeline layout fingerprint stopped resolving every row's mentions by scanning the whole conversation for each unresolved `<@id>` — a measured O(rows × timeline) pass per state change, about **−36% per row** on that path, recorded in `docs/performance.md`. Components measured as noise (the twice-per-row date conversion, the per-frame row-height sum and the gateway member-mirror clone) were left alone rather than traded for scroll-extent or atomicity correctness.

Known limits: the packages are unsigned, and live Discord interoperability plus physical microphone/speaker behaviour remain **unverified** — verification is synthetic and offline.

## Nivra 1.0.13

Esta versão fecha as rodadas 17 a 19. A chamada cai e volta sozinha quando o servidor manda, os sons da call tocam mesmo com a janela minimizada ou na bandeja, a pré-visualização de texto e a primeira imagem colada funcionam, o fórum abre em grade com miniaturas, tags e cache, a interface e a bandeja falam português e espanhol, e o executável ficou cerca de 12 MB menor em cada sistema porque a fonte de japonês, chinês e coreano agora vem do sistema operacional.

### Chamadas de voz

- **A call reconecta sozinha.** Quando a conexão de voz fecha, o Nivra segue a regra do Discord: sessão expirada ou servidor trocado reentra sozinho (até 3 tentativas, com espera crescente até 30 s), queda de rede retoma a sessão e um kick ou limite de taxa encerra de verdade. Durante a tentativa o painel mostra "Reconectando…", o botão Reentrar fica à mão, e ao voltar o microfone, o fone e os dispositivos ficam como estavam. Se a call terminar, a mensagem diz o código real e tem "Copiar detalhes" (#97).
- **Sons da call sempre.** Entrada, saída, mute e desmute (seus e das outras pessoas), ensurdecer e queda/reconexão tocam com a janela em foco, atrás de outra janela, minimizada ou só na bandeja. A fila comporta uma rajada de dez entradas sem perder nenhum som, há um som novo para mute/desmute de outras pessoas (ligado por padrão) e, com a janela sem foco, entrada e saída também mostram a notificação do sistema com o nome. Em Configurações > Notificações > Sons da call, "Testar sons" toca a sequência no dispositivo escolhido (#100).
- **Microfone ao ensurdecer.** Com o ensurdecer ligado, o ícone do microfone no cartão da conta e no painel da call mostra o corte, com a dica explicando o motivo; clicar no microfone desliga o ensurdecer e liga o microfone, como no Discord oficial, e o estado enviado ao servidor continua igual ao mostrado (#101).
- **Exportar diagnóstico.** Configurações > Ajuda gera um `.zip` na Área de Trabalho com o log das últimas 24 h, o registro de travamentos, a versão e os dispositivos de áudio (sem segredos), e "Abrir pasta de logs" leva aos arquivos. O log da chamada agora tem data e hora locais em cada linha, registra o ciclo inteiro da conexão e o motivo real do fechamento (#97).

### Mensagens, anexos e vídeo

- **Pré-visualizar voltou.** Arquivos `.md`, `.txt`, `.json`, código e logs abrem a pré-visualização, venham do CDN ou do proxy, com link renovado, acentos, BOM e CRLF; arquivo grande demais ou link expirado mostram o motivo certo, em português e espanhol (#95).
- **A primeira imagem colada envia de primeira.** Um Ctrl+V não vira mais dois envios nem falha na estreia; se o envio for recusado, o cartão vermelho mostra a causa em linguagem simples e ganha "Tentar de novo", além de "Voltar para o compositor" (#98).
- **Vídeos curtos tocam sozinhos** no cartão, mudos por padrão (com som se a preferência estiver ligada), e pausam quando a janela perde o foco (#93).
- **Status de download traduzido** e conferência de tamanho corrigida para imagens redimensionadas (#104).

### Fórum

- **Grade com miniaturas.** Os cartões de fórum mostram até quatro imagens pré-carregadas, autor, trecho e reações; a lista abre em grade com mosaico de 1, 2, 3, 4 ou mais imagens (com "+N"), selo de GIF, play de vídeo, spoiler coberto e visualizador ao clicar. O menu "Ordenar e ver" alterna os layouts, e a tela seguinte é pré-aquecida sem estourar o orçamento de memória (#105, #106).
- **Tags e filtros.** Chips com emoji, barra de filtros com "Todos", "Qualquer" e "Todas", e seletor de tags no compositor (#107).
- **Abre na hora e carrega mais leve.** A lista de publicações passou a vir em uma requisição por página (25 requisições a menos por fórum) e fica guardada em disco por conta, com o selo "Atualizado" quando chega a versão nova; fóruns grandes caem para um plano B com aviso discreto e uma linha no log (#103, #108, #109).

### Interface e idiomas

- Menus de canal, a página Chat/Layout e o menu da bandeja (abrir, minimizar, sair) agora aparecem em português e espanhol (#93, #94, #102).

### Distribuição e documentação

- **Executável cerca de 12 MB menor em cada sistema** (24 MB no Linux, cujo pacote também embute o `.deb`). A fonte de japonês, chinês e coreano deixou de ser embutida: quando aparece texto CJK, o Nivra lê uma vez a fonte do sistema (Windows, macOS ou Linux), valida a cobertura por script e a usa; sem fonte instalada, o texto mostra o caractere de substituição e um aviso único explica o que instalar. Nada é baixado. Windows x64: 100.271.104 → 88.198.656 bytes; Windows ARM64: 66.161.664 → 54.084.608; Linux x64: 110.946.740 → 86.924.944; macOS ARM64: 53.848.936 → 41.770.019 (#111).
- O README foi reorganizado e a tabela de desempenho voltou rotulada como referência histórica do projeto Serein (a medição do Nivra no Windows segue pendente do dono, com `scripts/measure.ps1`) (#99, #112).

### English

1.0.13 closes rounds 17–19. The call reconnects on its own: an expired session or a moved voice server rejoins (up to 3 attempts with a growing 5/15/30 s wait), a network drop resumes, and a kick or rate limit ends it, with a "Reconnecting…" panel, a Rejoin button, mute/deafen/devices restored, and an honest close code with Copy details (#97). Call cues — join, leave, your and other members' mute/unmute, deafen and call drop/reconnect — play with the window focused, unfocused, minimized or hidden in the tray, a burst of ten arrivals keeps ten sounds, a new member mute/unmute sound is on by default, unfocused join/leave raises a system notification with the member name, and Settings > Notifications > Call sounds has Test sounds (#100). Deafening now shows the crossed-out microphone on the account card and call panel, and clicking the microphone undeafens and unmutes like the official client (#101). Settings > Help exports a diagnostics `.zip` to the Desktop (last 24 h of log, freeze report, version and audio devices, no secrets) and opens the logs folder; the call log has local timestamps on every line and records the full connection cycle and close reason (#97).

Text preview works again for `.md`, `.txt`, `.json`, code and logs from the CDN or the proxy, with renewed links, BOM and CRLF handling and translated reasons (#95). The first Ctrl+V image sends on the first try, and a rejected upload states the real reason with Try again and Back to composer (#98). Short videos autoplay on the card, muted by default (sound follows the preference) and paused when the window loses focus (#93). Download status is translated and the resized-rendition size check is fixed (#104).

Forum: cards preload up to four images with author, excerpt and reactions, the list opens in a gallery with 1/2/3/4+ image mosaics, a GIF badge, video play, covered spoilers and an image viewer, with a sort/view menu and a prefetched next screen inside the memory budget (#105, #106); tag chips, a Todos/Any/All filter bar and a composer tag picker (#107); one request per page (25 fewer per forum) plus an account-scoped disk cache that opens instantly with an Updated badge, a fallback with a discreet notice and a log line for large forums (#103, #108, #109). Channel menus, the Chat/Layout page and the tray menu (show, minimize, quit) are translated to Portuguese and Spanish (#93, #94, #102).

Distribution: the executable is about 12 MB smaller on every platform (24 MB on Linux, whose tarball also bundles the `.deb`) because the Japanese/Chinese/Korean font is no longer embedded — CJK text reads an installed system font once, validates coverage per script and uses it, with a one-time translated notice when none is installed and no download. Windows x64 100,271,104 → 88,198,656 bytes; Windows ARM64 66,161,664 → 54,084,608; Linux x64 110,946,740 → 86,924,944; macOS ARM64 53,848,936 → 41,770,019 (#111). The README was reorganized and the performance table restored as a historical Serein reference, with Nivra's own Windows measurement still pending (#99, #112).

## Nivra 1.0.12

Esta versão fecha a rodada 16: corrige o que impedia de abrir a pré-visualização de texto e as Licenças, o cartão da conta que esticava, os sons da chamada com a janela minimizada ou na bandeja, e os vídeos acima de 1080p; adiciona proteção contra DLLs estranhas, log de erros sem dados sensíveis com página de Ajuda, animações padronizadas com a opção "reduzir movimento" e botões mais claros e amigáveis. O executável ficou cerca de 3% menor.

### Correções

- Pré-visualizar um arquivo de texto e a tela Configurações > Licenças voltaram a abrir: os dois diálogos só eram desenhados depois dos avisos iniciais de boas-vindas, então quem já os havia aceitado nunca os via. Arquivo grande demais (acima de 256 KB) e link expirado mostram o aviso certo (#85).
- O cartão da conta (avatar, nome, status, microfone, fone e engrenagem) não estica mais: a borda de cima era uma alça invisível que podia guardar a altura da janela, e as seções de chamada e de atualização deixam o cartão maior; agora ele tem sempre a altura do conteúdo e volta sozinho ao tamanho certo quando a chamada ou o banner some (#86).
- Vídeos de qualquer resolução, de 640×360 até cerca de 16 megapixels (ex.: 4500×3000), cabem no player: o decodificador entrega o quadro já reduzido para a caixa de prévia (1920×1080 na horizontal, 1080×1920 na vertical), sem aumentar vídeos pequenos. A antiga recusa "funciona até 1080p" saiu. Codecs sem decodificador no sistema (por exemplo HEVC ou AV1 sem a extensão do Windows) continuam sem tocar — nesse caso o cartão mostra o motivo real com "Baixar vídeo" e "Abrir o original" (#88).
- Os sons da chamada — entrada, saída, mute e desmute seus e das outras pessoas, ensurdecer e queda/reconexão — tocam com o Nivra aberto, sem foco, minimizado ou só na bandeja: os avisos eram gerados no desenho da tela, que não roda com a janela escondida. Agora saem direto do fluxo de eventos, em fila maior (uma rajada de dez entradas toca dez sons) e com um som novo, ligado por padrão, para mute/desmute de outras pessoas. Com a janela sem foco ou minimizada, entrada e saída também mostram a notificação do sistema com o nome (#87).
- Proteção contra DLLs estranhas na pasta do Nivra: o executável do Windows passa a resolver as dependências só de System32, varre os módulos carregados na abertura e avisa, sem bloquear, quando encontra uma DLL com nome de sistema ao lado do .exe (winmm.dll, version.dll, d3d11.dll...), com "Copiar detalhes". Esse era o caso do aviso "Self-protection failed. Error code: 4": ele vinha de um programa de terceiros na pasta, não do Nivra (#89).
- Log de erros sem dados sensíveis e com relatório de falha: os erros das rotinas já migradas para o novo log (downloads, pré-visualização de anexos, composição de janela e registro do aplicativo) passam a ser gravados em arquivos com rotação em `%LOCALAPPDATA%\nivra\logs` (no máximo 5 arquivos de 2 MiB); tokens, cookies, e-mails, IDs e texto de mensagem nunca são gravados. Se o Nivra fechar inesperadamente, a próxima abertura mostra o aviso "O Nivra fechou inesperadamente" com "Copiar relatório", e Configurações > Ajuda ganhou "Copiar log de erros" (com o aviso "Log copiado") e "Abrir pasta de logs". Alguns avisos internos (entrega de atualização, fallback do ícone da bandeja e diagnósticos de GPU/vídeo) ainda só aparecem no console (#91).

### Interface e desempenho

- Animações com um padrão único — durações curta, média e longa e uma só curva — em botões, menus, diálogos, toasts, banners, painéis, troca de canal, entrada de mensagem, medidor de upload, mídias e anel de fala. A animação interrompida no meio continua de onde está, e quando nada anima a interface para de repintar. Nova opção "Reduzir movimento" em Configurações > Aparência, desligada por padrão (#83).
- Botões e ações mais claros: botões de ícone desabilitados explicam o motivo ao passar o mouse, e ações destrutivas passam por uma confirmação única, coberta por teste (#84).
- O executável do Windows ficou cerca de 3% menor (x64: 102.763.008 → 99.608.064 bytes; ARM64: 70.402.560 → 65.595.392 bytes) com uma única unidade de compilação e otimização leve nos crates frios; voz, áudio e vídeo mantêm a otimização máxima. Nada de compactador de executável (#82).

### English

1.0.12 closes round 16. The text preview and the Licenses screen open again (both dialogs were drawn only after the first-run notice, so any user who had accepted it never saw them; an oversized file and an expired link show the right toast). The account card keeps its content height and returns to it when a call or the update banner goes away. Call cues — join, leave, your and other members' mute/unmute, deafen and call drop/reconnect — play with the window focused, unfocused, minimized or hidden in the tray, pumped from the event path instead of the paint pass, with a 64-cue queue (ten arrivals keep ten sounds), a new member mute/unmute sound on by default, and a system notification with the member name on join/leave while the window is unfocused or minimized. Any video resolution from 640×360 up to about 16 megapixels (e.g. 4500×3000) fits the preview box (1920×1080 landscape, 1080×1920 portrait, never upscaled), so the old "up to 1080p" refusal is gone; codecs the system does not decode (HEVC/AV1/VP9 without the Windows extension) still do not play, and the card states the real reason with Download video / Open original. The Windows binary links with /DEPENDENTLOADFLAG:0x800, sets SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32), scans loaded modules at startup and shows a non-blocking warning with Copy details when system-named DLLs sit next to the executable (that was the "Self-protection failed. Error code: 4" report, which came from a third-party program). Errors captured by the new log (downloads, attachment previews, window compositing and application registration) go to a rotating redacted log (5 × 2 MiB) with tokens, cookies, e-mails, ids and message text never written; a crash report banner appears on the next launch, and Settings > Help has Copy error log and Open logs folder. Some internal diagnostics (update handoff, tray-icon fallback and GPU/video diagnostics) still only reach the console.

Interface: one motion token set (short/medium/long, one curve) across buttons, menus, dialogs, toasts, banners, panels, channel switches, message entry, the upload meter and the speaking ring, with interrupted animation continuing from its position and no repaint once settled, plus a Reduce motion switch (default off) in Settings > Appearance; disabled icon buttons name their action on hover and destructive actions go through one confirmation. Windows executables are about 3% smaller (x64 102,763,008 → 99,608,064 bytes; ARM64 70,402,560 → 65,595,392 bytes) with one codegen unit and cold-crate opt-level "s". Test stability: the pillar-1 benchmark and the starved login test were de-flaked (#81) and the paused video seek no longer races the clip end (#90).

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

## SereinExt 1.0.4

Version 1.0.4 keeps the 1.0.3 client and adds the local work finished after that release:

- saved logins survive a system keyring hiccup, with clearer guidance when sign-in needs attention;
- the app tells you when the PC cannot keep up with Maximum noise suppression and switches to Standard;
- the first-run terms dialog is simpler;
- link previews handle X, Vimeo and YouTube addresses more strictly.

## SereinExt 1.0.3

Version 1.0.3 keeps the 1.0.2 client and adds the local work finished after that release:

- the app language follows Windows on first launch when Portuguese or Spanish is available, with a language control on the sign-in screen;
- login, chat chrome and call controls use the chosen language;
- screen sharing starts from the whole display, with audio under it and apps folded away;
- someone connecting to or leaving the current call plays a sound, including while Do Not Disturb is on, and retries if playback is busy;
- a failed image or file send shows the error on the preview, with a way to put the file back in the composer.

## SereinExt 1.0.2

Version 1.0.2 integrates selected upstream improvements that fit SereinExt without replacing the fork-specific fixes:

- expanded Extension SDK capabilities for bounded app queries, messaging settings, guild folders, native actions and action-result feedback (upstream #411);
- Discord-like inline image sizing, higher-quality media renditions and smoother MP4/MOV/M4V-backed gifv motion while preserving SereinExt isolated web previews (upstream #409);
- permission-gated server sticker management with static PNG/JPEG/WebP preparation, upload, edit and delete support (upstream #412);
- the upstream server-creation flow with create/join picker, server name/icon preparation and bounded request state (upstream #414);
- the existing SereinExt voice, login, multi-device presence, PT-BR/ES, moderation, Hyprland and web-media changes remain preserved.

Server creation is included as an **experimental compatibility feature**: upstream notes that the normal-user `POST /guilds` behavior is unofficial and was not live-verified, so SereinExt keeps the flow bounded and should not treat a failed request as proof that no server was created.

## SereinExt 1.0.1

Version 1.0.1 is the first maintained SereinExt line. The current `main` includes:

- corrected remote voice join/leave cues, AEC delay handling and selected-device recovery;
- moderator/member-removal fixes and lower drag/presentation latency;
- direct join-video flow with bounded failure state;
- compact Discord multi-device presence for desktop/mobile/web;
- more reliable Discord login WebView focus and Linux permission flow;
- persistent English, Português (Brasil) and Español UI locale infrastructure;
- isolated in-app YouTube, X/Twitter and Vimeo previews with device permissions denied;
- Hyprland tray/notification restoration ported from upstream;
- SereinExt-specific credential/update identity and cleaned voice UI encoding.

The source is still being validated before a binary GitHub Release is published. A tag does not imply that upstream Serein packages are SereinExt builds.
