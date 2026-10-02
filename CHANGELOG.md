# Changelog

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
