# Changelog

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
