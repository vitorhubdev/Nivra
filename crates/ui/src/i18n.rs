//! Small built-in locale catalog. Unknown keys deliberately fall back to English so
//! incremental translation cannot make a control disappear or become unusable.
use model::Language;

pub fn text(language: Language, english: &'static str) -> &'static str {
	let translated = match language {
		Language::English => return english,
		Language::PortugueseBrazil => portuguese_brazil(english),
		Language::Spanish => spanish(english),
	};
	match translated {
		Some(text) => text,
		None => {
			#[cfg(test)]
			UNTRANSLATED_KEYS.with(|keys| keys.borrow_mut().push(english.to_owned()));
			english
		}
	}
}

/// True when any bundled language knows this key, so a missing translation of a known
/// string is reported instead of silently passing through.
#[cfg(test)]
fn is_catalog_key(key: &str) -> bool {
	portuguese_brazil(key).is_some() || spanish(key).is_some()
}

/// Fixed egui temp key holding the current interface language. `MessagingUi`
/// stores it every frame so shared render helpers (menus, save bars) can read
/// it without signature changes in files owned by other agents.
const INTERFACE_LANGUAGE_KEY: &str = "nivra-interface-language";

pub fn store_interface_language(ctx: &egui::Context, language: Language) {
	ctx.data_mut(|data| data.insert_temp(egui::Id::unique(INTERFACE_LANGUAGE_KEY), language));
}

/// Language for shared chrome; English when no frame stored one (tests, previews).
pub fn interface_language(ctx: &egui::Context) -> Language {
	ctx.data(|data| data.get_temp::<Language>(egui::Id::unique(INTERFACE_LANGUAGE_KEY)))
		.unwrap_or(Language::English)
}

/// Translate using the language stored on the current egui context.
#[macro_export]
macro_rules! tr_ui {
	($ui:expr, $english:expr) => {
		$crate::i18n::text($crate::i18n::interface_language($ui.ctx()), $english)
	};
}

/// Translate a runtime string (an error or a status that only exists at run time).
/// Known keys come from the catalog; anything else is returned unchanged.
pub fn text_str(language: Language, english: &str) -> std::borrow::Cow<'_, str> {
	let translated = match language {
		Language::English => return std::borrow::Cow::Borrowed(english),
		Language::PortugueseBrazil => portuguese_brazil(english),
		Language::Spanish => spanish(english),
	};
	match translated {
		Some(text) => std::borrow::Cow::Borrowed(text),
		None => {
			#[cfg(test)]
			if is_catalog_key(english) {
				// Only known keys are recorded, so runtime statuses stay quiet.
				UNTRANSLATED_KEYS.with(|keys| keys.borrow_mut().push(english.to_owned()));
			}
			std::borrow::Cow::Borrowed(english)
		}
	}
}

/// Translate a runtime string using the language stored on the current egui context.
#[macro_export]
macro_rules! tr_str {
	($ui:expr, $english:expr) => {
		$crate::i18n::text_str($crate::i18n::interface_language($ui.ctx()), $english)
	};
}

/// Translate using the language stored on an egui context (dialogs without `Ui`).
#[macro_export]
macro_rules! tr_ctx {
	($ctx:expr, $english:expr) => {
		$crate::i18n::text($crate::i18n::interface_language($ctx), $english)
	};
}

/// Keys that fell back to English on this thread since the last drain. Tests
/// render surfaces in another language and require this list to stay empty.
#[cfg(test)]
pub fn drain_untranslated_keys() -> Vec<String> {
	UNTRANSLATED_KEYS.with(|keys| std::mem::take(&mut *keys.borrow_mut()))
}

#[cfg(test)]
thread_local! {
	static UNTRANSLATED_KEYS: std::cell::RefCell<Vec<String>> =
		const { std::cell::RefCell::new(Vec::new()) };
}

fn portuguese_brazil(key: &str) -> Option<&'static str> {
	Some(match key {
		"Voice Connected" => "Voz conectada",
		"Voice preview" => "Prévia de voz",
		"Connecting…" => "Conectando…",
		"Call failed" => "Chamada falhou",
		"A selected message could not be deleted and is back in the conversation" => {
			"Uma mensagem selecionada não pôde ser apagada e voltou para a conversa"
		}
		"User settings" => "Configurações do usuário",
		"App settings" => "Configurações do aplicativo",
		"Customization" => "Personalização",
		"My Account" => "Minha conta",
		"Profile" => "Perfil",
		"Mention" => "Mencionar",
		"Add Note" => "Adicionar nota",
		"Edit Friend Nickname" => "Editar apelido de amigo",
		"Add Friend Nickname" => "Adicionar apelido de amigo",
		"Private nicknames are available for confirmed friends." => {
			"Apelidos particulares estão disponíveis para amigos confirmados."
		}
		"Pin DM" => "Fixar DM",
		"Unpin DM" => "Desafixar DM",
		"Pinned direct messages are saved on this device." => {
			"Conversas fixadas são salvas neste dispositivo."
		}
		"Mute Conversation" => "Silenciar conversa",
		"Unmute Conversation" => "Reativar conversa",
		"Mute this direct message's notifications until you unmute it." => {
			"Silencia as notificações desta conversa até você reativá-la."
		}
		"Close DM" => "Fechar DM",
		"Remove this conversation from your DM list. Messages are kept." => {
			"Remove esta conversa da sua lista de conversas. As mensagens são mantidas."
		}
		"No open direct message with this user." => "Nenhuma conversa aberta com este usuário.",
		"Block" => "Bloquear",
		"Unblock" => "Desbloquear",
		"Change Nickname" => "Mudar apelido",
		"Nickname" => "Apelido",
		"Roles" => "Cargos",
		"Kick" => "Expulsar",
		"Save" => "Salvar",
		"This removes the member from this server. They can rejoin with a new invite." => {
			"Isso remove o membro deste servidor. Ele pode voltar com um novo convite."
		}
		"General" => "Geral",
		"Appearance" => "Aparência",
		"Chat" => "Conversas",
		"Messaging Permissions" => "Permissões de mensagens",
		"Notifications" => "Notificações",
		"Game Activity" => "Atividade de jogos",
		"Voice & Video" => "Voz e vídeo",
		"Keybinds" => "Atalhos de teclado",
		"Enable global shortcuts" => "Ativar atalhos globais",
		"Mute, deafen and push-to-talk stay off until you turn this on. They then work even when Nivra is in the background." => {
			"Mudo, ensurdecer e push-to-talk ficam desligados até você ativar isto. Depois funcionam mesmo com o Nivra em segundo plano."
		}
		"Data & Privacy" => "Dados e privacidade",
		"Updates" => "Atualizações",
		"Extensions" => "Extensões",
		"Themes" => "Temas",
		"The Discord account signed in on this device." => {
			"A conta do Discord conectada neste dispositivo."
		}
		"Choose how you appear across Discord." => "Escolha como você aparece no Discord.",
		"Startup, window and graphics behavior on this device." => {
			"Inicialização, janela e gráficos neste dispositivo."
		}
		"Theme, colours, window effects and layout." => "Tema, cores, efeitos da janela e layout.",
		"How messages, media, links and scrolling behave." => {
			"Como mensagens, mídia, links e rolagem se comportam."
		}
		"Control who can contact you and how messages are filtered." => {
			"Controle quem pode falar com você e como as mensagens são filtradas."
		}
		"Choose which notifications you receive and how they appear." => {
			"Escolha quais notificações você recebe e como elas aparecem."
		}
		"Show others what you are playing." => "Mostre aos outros o que você está jogando.",
		"Microphone, speakers, camera and voice processing." => {
			"Microfone, alto-falantes, câmera e processamento de voz."
		}
		"Keyboard shortcuts for Nivra." => "Atalhos de teclado do Nivra.",
		"What Nivra keeps on this device." => "O que o Nivra mantém neste dispositivo.",
		"Keep Nivra up to date on this device." => "Mantenha o Nivra atualizado neste dispositivo.",
		"Manage community plugins." => "Gerencie plugins da comunidade.",
		"Choose a community theme." => "Escolha um tema da comunidade.",
		"Language" => "Idioma",
		"App language" => "Idioma do aplicativo",
		"Changes apply immediately and are saved on this device." => {
			"As alterações são aplicadas imediatamente e salvas neste dispositivo."
		}
		"Startup" => "Inicialização",
		"Open Nivra when your computer starts" => "Abrir o Nivra ao iniciar o computador",
		"Nivra signs in and connects in the background." => {
			"O Nivra entra na conta e conecta em segundo plano."
		}
		"Start minimized" => "Iniciar minimizado",
		"Start in the background, out of your way." => {
			"Iniciar em segundo plano, sem ocupar a tela."
		}
		"Automatic startup is available on Windows and macOS." => {
			"A inicialização automática está disponível no Windows e macOS."
		}
		"Window" => "Janela",
		"Hide Nivra title bar" => "Ocultar a barra de título do Nivra",
		"Use the system title bar and window buttons instead." => {
			"Use a barra de título e os botões de janela do sistema."
		}
		"Keep Nivra in the menu bar" => "Manter o Nivra na barra de menus",
		"Keep Nivra in the system tray" => "Manter o Nivra na bandeja do sistema",
		"Nivra keeps running in the system tray" => "O Nivra continua na bandeja",
		"The tray is unavailable on this platform." => {
			"A bandeja do sistema não está disponível nesta plataforma."
		}
		"Graphics" => "Gráficos",
		"Render with" => "Renderizar com",
		"Search" => "Buscar",
		"Close settings (Esc)" => "Fechar configurações (Esc)",
		"Unofficial · not endorsed by Discord" => "Não oficial · não endossado pelo Discord",
		"Exit preview" => "Sair da prévia",
		"Log out" => "Sair da conta",
		"Your account" => "Sua conta",
		"Offline preview · synthetic account" => "Prévia offline · conta sintética",
		"Signed in with your Discord account" => "Conectado com sua conta do Discord",
		"Display name" => "Nome de exibição",
		"Email, password and security" => "E-mail, senha e segurança",
		"Managed in Discord" => "Gerenciado no Discord",
		"Edit profile" => "Editar perfil",
		"Session" => "Sessão",
		"Closes the offline fixture. Nothing is stored for the preview." => {
			"Fecha a prévia offline. Nada é armazenado para a prévia."
		}
		"Removes the saved login and clears this account's local cache and drafts." => {
			"Remove o login salvo e limpa o cache local e os rascunhos desta conta."
		}
		"Theme" => "Tema",
		"Accent" => "Cor de destaque",
		"Primary color" => "Cor primária",
		"The active theme brings its own accent; it takes over while the theme is in use." => {
			"O tema ativo traz sua própria cor de destaque e ela é usada enquanto o tema estiver ativo."
		}
		"Used for buttons, selection and message highlights." => {
			"Usada em botões, seleção e destaques de mensagens."
		}
		"Reset" => "Redefinir",
		"Choose primary color" => "Escolher cor primária",
		"Window effects" => "Efeitos da janela",
		"Transparency & blur" => "Transparência e desfoque",
		"Restart Nivra after changing this. Themes can customize effects while enabled." => {
			"Reinicie o Nivra após alterar isto. Temas podem personalizar os efeitos enquanto estiverem ativos."
		}
		"Transparency" => "Transparência",
		"Blur" => "Desfoque",
		"Zero disables blur; the native compositor controls its exact strength." => {
			"Zero desativa o desfoque; o compositor nativo controla a intensidade exata."
		}
		"Apply to all surfaces" => "Aplicar a todas as superfícies",
		"Include sidebars, server rail, headers, and composer." => {
			"Inclui barras laterais, trilho de servidores, cabeçalhos e compositor."
		}
		"Channel list" => "Lista de canais",
		"Show hidden channels" => "Mostrar canais ocultos",
		"Show channels you cannot currently access." => {
			"Mostra canais aos quais você não tem acesso no momento."
		}
		"Colour preset" => "Predefinição de cores",
		"Share game activity" => "Compartilhar atividade de jogo",
		"Detect running games and ask Discord to share them as activity." => {
			"Detecta jogos em execução e pede ao Discord para compartilhá-los como atividade."
		}
		"Enable on Discord" => "Ativar no Discord",
		"Check again" => "Verificar novamente",
		"Looking for a running game" => "Procurando um jogo em execução",
		"Activity sharing is off" => "O compartilhamento de atividade está desativado",
		"Synthetic activity, never shared or saved." => {
			"Atividade sintética, nunca compartilhada nem salva."
		}
		"Local storage" => "Armazenamento local",
		"Clear cache" => "Limpar cache",
		"Removes cached messages and media. Drafts and your login stay." => {
			"Remove mensagens e mídias em cache. Rascunhos e seu login permanecem."
		}
		"Messages and drafts are cached on this device inside bounded, account-isolated files. Cache data is not encrypted by Nivra; saved login tokens use the OS credential store." => {
			"Mensagens e rascunhos ficam em cache neste dispositivo em arquivos limitados e isolados por conta. Os dados de cache não são criptografados pelo Nivra; tokens de login salvos usam o armazenamento de credenciais do sistema."
		}
		"Your privacy" => "Sua privacidade",
		"Watch" => "Assistir",
		"Open on YouTube" => "Abrir no YouTube",
		"Open on Vimeo" => "Abrir no Vimeo",
		"Play" => "Reproduzir",
		"Pause" => "Pausar",
		"Resume" => "Continuar",
		"Replay" => "Assistir de novo",
		"Retry" => "Tentar de novo",
		"Cancel" => "Cancelar",
		"Fullscreen" => "Tela cheia",
		"Exit fullscreen (Esc)" => "Sair da tela cheia (Esc)",
		"Seek video" => "Mover no vídeo",
		"Download video" => "Baixar vídeo",
		"Save this video to your computer" => "Salvar este vídeo no seu computador",
		"Video attachment unavailable" => "O anexo de vídeo não está disponível",
		"This file is not a video" => "Este arquivo não é um vídeo",
		"Unsupported embed video provider or URL" => "Este vídeo não pode ser reproduzido aqui",
		"Video preview limit: 100 MiB" => "Limite de prévia de vídeo: 100 MiB",
		"Video server does not support buffering; download to play externally" => {
			"O servidor de vídeo não permite tocar aqui; baixe o arquivo para assistir"
		}
		"Video download failed or changed; reload the conversation" => {
			"O download do vídeo falhou ou mudou; recarregue a conversa"
		}
		"Video link expired; reload the conversation" => {
			"O link do vídeo expirou; recarregue a conversa"
		}
		"Video worker stopped; restart Nivra" => "O player de vídeo parou; reinicie o Nivra",
		"Could not start video worker" => "Não foi possível iniciar o player de vídeo",
		"Video audio output stopped" => "A saída de áudio do vídeo parou",
		"Unsupported video audio timing" => "O áudio deste vídeo não pode ser sincronizado",
		"Video buffering stalled; retry or download to play externally" => {
			"O vídeo travou ao carregar; tente de novo ou baixe o arquivo"
		}
		"This video format or codec is not supported on this system." => {
			"Este formato ou codec de vídeo não funciona neste computador."
		}
		"The video could not be decoded safely." => {
			"Este vídeo não pôde ser reproduzido com segurança."
		}
		"Inline playback supports videos up to 1080p." => "A reprodução aqui funciona até 1080p.",
		"Videos longer than two hours are not supported." => {
			"Vídeos com mais de duas horas não são suportados."
		}
		"This video cannot seek to that position." => {
			"Este vídeo não pode mudar para essa posição."
		}
		"This video format or codec is not supported by Windows." => {
			"Este formato ou codec de vídeo não é suportado pelo Windows."
		}
		"This video format or codec is not supported by macOS." => {
			"Este formato ou codec de vídeo não é suportado pelo macOS."
		}
		"This video format or codec is not supported by GStreamer." => {
			"Este formato ou codec de vídeo não é suportado pelo GStreamer."
		}
		"Nivra does not collect telemetry or upload diagnostics. Discord retains service-side data according to its own policies." => {
			"O Nivra não coleta telemetria nem envia diagnósticos. O Discord mantém dados do serviço de acordo com as próprias políticas."
		}
		"Offline preview · changes stay in this session and are never sent." => {
			"Prévia offline · as alterações ficam nesta sessão e nunca são enviadas."
		}
		"Closing the window keeps Nivra in the menu bar. Quit from its menu to exit." => {
			"Fechar a janela mantém o Nivra na barra de menus. Use o menu para encerrar."
		}
		"Closing keeps Nivra running. Use the tray to show, minimize or quit." => {
			"Fechar mantém o Nivra em execução. Use a bandeja para mostrar, minimizar ou encerrar."
		}
		"Closing the window keeps Nivra in the notification area. Quit from its menu to exit." => {
			"Fechar a janela mantém o Nivra na área de notificação. Use o menu para encerrar."
		}
		"Takes effect the next time Nivra starts." => {
			"Entra em vigor na próxima vez que o Nivra iniciar."
		}
		"Online" => "Online",
		"Idle" => "Ausente",
		"Do Not Disturb" => "Não perturbe",
		"Invisible" => "Invisível",
		"Don't clear" => "Não limpar",
		"30 minutes" => "30 minutos",
		"1 hour" => "1 hora",
		"4 hours" => "4 horas",
		"Today" => "Hoje",
		"Switch to" => "Alternar para",
		"Forget" => "Esquecer",
		"Nivra clears it" => "O Nivra limpa o status",
		"You" => "Você",
		"Custom status" => "Status personalizado",
		"Shown next to your name across Discord." => "Exibido ao lado do seu nome no Discord.",
		"Switch accounts" => "Alternar contas",
		"Add an account" => "Adicionar uma conta",
		"Forget this account on this device" => "Esquecer esta conta neste dispositivo",
		"Loading profile…" => "Carregando perfil…",
		"Reload profile" => "Recarregar perfil",
		"You will not receive desktop notifications" => {
			"Você não receberá notificações na área de trabalho"
		}
		"You will appear offline" => "Você aparecerá offline",
		"Edit custom status" => "Editar status personalizado",
		"Set a custom status" => "Definir status personalizado",
		"No custom status" => "Sem status personalizado",
		"Status text" => "Texto do status",
		"What's on your mind?" => "No que você está pensando?",
		"Clear after" => "Limpar depois",
		"Use up to 128 characters without control characters." => {
			"Use até 128 caracteres sem caracteres de controle."
		}
		"Clear" => "Limpar",
		"Apply" => "Aplicar",
		"No settings found" => "Nenhuma configuração encontrada",
		"Try theme, notifications, voice, or cache." => "Tente tema, notificações, voz ou cache.",
		"Use a different account" => "Usar outra conta",
		"Waiting for Discord…" => "Aguardando o Discord…",
		"Use another account" => "Usar outra conta",
		"Continue with Discord" => "Continuar com o Discord",
		"Welcome back" => "Bem-vindo de volta",
		"Interface language" => "Idioma da interface",
		"Loading discord.com…" => "Abrindo discord.com…",
		"Welcome to Nivra" => "Bem-vindo ao Nivra",
		"Continue with a saved account, or sign in with another one." => {
			"Continue com uma conta salva ou entre com outra."
		}
		"Sign in with Discord." => "Entre com o Discord.",
		"Early preview" => "Prévia inicial",
		"Checking your saved login" => "Conferindo o login salvo",
		"Connecting to Discord" => "Conectando ao Discord",
		"Sign in to Discord" => "Entrar no Discord",
		"discord.com · temporary login window · passwords and 2FA never leave the page" => {
			"discord.com · janela temporária de login · a senha e o 2FA não saem da página"
		}
		"Sign in with a session token" => "Entrar com um token de sessão",
		"For owners who already hold a valid Discord session token, for example from another signed-in Nivra install. Passwords and 2FA are never used here; this bypasses Discord's hosted login page entirely." => {
			"Para quem já tem um token de sessão do Discord, por exemplo de outra instalação do Nivra. Senha e 2FA não são usados aqui; a página de login do Discord não abre."
		}
		"Session token" => "Token de sessão",
		"Connect with this token" => "Conectar com este token",
		"About Nivra" => "Sobre o Nivra",
		"Forget saved login" => "Esquecer o login salvo",
		"Messaging, reactions, search and read markers have offline tests. Real Discord interoperability is still unverified; attachment uploads and advanced search remain incomplete." => {
			"Mensagens, reações, busca e marcadores de leitura têm testes offline. A interoperabilidade real com o Discord ainda não foi verificada; envio de anexos e busca avançada seguem incompletos."
		}
		"Messages and drafts are cached locally. Login tokens use the operating system credential store." => {
			"Mensagens e rascunhos ficam em cache local. Tokens de login usam o cofre de credenciais do sistema."
		}
		"Unofficial clients may put your Discord account at risk." => {
			"Clientes não oficiais podem colocar sua conta do Discord em risco."
		}
		"Explore the offline preview" => "Ver a prévia offline",
		"Sample conversations. No Discord connection." => {
			"Conversas de exemplo. Sem conexão com o Discord."
		}
		"or" => "ou",
		"Sign in again" => "Entrar de novo",
		"Sign in before calling" => "Entre antes de ligar",
		"Sign in before changing your profile picture" => "Entre antes de trocar a foto",
		"Sign in through Discord; saved-login lookup stopped" => {
			"Entrando pelo Discord; a busca do login salvo parou"
		}
		"Waiting for Discord login" => "Esperando o login do Discord",
		"Platform login webview unavailable; see platform-support.md" => {
			"A janela de login não abriu; veja platform-support.md"
		}
		"Platform login webview unavailable" => "A janela de login não abriu",
		"Saved accounts" => "Contas salvas",
		"This is my account" => "Esta é a minha conta",
		"Check this to continue." => "Marque isto para continuar.",
		"Independent and open source. Not affiliated with Discord." => {
			"Independente e de código aberto. Não afiliado ao Discord."
		}
		"Message" => "Mensagem",
		"Unread messages" => "Mensagens não lidas",
		"Mark as read" => "Marcar como lida",
		"Jump to unread" => "Ir para não lidas",
		"Copy" => "Copiar",
		"Copy message" => "Copiar mensagem",
		"Copy download link" => "Copiar link de download",
		"Reply" => "Responder",
		"Forward" => "Encaminhar",
		"Forward message" => "Encaminhar mensagem",
		"Create Thread…" => "Criar tópico…",
		"View reactions" => "Ver reações",
		"Mark read through here" => "Marcar como lida até aqui",
		"Mark Unread" => "Marcar como não lida",
		"Unpin message" => "Desafixar mensagem",
		"Pin message" => "Fixar mensagem",
		"Edit message" => "Editar mensagem",
		"Enter to save · Shift+Enter for a new line · Esc to cancel" => {
			"Enter salva · Shift+Enter quebra linha · Esc cancela"
		}
		"Remove from delete selection" => "Tirar da seleção",
		"Select for batch delete" => "Selecionar para apagar",
		"You can select up to 5 messages at a time." => {
			"Dá para selecionar até 5 mensagens por vez."
		}
		"Select" => "Selecionar",
		"selected" => "selecionadas",
		"Shift+click selects a range · drag paints · Esc exits" => {
			"Shift+clique seleciona um intervalo · arrastar pinta · Esc sai"
		}
		"Esc stops the rest" => "Esc interrompe o resto",
		"Stop" => "Parar",
		"Delete" => "Apagar",
		"Download" => "Baixar",
		"Save .txt" => "Salvar .txt",
		"Save .md" => "Salvar .md",
		"Export chat" => "Exportar conversa",
		"Exporting chat" => "Exportando conversa",
		"Export cancelled" => "Exportação cancelada",
		"Select all visible" => "Selecionar visíveis",
		"Select messages to enable actions" => "Selecione mensagens para ativar as ações",
		"None of the selected messages can be deleted" => {
			"Nenhuma das mensagens selecionadas pode ser apagada"
		}
		"Maximum 5 messages per delete" => "Máximo de 5 mensagens por vez para apagar",
		"No attachments in the selection" => "Sem anexos na seleção",
		"Maximum 15 attachments per download" => "Máximo de 15 anexos por download",
		"You can delete up to 5 at a time" => "Dá para apagar até 5 de uma vez",
		"Only your messages can be deleted here" => "Só suas mensagens podem ser apagadas aqui",
		"You can download up to 15 attachments at a time" => {
			"Dá para baixar até 15 anexos de uma vez"
		}
		"Download attachments" => "Baixar anexos",
		"Copy text" => "Copiar texto",
		"Downloads" => "Downloads",
		"Choose a folder…" => "Escolha uma pasta…",
		"Queued" => "Na fila",
		"Downloading" => "Baixando",
		"Done" => "Pronto",
		"Failed" => "Falhou",
		"Cancelled" => "Cancelado",
		"Open folder" => "Abrir pasta",
		"Selection copied" => "Seleção copiada",
		"Selection saved" => "Seleção salva",
		"Download complete" => "Download concluído",
		"Download cancelled" => "Download cancelado",
		"Delete message…" => "Apagar mensagem…",
		"Delete message immediately" => "Apagar mensagem agora",
		"Message history is unavailable with current permission information." => {
			"O histórico não está disponível com as permissões atuais."
		}
		"Mute" => "Silenciar",
		"Unmute" => "Ativar microfone",
		"Deafen" => "Silenciar áudio",
		"Undeafen" => "Ativar áudio",
		"Disconnect" => "Desconectar",
		"Dismiss call" => "Fechar chamada",
		"Reconnect to call" => "Reconectar à chamada",
		"Recent call" => "Chamada recente",
		"You were in this call recently" => "Você estava nesta chamada recentemente",
		"Dismiss" => "Dispensar",
		"Reconnecting" => "Reconectando",
		"attempt" => "tentativa",
		"Reconnect now" => "Reconectar agora",
		"Connected" => "Conectado",
		"Reconnecting…" => "Reconectando…",
		"No connection" => "Sem conexão",
		"Ping" => "Ping",
		"Connected for" => "Conectado há",
		"Connection recovery" => "Recuperação de conexão",
		"Rejoin calls after brief disconnects" => "Reentrar na chamada após quedas breves",
		"Automatically returns to the same call when Discord reconnects within 15 seconds." => {
			"Volta automaticamente para a mesma chamada quando o Discord reconecta em até 15 segundos."
		}
		"This can put you back on voice without an extra tap after short outages." => {
			"Isso pode colocá-lo de volta na voz sem um toque extra após quedas curtas."
		}
		"Only enable this if you are comfortable rejoining voice automatically on this device." => {
			"Só ative se você aceitar reentrar na voz automaticamente neste dispositivo."
		}
		"Continue" => "Continuar",
		"Enable auto-rejoin" => "Ativar reentrada automática",
		"Share your screen" => "Compartilhar tela",
		"Stop sharing" => "Parar de compartilhar",
		"Turn on camera" => "Ligar câmera",
		"Turn off camera" => "Desligar câmera",
		"Turn on microphone" => "Ligar microfone",
		"Turn off microphone" => "Desligar microfone",
		"Turn on incoming audio" => "Ouvir a chamada",
		"Turn off incoming audio" => "Deixar de ouvir a chamada",
		"Speaking is unavailable in this channel." => "Não é possível falar neste canal.",
		"Voice settings" => "Configurações de voz",
		"Microphone and speaker settings" => "Microfone e alto-falantes",
		"Noise suppression" => "Supressão de ruído",
		"Removes background noise from your microphone before anyone else hears it." => {
			"Tira o barulho de fundo do seu microfone antes que os outros escutem."
		}
		"Bot" => "Bot",
		"Screens" => "Telas",
		"Apps" => "Apps",
		"Options" => "Opções",
		"None open" => "Nenhum aberto",
		"open" => "abertos",
		"Resolution" => "Resolução",
		"Frame rate" => "Quadros/s",
		"Choose a screen or window" => "Escolha uma tela ou janela",
		"Looking for screens and windows…" => "Procurando telas e janelas…",
		"Offline preview · no screen is captured" => "Prévia offline · nenhuma tela é capturada",
		"In a call" => "Em chamada",
		"In a call · microphone muted" => "Em chamada · microfone mudo",
		"In a call · deafened" => "Em chamada · áudio desligado",
		"unread mentions" => "menções não lidas",
		"User volume" => "Volume da pessoa",
		"Reset volume" => "Restaurar volume",
		"Silent" => "Sem som",
		"Normal" => "Normal",
		"Louder than normal" => "Mais alto que o normal",
		"5% quieter" => "5% mais baixo",
		"5% louder" => "5% mais alto",
		"Bots start at 50% to protect your hearing. You can still raise it here." => {
			"Bots começam em 50% para proteger sua audição. Você ainda pode aumentar aqui."
		}
		"Start bots at 50% volume" => "Bots começam com 50% de volume",
		"Protects your hearing from bots that join very loud. Right-click a bot in the call to change its volume." => {
			"Protege sua audição de bots que entram muito altos. Clique com o botão direito num bot na chamada para mudar o volume dele."
		}
		"Light" => "Leve",
		"Maximum" => "Máxima",
		"Recommended" => "Recomendado",
		"All voice settings" => "Todas as configurações de voz",
		"No filter. For studio microphones or when playing music." => {
			"Sem filtro. Para microfones de estúdio ou quando for tocar música."
		}
		"Steady hum like fans or air conditioning. Lightest on your PC." => {
			"Zumbido constante, como ventilador ou ar-condicionado. O mais leve para o PC."
		}
		"Keyboard, clicks and everyday home noise. Works well for most people." => {
			"Teclado, cliques e barulhos do dia a dia em casa. Funciona bem para a maioria."
		}
		"Very noisy home, or friends complain about your background noise. Uses more of your PC." => {
			"Casa muito barulhenta ou amigos reclamando do seu barulho. Usa mais o PC."
		}
		"PC usage: none" => "Uso do PC: nenhum",
		"PC usage: very low" => "Uso do PC: muito baixo",
		"PC usage: low" => "Uso do PC: baixo",
		"PC usage: medium" => "Uso do PC: médio",
		"Friends complaining about noise? Choose Maximum. If your voice cuts out or your PC slows down, go back to Standard." => {
			"Amigos reclamando do barulho? Escolha Máxima. Se sua voz começar a picotar ou o PC ficar lento, volte para Padrão."
		}
		"Your PC couldn't keep up with Maximum, so Nivra switched to Standard to keep your voice smooth." => {
			"Seu PC não acompanhou a Máxima, então o Nivra voltou para Padrão para sua voz não travar."
		}
		"The defaults work for most people. Change these only if something sounds wrong." => {
			"Os padrões servem para a maioria. Mude só se algo estiver soando errado."
		}
		"Click to turn on or off · right-click to choose the level" => {
			"Clique para ligar ou desligar · botão direito para escolher o nível"
		}
		"Noise suppression is unavailable in this build or preview." => {
			"A supressão de ruído não está disponível nesta versão ou prévia."
		}
		"Share a screen or window" => "Compartilhar uma tela ou janela",
		"Stop sharing your screen" => "Parar de compartilhar a tela",
		"Stop sharing your camera" => "Parar de compartilhar a câmera",
		"Share your selected camera with this call" => {
			"Compartilhar a câmera escolhida nesta chamada"
		}
		"Turn off noise suppression" => "Desligar supressão de ruído",
		"Turn on noise suppression" => "Ligar supressão de ruído",
		"Voice processing" => "Processamento de voz",
		"Voice processing & input mode" => "Processamento de voz e modo de entrada",
		"Mode" => "Modo",
		"Off" => "Desligado",
		"Standard" => "Padrão",
		"Echo cancellation" => "Cancelamento de eco",
		"Recommended when speakers can be picked up by your microphone." => {
			"Recomendado quando o alto-falante pode ser captado pelo microfone."
		}
		"Automatic microphone volume" => "Volume automático do microfone",
		"Keeps speech at a more consistent loudness without changing your output volume." => {
			"Mantém a fala num volume mais estável, sem mudar o volume de saída."
		}
		"Push to talk" => "Apertar para falar",
		"When enabled, your microphone transmits only while the configured shortcut is held." => {
			"Com isso ligado, o microfone só transmite enquanto o atalho estiver pressionado."
		}
		"Mute and deafen always take priority." => {
			"Silenciar o microfone e o áudio sempre tem prioridade."
		}
		"Hold your configured shortcut when you want to speak." => {
			"Segure o atalho configurado quando quiser falar."
		}
		"Deafen turns off incoming audio and mutes your microphone with it." => {
			"Ensurdecer desliga o áudio da chamada e silencia o microfone junto."
		}
		"Advanced input settings" => "Ajustes avançados de entrada",
		"Voice activity threshold" => "Limite de atividade de voz",
		"Only transmit sound above the threshold." => "Só transmite som acima do limite.",
		"Open voice activity; mute and push to talk still apply." => {
			"Microfone aberto; silenciar e apertar para falar continuam valendo."
		}
		"Input level" => "Nível de entrada",
		"Light suppression strength" => "Intensidade do nível Leve",
		"Higher levels remove more noise but can affect natural voice detail." => {
			"Níveis mais altos tiram mais ruído, mas podem mudar o detalhe natural da voz."
		}
		"Low" => "Baixo",
		"Moderate" => "Moderado",
		"High" => "Alto",
		"Very high" => "Muito alto",
		"Recommended defaults" => "Padrões recomendados",
		"Raw microphone" => "Microfone sem tratamento",
		"Devices & levels" => "Dispositivos e volumes",
		"Input device" => "Dispositivo de entrada",
		"Output device" => "Dispositivo de saída",
		"Microphone gain" => "Ganho do microfone",
		"Speaker volume" => "Volume do alto-falante",
		"100% is the original level. Higher levels may distort." => {
			"100% é o nível original. Acima disso pode distorcer."
		}
		"Rescan devices" => "Procurar dispositivos de novo",
		"Reset levels" => "Restaurar volumes",
		"System default follows your operating-system choice. Select a device only when you want Nivra to stay pinned to it." => {
			"O padrão do sistema segue a escolha do sistema operacional. Escolha um dispositivo só quando quiser que o Nivra fique nele."
		}
		"System default (recommended)" => "Padrão do sistema (recomendado)",
		"Device unavailable — choose another" => "Dispositivo indisponível — escolha outro",
		"Looking for audio devices..." => "Procurando dispositivos de áudio...",
		"Looking for audio devices…" => "Procurando dispositivos de áudio…",
		"Could not start audio device discovery" => {
			"Não foi possível procurar os dispositivos de áudio"
		}
		"Audio devices loaded · headphones avoid microphone echo" => {
			"Dispositivos de áudio carregados · fone evita eco do microfone"
		}
		"Audio device discovery stopped" => "A busca de dispositivos de áudio parou",
		"One selected audio device is unavailable. Choose System default or rescan devices." => {
			"Um dispositivo de áudio escolhido não está disponível. Use o padrão do sistema ou procure de novo."
		}
		"Microphone unavailable · choose another input. You are still connected." => {
			"Microfone indisponível · escolha outra entrada. Você continua na chamada."
		}
		"Microphone unavailable · still connected. Choose another input in Audio settings." => {
			"Microfone indisponível · você continua na chamada. Escolha outra entrada em Áudio."
		}
		"Camera" => "Câmera",
		"Voice privacy code" => "Código de privacidade da voz",
		"Call without end-to-end encryption" => "Chamada sem criptografia de ponta a ponta",
		"Compare with the other participants. This code changes with the encrypted call group." => {
			"Compare com os outros participantes. Este código muda com o grupo criptografado da chamada."
		}
		"Audio preferences are saved on this device. Your microphone starts only when you join a call or start testing." => {
			"As preferências de áudio ficam neste dispositivo. O microfone só liga quando você entra numa chamada ou começa um teste."
		}
		"Install a voice-enabled build to use these controls." => {
			"Use uma versão com voz para estes controles."
		}
		"Friends" => "Amigos",
		"Add Friend" => "Adicionar amigo",
		"You can add friends with their Discord username." => {
			"Você pode adicionar amigos pelo nome de usuário do Discord."
		}
		"Username" => "Nome de usuário",
		"Enter a username" => "Digite um nome de usuário",
		"Sending…" => "Enviando…",
		"Send Friend Request" => "Enviar pedido de amizade",
		"Offline demo · actions are simulated." => {
			"Demonstração offline · as ações são simuladas."
		}
		"Reconnect before sending a friend request." => {
			"Reconecte antes de enviar um pedido de amizade."
		}
		"All" => "Todos",
		"Pending" => "Pendentes",
		"Blocked & Ignored" => "Bloqueados e ignorados",
		"All friends" => "Todos os amigos",
		"Blocked & ignored" => "Bloqueados e ignorados",
		"Blocked and ignored users are not available yet." => {
			"Usuários bloqueados e ignorados ainda não estão disponíveis."
		}
		"Friends are not available yet." => "Os amigos ainda não estão disponíveis.",
		"No blocked or ignored users match your search." => {
			"Nenhum usuário bloqueado ou ignorado corresponde à busca."
		}
		"No friends match your search." => "Nenhum amigo corresponde à busca.",
		"No blocked or ignored users." => "Nenhum usuário bloqueado ou ignorado.",
		"No friends yet." => "Nenhum amigo ainda.",
		"No friends are currently online." => "Nenhum amigo está online agora.",
		"Some friends’ online status and activity couldn’t be loaded. The Online list may be incomplete." => {
			"O status e a atividade de alguns amigos não carregaram. A lista Online pode estar incompleta."
		}
		"Dismiss friend status warning" => "Dispensar aviso de status dos amigos",
		"Direct Messages" => "Mensagens diretas",
		"Spam Filters" => "Filtros de spam",
		"Friend Requests" => "Pedidos de amizade",
		"Connected Games" => "Jogos conectados",
		"Direct Message (DM) Permissions" => "Permissões de mensagem direta (DM)",
		"Friend Request Permissions" => "Permissões de pedido de amizade",
		"Messaging in Connected Games" => "Mensagens em jogos conectados",
		"Automatically filter suspected spam messages" => {
			"Filtrar automaticamente mensagens suspeitas de spam"
		}
		"Discord can filter out some messages that contain spam. These messages go to your Spam inbox." => {
			"O Discord pode filtrar algumas mensagens com spam. Elas vão para sua caixa de spam."
		}
		"Filter all spam" => "Filtrar todo o spam",
		"Filter messages from non-friends" => "Filtrar mensagens de quem não é amigo",
		"Don't filter spam" => "Não filtrar spam",
		"Your account uses a custom spam filter setting. Select an option to replace it." => {
			"Sua conta usa um filtro de spam personalizado. Selecione uma opção para substituí-lo."
		}
		"All servers" => "Todos os servidores",
		"Server" => "Servidor",
		"Some servers have different preferences. Choose a server to review its settings." => {
			"Alguns servidores têm preferências diferentes. Escolha um servidor para ver as configurações."
		}
		"Changes apply to all current servers and set the default for newly joined servers." => {
			"As mudanças valem para os servidores atuais e viram o padrão dos novos servidores."
		}
		"Changes apply to this server only." => "As mudanças valem só para este servidor.",
		"Allow DMs from other server members" => "Permitir DMs de outros membros do servidor",
		"Filter messages from server members I may not know" => {
			"Filtrar mensagens de membros que talvez eu não conheça"
		}
		"Move messages from people you may not know into Message Requests." => {
			"Move mensagens de pessoas que você talvez não conheça para Solicitações de Mensagem."
		}
		"There are too many servers to update together. Choose an individual server." => {
			"Há servidores demais para atualizar juntos. Escolha um servidor."
		}
		"Saving…" => "Salvando…",
		"Loading your preferences…" => "Carregando suas preferências…",
		"Try again" => "Tentar de novo",
		"Allow friend requests from" => "Permitir pedidos de amizade de",
		"Control who can send you friend requests and how they appear." => {
			"Controle quem pode mandar pedidos de amizade e como eles aparecem."
		}
		"Everyone" => "Todos",
		"Friends of friends" => "Amigos de amigos",
		"Server members" => "Membros do servidor",
		"Only from servers where you also allow Direct Messages." => {
			"Só de servidores onde você também permite mensagens diretas."
		}
		"Show personalized messages" => "Mostrar mensagens personalizadas",
		"Show personalized messages on incoming friend requests. If you accept, the message will still appear in your DMs." => {
			"Mostra mensagens personalizadas nos pedidos recebidos. Se você aceitar, ela continua nas suas DMs."
		}
		"Settings for games that use Discord to power their social experiences." => {
			"Configurações de jogos que usam o Discord nas experiências sociais."
		}
		"Allow friends from games to send direct messages and invites" => {
			"Permitir que amigos de jogos mandem mensagens e convites"
		}
		"Let friends from connected games send DMs and invite you to play, even when the game isn't open." => {
			"Deixa amigos de jogos conectados mandarem DMs e convidarem para jogar, mesmo com o jogo fechado."
		}
		"Show Direct Messages in games" => "Mostrar mensagens diretas nos jogos",
		"Read and respond to DMs directly from in-game chats." => {
			"Leia e responda DMs direto das conversas no jogo."
		}
		"Show all DMs" => "Mostrar todas as DMs",
		"Show only DMs from people who also play the game" => "Mostrar só DMs de quem também joga",
		"Don't show DMs" => "Não mostrar DMs",
		"Your account uses a custom in-game DM setting. Select an option to replace it." => {
			"Sua conta usa uma configuração personalizada de DM no jogo. Selecione uma opção para substituí-la."
		}
		"Mark As Read" => "Marcar como lida",
		"Server Settings" => "Configurações do servidor",
		"Create invite" => "Criar convite",
		"Leave server" => "Sair do servidor",
		"Leave server?" => "Sair do servidor?",
		"Are you sure you want to leave" => "Tem certeza que quer sair de",
		"You will not be able to rejoin this server unless you are re-invited." => {
			"Você não poderá voltar a este servidor sem um novo convite."
		}
		"Leaving…" => "Saindo…",
		"Leave Server" => "Sair do servidor",
		"Close" => "Fechar",
		"Showing the beginning of a large file." => "Mostrando o início de um arquivo grande.",
		"Show more" => "Mostrar mais",
		"Offline preview · no server changes" => "Prévia offline · nenhuma mudança no servidor",
		"Remove From Favorites" => "Remover dos favoritos",
		"Add To Favorites" => "Adicionar aos favoritos",
		"Favorites are saved on this device." => "Os favoritos ficam salvos neste dispositivo.",
		"Invite to Channel" => "Convidar para o canal",
		"Copy Link" => "Copiar link",
		"Unmute Channel" => "Reativar canal",
		"Mute Channel" => "Silenciar canal",
		"For 15 Minutes" => "Por 15 minutos",
		"For 1 Hour" => "Por 1 hora",
		"For 3 Hours" => "Por 3 horas",
		"For 8 Hours" => "Por 8 horas",
		"For 24 Hours" => "Por 24 horas",
		"Until I Turn It Back On" => "Até eu reativar",
		"Copy Channel ID" => "Copiar ID do canal",
		"Hide Muted Channels" => "Ocultar canais silenciados",
		"Restart to update" => "Reiniciar para atualizar",
		"Updating…" => "Atualizando…",
		"Update available" => "Atualização disponível",
		"Dismiss update" => "Dispensar atualização",
		"Download update" => "Baixar atualização",
		"Check for updates" => "Verificar atualizações",
		"Update checks are disabled in debug builds." => {
			"Verificações de atualização estão desligadas em compilações de depuração."
		}
		"Finish the current update before checking again." => {
			"Termine a atualização atual antes de verificar de novo."
		}
		"Search themes" => "Buscar temas",
		"Search extensions" => "Buscar extensões",
		"Checking for packages and updates" => "Verificando pacotes e atualizações",
		"Working on your last action" => "Executando sua última ação",
		"Clear search" => "Limpar busca",
		"Create theme" => "Criar tema",
		"More" => "Mais",
		"Import theme…" => "Importar tema…",
		"Refresh catalog" => "Atualizar catálogo",
		"Look for new packages and updates. Nothing installs on its own." => {
			"Busca novos pacotes e atualizações. Nada instala sozinho."
		}
		"Import package…" => "Importar pacote…",
		"Open a package file from this computer." => "Abrir um arquivo de pacote deste computador.",
		"No matches" => "Sem resultados",
		"No themes yet" => "Nenhum tema ainda",
		"No extensions yet" => "Nenhuma extensão ainda",
		"Refresh the catalog or import a creator's package to get started." => {
			"Atualize o catálogo ou importe o pacote de um criador para começar."
		}
		"Try a different name or creator." => "Tente outro nome ou criador.",
		"Preview" => "Pré-visualizar",
		"View preview" => "Ver prévia",
		"Loading preview..." => "Carregando prévia...",
		"Preview not loaded" => "Prévia não carregada",
		"Preview unavailable" => "Prévia indisponível",
		"Previewing theme" => "Pré-visualizando tema",
		"Theme preview" => "Prévia do tema",
		"Changes are not saved yet" => "Mudanças ainda não salvas",
		"Back to themes" => "Voltar aos temas",
		"Back to theme editor" => "Voltar ao editor",
		"Customize" => "Personalizar",
		"Use theme" => "Usar tema",
		"Apply this installed theme to the app." => "Aplica este tema instalado ao app.",
		"Edit theme" => "Editar tema",
		"Open tool" => "Abrir ferramenta",
		"Disable" => "Desativar",
		"Update" => "Atualizar",
		"Active" => "Ativo",
		"Enabled" => "Ativado",
		"Remove" => "Remover",
		"by" => "por",
		"Plugin" => "Plugin",
		"Cleanup pending" => "Limpeza pendente",
		"Retry cleanup" => "Tentar limpeza de novo",
		"Add a new tool to your conversations." => {
			"Adicione uma ferramenta nova às suas conversas."
		}
		"Install theme" => "Instalar tema",
		"Review & enable" => "Revisar e ativar",
		"Review the new release before it replaces this version." => {
			"Revise a nova versão antes que ela substitua esta."
		}
		"Remove this theme and delete its local data." => {
			"Remove este tema e apaga os dados locais dele."
		}
		"Finish removing this extension and its local data." => {
			"Termina de remover esta extensão e os dados locais dela."
		}
		"Removes this extension and deletes its local data." => {
			"Remove esta extensão e apaga os dados locais dela."
		}
		"Selecting artwork sends it as an image attachment." => {
			"Escolher arte envia como anexo de imagem."
		}
		"Example deleted-message appearance" => "Exemplo de mensagem apagada",
		"Creator preview" => "Prévia do criador",
		"Close preview" => "Fechar prévia",
		"Reviewed" => "Revisado",
		"Unreviewed" => "Não revisado",
		"View source" => "Ver código-fonte",
		"Unreviewed package — its source has not been reviewed for the catalog." => {
			"Pacote não revisado — o código-fonte não foi revisado para o catálogo."
		}
		"No access to conversations or composer text." => {
			"Sem acesso a conversas ou ao texto do editor."
		}
		"Allow this extension to" => "Permitir que esta extensão",
		"Enable this theme" => "Ativar este tema",
		"Enable this extension" => "Ativar esta extensão",
		"Everything it may touch is listed below." => {
			"Tudo que ela pode acessar está listado abaixo."
		}
		"Allow every listed permission to continue." => {
			"Permita todas as permissões listadas para continuar."
		}
		"Enable explicit emoji and sticker image attachment selection" => {
			"Permitir escolha de imagens de emoji e figurinha"
		}
		"Customize app colors, typography and control styling" => {
			"Personalizar cores, tipografia e controles do app"
		}
		"Read live message events and text in the active conversation" => {
			"Ler eventos e texto da conversa ativa"
		}
		"Read the message I choose for an action" => "Ler a mensagem que eu escolher",
		"Read my draft and propose text changes" => "Ler meu rascunho e sugerir textos",
		"Store up to 1 MiB of local data for this account" => {
			"Guardar até 1 MiB de dados locais desta conta"
		}
		"Read my account and current conversation details" => {
			"Ler minha conta e detalhes da conversa atual"
		}
		"Read my loaded profile, including biography and pronouns" => {
			"Ler meu perfil, incluindo bio e pronomes"
		}
		"Read my loaded server names and identifiers" => {
			"Ler nomes e identificadores dos meus servidores"
		}
		"Read current channel metadata, recipients and permissions" => {
			"Ler metadados, participantes e permissões do canal"
		}
		"Receive changes to separately granted account and conversation data" => {
			"Receber mudanças de dados com permissão separada"
		}
		"Read loaded embed text, stickers and message reference metadata" => {
			"Ler embeds, figurinhas e referências de mensagens"
		}
		"Read loaded forum and thread summaries" => "Ler resumos de fóruns e tópicos",
		"Read current typing users and loaded pins; observe reactions" => {
			"Ler quem está digitando, fixados e reações"
		}
		"Read loaded channel topics, categories, thread details and permissions" => {
			"Ler tópicos, categorias, tópicos de discussão e permissões"
		}
		"Read loaded server members, roles and server profiles" => {
			"Ler membros, cargos e perfis dos servidores"
		}
		"Read the list of loaded, readable conversations" => "Ler a lista de conversas legíveis",
		"Read loaded message replies, mentions, attachment metadata and reactions" => {
			"Ler respostas, menções, anexos e reações"
		}
		"Read my loaded friends, requests, blocked and ignored users" => {
			"Ler amigos, pedidos, bloqueados e ignorados"
		}
		"Read loaded messages in the active conversation" => "Ler mensagens da conversa ativa",
		"Read loaded members of the active conversation" => "Ler membros da conversa ativa",
		"Read loaded user presence status" => "Ler status de presença",
		"Read current call state and participant identifiers" => {
			"Ler estado da chamada e participantes"
		}
		"Read unread and mention counts in the active conversation" => {
			"Ler não lidas e menções da conversa ativa"
		}
		"Enable" => "Ativar",
		"PNG or JPEG, up to 2 MiB. This image does not change the chat background." => {
			"PNG ou JPEG, até 2 MiB. Não muda o fundo do chat."
		}
		"Disabling removes the extension and its local data. Re-enabling starts fresh." => {
			"Desativar remove a extensão e os dados locais. Reativar começa do zero."
		}
		"Save Changes" => "Salvar mudanças",
		"Back" => "Voltar",
		"Working…" => "Trabalhando…",
		"Theme details" => "Detalhes do tema",
		"How your theme appears in the gallery." => "Como seu tema aparece na galeria.",
		"Theme name" => "Nome do tema",
		"My theme" => "Meu tema",
		"Theme name is required." => "Nome do tema é obrigatório.",
		"Created by" => "Criado por",
		"Your name" => "Seu nome",
		"Creator name is required." => "Nome do criador é obrigatório.",
		"Card cover" => "Capa do cartão",
		"Choose the image shown on your theme card in Themes." => {
			"Escolha a imagem do cartão do tema em Temas."
		}
		"App background" => "Fundo do app",
		"Use one image behind your conversations and sidebars." => {
			"Use uma imagem atrás das conversas e barras laterais."
		}
		"This older theme uses its original image placement." => {
			"Este tema antigo usa o posicionamento original da imagem."
		}
		"Use image across the app" => "Usar imagem no app todo",
		"Preview in app" => "Pré-visualizar no app",
		"Save and apply" => "Salvar e aplicar",
		"Basics" => "Básico",
		"Background" => "Fundo",
		"Colors" => "Cores",
		"Advanced" => "Avançado",
		"Discard unsaved theme?" => "Descartar tema não salvo?",
		"Your changes have not been saved." => "Suas mudanças não foram salvas.",
		"Discard changes" => "Descartar mudanças",
		"Keep editing" => "Continuar editando",
		"Custom cover" => "Capa personalizada",
		"Automatic preview" => "Prévia automática",
		"Replace cover" => "Trocar capa",
		"Choose cover" => "Escolher capa",
		"Background image" => "Imagem de fundo",
		"No image selected" => "Nenhuma imagem",
		"Replace image" => "Trocar imagem",
		"Choose image" => "Escolher imagem",
		"Editing" => "Editando",
		"Dark" => "Escuro",
		"Image opacity" => "Opacidade da imagem",
		"Image fit" => "Ajuste da imagem",
		"Fill area" => "Preencher área",
		"Fit entire image" => "Imagem inteira",
		"Section opacity" => "Opacidade da seção",
		"Select an area, then choose how much of the image shows through." => {
			"Selecione uma área e escolha quanto da imagem aparece."
		}
		"Window gradient" => "Degradê da janela",
		"More colors" => "Mais cores",
		"Surface opacity" => "Opacidade da superfície",
		"0% shows the image. 100% is a solid section color." => {
			"0% mostra a imagem. 100% é cor sólida."
		}
		"Selected section" => "Seção selecionada",
		"Top bars" => "Barras superiores",
		"Server list" => "Lista de servidores",
		"People & channels" => "Pessoas e canais",
		"Message list" => "Lista de mensagens",
		"Member list" => "Lista de membros",
		"Message input area" => "Área de digitação",
		"Window title and conversation header" => "Título da janela e cabeçalho",
		"The left server rail" => "A barra de servidores à esquerda",
		"Direct messages and channel navigation" => "DMs e navegação de canais",
		"The conversation timeline" => "A linha da conversa",
		"The member and search pane on the right" => "O painel de membros e busca à direita",
		"The area around the message box" => "A área ao redor da caixa de mensagem",
		"Window background" => "Fundo da janela",
		"Sidebar" => "Barra lateral",
		"Message area" => "Área de mensagens",
		"Cards & message input" => "Cartões e campo de mensagem",
		"Hover" => "Passar o mouse",
		"Selection" => "Seleção",
		"Borders" => "Bordas",
		"Headings" => "Títulos",
		"Body text" => "Texto do corpo",
		"Secondary text" => "Texto secundário",
		"Text on accent" => "Texto no destaque",
		"Success" => "Sucesso",
		"Warning" => "Aviso",
		"Error & danger" => "Erro e perigo",
		"Mention background" => "Fundo de menção",
		"Mention text" => "Texto de menção",
		"Buttons, selection and highlights" => "Botões, seleção e destaques",
		"Messages and regular labels" => "Mensagens e rótulos comuns",
		"Timestamps and supporting text" => "Horários e textos de apoio",
		"Channel, conversation and member lists" => "Listas de canais, conversas e membros",
		"Background behind your messages" => "Fundo atrás das suas mensagens",
		"Use the default color for this appearance" => "Usar a cor padrão desta aparência",
		"Use #RRGGBB or #RRGGBBAA." => "Use #RRGGBB ou #RRGGBBAA.",
		"Horizontal" => "Horizontal",
		"Vertical" => "Vertical",
		"Use the built-in value" => "Usar o valor original",
		"Text, spacing & corners" => "Texto, espaçamento e cantos",
		"These settings apply to dark and light appearances." => {
			"Estas configurações valem para aparência clara e escura."
		}
		"Buttons" => "Botões",
		"Small text" => "Texto pequeno",
		"Code" => "Código",
		"Control height" => "Altura dos controles",
		"Item spacing" => "Espaço entre itens",
		"Button padding" => "Margem dos botões",
		"Control corners" => "Cantos dos controles",
		"Window corners" => "Cantos da janela",
		"Menu corners" => "Cantos dos menus",
		"Sharing & export" => "Compartilhar e exportar",
		"The license and version are required. A source URL is optional for local themes." => {
			"Licença e versão são obrigatórias. URL de origem é opcional para temas locais."
		}
		"License" => "Licença",
		"Version" => "Versão",
		"Source URL" => "URL de origem",
		"Optional" => "Opcional",
		"Use a valid HTTPS source URL or leave this blank." => {
			"Use uma URL de origem HTTPS válida ou deixe em branco."
		}
		"License and version are required." => "Licença e versão são obrigatórias.",
		"Only share images you own or have permission to use. Keep required attribution." => {
			"Só compartilhe imagens suas ou com permissão. Mantenha os créditos exigidos."
		}
		"Export theme" => "Exportar tema",
		"Add a theme name and creator name before saving." => {
			"Adicione nome do tema e do criador antes de salvar."
		}
		"Add a license and version before saving." => "Adicione licença e versão antes de salvar.",
		"Check the license, version, and optional source URL." => {
			"Verifique licença, versão e URL de origem opcional."
		}
		"Correct the highlighted color value." => "Corrija a cor destacada.",
		"Keep image and section opacity between 0% and 100%." => {
			"Mantenha a opacidade da imagem e das seções entre 0% e 100%."
		}
		"Correct the highlighted gradient value." => "Corrija o degradê destacado.",
		"Check the remaining theme settings before saving." => {
			"Verifique o resto das configurações do tema antes de salvar."
		}
		"Reload server settings" => "Recarregar configurações",
		"Loading server settings…" => "Carregando configurações…",
		"Reconnect to load server settings." => "Reconecte para carregar as configurações.",
		"Load server settings" => "Carregar configurações",
		"Discard unsaved changes?" => "Descartar mudanças não salvas?",
		"Your changes to this server will be lost." => {
			"Suas mudanças neste servidor serão perdidas."
		}
		"Wait for the current save to finish before closing." => {
			"Aguarde o salvamento atual terminar antes de fechar."
		}
		"Delete server" => "Excluir servidor",
		"This action cannot be undone." => "Esta ação não pode ser desfeita.",
		"Enter server name" => "Digite o nome do servidor",
		"Deleting…" => "Excluindo…",
		"Delete Server" => "Excluir servidor",
		"Server Profile" => "Perfil do servidor",
		"Customize how your server appears in invite links and, if enabled, in Server Discovery and Announcement Channel messages." => {
			"Personalize como seu servidor aparece em links de convite e, se ativado, na Descoberta e em mensagens de canal de anúncios."
		}
		"Name" => "Nome",
		"Icon" => "Ícone",
		"We recommend an image of at least 512×512." => {
			"Recomendamos imagem de pelo menos 512×512."
		}
		"Preparing icon…" => "Preparando ícone…",
		"Change Server Icon" => "Trocar ícone do servidor",
		"Remove Icon" => "Remover ícone",
		"Banner" => "Banner",
		"Traits" => "Características",
		"Add up to 5 traits to show off your server's interests and personality." => {
			"Adicione até 5 características para mostrar os interesses do servidor."
		}
		"Trait name" => "Nome da característica",
		"Remove trait" => "Remover característica",
		"Description" => "Descrição",
		"How did your server get started? Why should people join?" => {
			"Como seu servidor começou? Por que entrar?"
		}
		"Tell the world a bit about this server." => "Conte um pouco sobre este servidor.",
		"Saving changes…" => "Salvando mudanças…",
		"Careful — you have unsaved changes!" => "Cuidado — há mudanças não salvas!",
		"Use a server name of 2–100 characters, a description of up to 300 characters, and valid traits without control characters." => {
			"Use nome de 2–100 caracteres, descrição de até 300 e características válidas."
		}
		"Could not save these changes. Check the selected channels, reconnect, or reload the server settings and try again." => {
			"Não foi possível salvar. Verifique os canais, reconecte ou recarregue e tente de novo."
		}
		"Reload the server settings before saving again. Your edits will be kept." => {
			"Recarregue as configurações antes de salvar de novo. Suas edições serão mantidas."
		}
		"Reconnect to save changes." => "Reconecte para salvar as mudanças.",
		"MODERATION" => "MODERAÇÃO",
		"APPS" => "APPS",
		"EXPRESSION" => "EXPRESSÃO",
		"PEOPLE" => "PESSOAS",
		"Engagement" => "Engajamento",
		"Manage settings that help keep your server active." => {
			"Configurações que mantêm seu servidor ativo."
		}
		"System Messages" => "Mensagens do sistema",
		"Configure system event messages sent to your server." => {
			"Configure mensagens de eventos do sistema."
		}
		"Send a random welcome message when someone joins this server." => {
			"Enviar boas-vindas aleatórias quando alguém entrar."
		}
		"Prompt members to reply to welcome messages with a sticker." => {
			"Pedir que membros respondam boas-vindas com figurinha."
		}
		"Send a message when someone boosts this server." => {
			"Enviar mensagem quando alguém impulsionar o servidor."
		}
		"Send helpful tips for server setup." => "Enviar dicas de configuração.",
		"System Messages Channel" => "Canal de mensagens do sistema",
		"This is the channel we send system event messages to." => {
			"É para este canal que enviamos mensagens do sistema."
		}
		"Activity Feed Settings" => "Feed de atividades",
		"Shows a feed of activity from games and connected apps in this server." => {
			"Mostra atividades de jogos e apps conectados."
		}
		"Display Activity Feed in this server" => "Mostrar feed de atividades aqui",
		"Server default" => "Padrão do servidor",
		"Default Notification Settings" => "Notificações padrão",
		"This will determine whether members who have not explicitly set their notification settings receive a notification for every message sent in this server or not." => {
			"Define se membros sem configuração própria recebem notificação de cada mensagem."
		}
		"All Messages" => "Todas as mensagens",
		"Only @mentions" => "Só @menções",
		"We highly recommend setting this to only @mentions for a Community Server." => {
			"Recomendamos só @menções para servidor de comunidade."
		}
		"Inactive Channel" => "Canal de inativos",
		"Inactive Timeout" => "Tempo de inatividade",
		"Automatically move members to this channel and mute them when they have been idle for longer than the inactive timeout. This does not affect browsers." => {
			"Move membros inativos para este canal e silencia. Não afeta navegadores."
		}
		"Unavailable channel" => "Canal indisponível",
		"No Inactive Channel" => "Sem canal de inativos",
		"No System Messages Channel" => "Sem canal de sistema",
		"None" => "Nenhum",
		"No accessible channels available." => "Nenhum canal acessível.",
		"Stickers" => "Figurinhas",
		"Members" => "Membros",
		"Invites" => "Convites",
		"Integrations" => "Integrações",
		"Audit Log" => "Registro de auditoria",
		"Navigation" => "Navegação",
		"Move around Nivra without reaching for the mouse." => {
			"Navegue pelo Nivra sem usar o mouse."
		}
		"Messages" => "Mensagens",
		"Composer shortcuts are only active while you are writing." => {
			"Os atalhos do compositor só valem enquanto você está escrevendo."
		}
		"Text Formatting" => "Formatação de texto",
		"Apply or remove formatting in the composer." => {
			"Aplica ou remove formatação no compositor."
		}
		"Global availability" => "Disponibilidade global",
		"Show Keyboard Shortcuts List" => "Mostrar lista de atalhos",
		"Switch Conversation" => "Trocar de conversa",
		"Close Settings or Dialog" => "Fechar configurações ou diálogo",
		"Send Message" => "Enviar mensagem",
		"Insert New Line" => "Inserir nova linha",
		"Edit Last Editable Message" => "Editar a última mensagem editável",
		"Bold" => "Negrito",
		"Italic" => "Itálico",
		"Underline" => "Sublinhado",
		"Strikethrough" => "Tachado",
		"Inline Code" => "Código em linha",
		"Code Block" => "Bloco de código",
		"Spoiler" => "Spoiler",
		"Push to Talk" => "Apertar para falar",
		"Toggle Mute" => "Alternar mudo",
		"Toggle Deafen" => "Alternar áudio",
		"Voice" => "Voz",
		"Control your microphone and incoming audio during a connected call." => {
			"Controla o microfone e o áudio da chamada enquanto você está conectado."
		}
		"Already bound to" => "Já usado por",
		"Brazil" => "Brasil",
		"United States" => "Estados Unidos",
		"Canada" => "Canadá",
		"United Kingdom" => "Reino Unido",
		"Germany" => "Alemanha",
		"Netherlands" => "Países Baixos",
		"France" => "França",
		"Spain" => "Espanha",
		"Poland" => "Polônia",
		"Finland" => "Finlândia",
		"Sweden" => "Suécia",
		"Singapore" => "Singapura",
		"Japan" => "Japão",
		"Hong Kong" => "Hong Kong",
		"Australia" => "Austrália",
		"India" => "Índia",
		"South Africa" => "África do Sul",
		"Chile" => "Chile",
		"Argentina" => "Argentina",
		"South Korea" => "Coreia do Sul",
		"Europe" => "Europa",
		"Russia" => "Rússia",
		"People in this call will see what you pick." => {
			"As pessoas nesta chamada verão o que você escolher."
		}
		"Looking for your screens…" => "Procurando suas telas…",
		"Entire screen" => "Tela inteira",
		"Share audio" => "Compartilhar áudio",
		"Also send sound from other apps. Your microphone stays as it is." => {
			"Também envia o som de outros apps. O microfone continua como está."
		}
		"Share an app" => "Compartilhar um app",
		"No apps are open to share." => "Não há apps abertos para compartilhar.",
		"App" => "App",
		"Refresh" => "Atualizar",
		"Quality" => "Qualidade",
		"Show cursor" => "Mostrar cursor",
		"Include the pointer in the shared video." => "Inclui o ponteiro no vídeo compartilhado.",
		"Share Screen" => "Compartilhar tela",
		"Before you use Nivra" => "Antes de usar o Nivra",
		"Nivra is an unofficial app for your own Discord account: it is not Discord, it is not endorsed by Discord, and Discord's rules still apply to your account. It also includes other people's work (libraries, fonts and icons) under their own licenses, and accepting here does not waive those licenses or shift their copyright. Continuing confirms that you understand both points." => {
			"O Nivra é um aplicativo não oficial para a sua própria conta do Discord: não é o Discord, não é endossado pelo Discord, e as regras do Discord continuam valendo para a sua conta. Ele também inclui trabalho de outras pessoas (bibliotecas, fontes e ícones) sob as licenças delas, e aceitar aqui não anula essas licenças nem transfere o direito autoral delas. Ao continuar, você confirma que entendeu esses dois pontos."
		}
		"Your app preferences could not be read, so Nivra cannot tell whether you accepted this before." => {
			"Não foi possível ler as preferências do aplicativo, então o Nivra não sabe se você já aceitou isto antes."
		}
		"View full licenses" => "Ver licenças completas",
		"Hide full licenses" => "Ocultar licenças completas",
		"Hide offline members" => "Ocultar membros offline",
		"Show offline members" => "Mostrar membros offline",
		"Display" => "Exibição",
		"GLOBAL" => "GLOBAL",
		"Send as text file?" => "Enviar como arquivo de texto?",
		"Your message is too long for chat, so it will be sent as a file." => {
			"Sua mensagem é longa demais para o chat e será enviada como arquivo."
		}
		"File name" => "Nome do arquivo",
		"Send" => "Enviar",
		"That file name will not work." => "Esse nome de arquivo não vai funcionar.",
		"Licenses" => "Licenças",
		"Legal" => "Informações legais",
		"Licenses for the libraries, fonts, icons and sounds included in Nivra." => {
			"Licenças das bibliotecas, fontes, ícones e sons incluídos no Nivra."
		}
		"Filter licenses" => "Filtrar licenças",
		"No licenses match this filter." => "Nenhuma licença corresponde a este filtro.",
		"All licenses" => "Todas as licenças",
		"Source code for the MPL-2.0 components in this version is published on its release page as" => {
			"O código-fonte dos componentes sob MPL-2.0 desta versão está publicado na página de lançamento dela como"
		}
		"Notification sounds" => "Sons de notificação",
		"Fonts" => "Fontes",
		"Emoji" => "Emojis",
		"Icons" => "Ícones",
		"Core libraries" => "Bibliotecas principais",
		"Sign-in" => "Entrada na conta",
		"Audio playback" => "Reprodução de áudio",
		"Other dependencies" => "Outras dependências",
		"App preferences were not saved. If your acceptance of the terms was not recorded, Nivra will ask again next launch." => {
			"As preferências do aplicativo não foram salvas. Se o seu aceite dos termos não foi registrado, o Nivra vai perguntar de novo na próxima vez que abrir."
		}
		"Your session expired; sign in again to continue." => {
			"Sua sessão expirou; entre de novo para continuar."
		}
		"I understand — continue" => "Entendi — continuar",
		"Zoom" => "Zoom",
		"Scales text and controls across the app." => {
			"Ajusta o texto e os controles em todo o aplicativo."
		}
		"Layout" => "Layout",
		"Reset layout" => "Redefinir layout",
		"Sidebar width" => "Largura da barra lateral",
		"Channel and conversation list width in wide windows." => {
			"Largura da lista de canais e conversas em janelas largas."
		}
		"Show People in wide windows" => "Mostrar Pessoas em janelas largas",
		"Keep the member list open whenever the window is wide enough." => {
			"Mantém a lista de membros aberta quando a janela é larga o suficiente."
		}
		"Messages and media" => "Mensagens e mídia",
		"Reset chat" => "Redefinir conversas",
		"Animate GIFs" => "Animar GIFs",
		"Visible chat GIFs play automatically." => {
			"Os GIFs visíveis no chat são reproduzidos automaticamente."
		}
		"Hide image and GIF links" => "Ocultar links de imagem e GIF",
		"Hide standalone links when their image or GIF preview is shown." => {
			"Oculta links soltos quando a prévia da imagem ou do GIF aparece."
		}
		"Links" => "Links",
		"Confirm before opening links" => "Confirmar antes de abrir links",
		"Ask before opening external links. Discord links always open directly." => {
			"Pergunta antes de abrir links externos. Links do Discord sempre abrem direto."
		}
		"Scrolling" => "Rolagem",
		"Smooth scrolling" => "Rolagem suave",
		"Animate wheel movement and jumps between messages." => {
			"Anima o movimento da roda e os saltos entre mensagens."
		}
		"Scrolling speed" => "Velocidade da rolagem",
		"Mouse wheel and trackpad movement. 100% is the default." => {
			"Movimento da roda e do trackpad. 100% é o padrão."
		}
		"Retry saving reading settings" => "Tentar salvar as preferências de leitura de novo",
		"Overview" => "Visão geral",
		"Sounds" => "Sons",
		"Badges" => "Emblemas",
		"Enable Desktop Notifications" => "Ativar notificações na área de trabalho",
		"For per-channel or per-server notifications, right-click the channel or server and select Notification Settings." => {
			"Para notificações por canal ou servidor, clique com o botão direito no canal ou servidor e escolha Configurações de notificação."
		}
		"Sound Volume" => "Volume dos sons",
		"Adjusts the volume of all notification sounds and ringtones." => {
			"Ajusta o volume de todos os sons de notificação e toques."
		}
		"Disable All Notification Sounds" => "Desativar todos os sons de notificação",
		"Disables notification sounds. Your individual sound preferences are saved and restored when you turn this off." => {
			"Desativa os sons de notificação. Suas preferências individuais são salvas e voltam quando você desliga isto."
		}
		"New Message" => "Nova mensagem",
		"New Message in the channel I'm currently reading" => {
			"Nova mensagem no canal que estou lendo"
		}
		"Incoming Ring" => "Toque de entrada",
		"Outgoing Ring" => "Toque de saída",
		"Microphone Muted" => "Microfone silenciado",
		"Microphone Unmuted" => "Microfone ativado",
		"Camera On" => "Câmera ligada",
		"Screen Share Started" => "Compartilhamento de tela iniciado",
		"Call Joined" => "Entrou na chamada",
		"User Left Call" => "Saiu da chamada",
		"Preview Sound" => "Ouvir som",
		"Ringtones, call devices and microphone processing." => {
			"Toques, dispositivos da chamada e processamento do microfone."
		}
		"Open" => "Abrir",
		"Enable Unread Message Badge" => "Mostrar emblema de mensagens não lidas",
		"Shows a red badge on the app icon when you have unread messages." => {
			"Mostra um emblema vermelho no ícone do app quando há mensagens não lidas."
		}
		"App icon badges are not available on this platform yet." => {
			"Emblemas no ícone do app ainda não estão disponíveis nesta plataforma."
		}
		"Limit from your account" => "Limite da sua conta",
		"Limit from server boost" => "Limite do boost do servidor",
		"This file exceeds the upload limit here; compress it or share a link" => {
			"Este arquivo excede o limite de envio aqui; comprima ou envie um link"
		}
		"Choose, drop, or paste files (Ctrl/Cmd/Option+V). Up to 10 files; each file must fit your upload limit. Send starts the upload." => {
			"Escolha, solte ou cole arquivos (Ctrl/Cmd/Option+V). Até 10 arquivos; cada um deve caber no seu limite de envio. Enviar inicia o upload."
		}
		"+" => "+",
		"< Integrations" => "< Integrations",
		"Add custom emoji that anyone can use in this server. Animated GIF emoji may be used by members with Discord Nitro." => {
			"Adicione emoji personalizados que qualquer pessoa possa usar neste servidor. Membros com Discord Nitro podem usar emoji GIF animados."
		}
		"Add custom stickers for members to use in this server. Artwork is cropped and resized to 320 × 320 pixels before upload." => {
			"Adicione figurinhas personalizadas para os membros usarem neste servidor. A arte é recortada e redimensionada para 320 × 320 pixels antes do envio."
		}
		"All Actions" => "Todas as ações",
		"All Users" => "Todos os usuários",
		"Bots and Apps" => "Bots e apps",
		"Copy invite link" => "Copy invite link",
		"Create Invite Link" => "Criar link de convite",
		"Create Role" => "Criar cargo",
		"Create an invite link to welcome people to this server." => {
			"Crie um link de convite para receber pessoas neste servidor."
		}
		"Custom role color" => "Custom role color",
		"Default Permissions\n@everyone · applies to all server members" => {
			"Permissões padrão\n@everyone · se aplica a todos os membros do servidor"
		}
		"Default role color" => "Default role color",
		"Delete Emoji" => "Delete Emoji",
		"Delete Role" => "Excluir cargo",
		"Delete Sticker" => "Delete Sticker",
		"Description (optional)" => "Description (optional)",
		"Discard Changes" => "Descartar alterações",
		"Drag and drop up to 10 images onto this page, or choose files. Review their names before uploading." => {
			"Arraste e solte até 10 imagens nesta página ou escolha arquivos. Revise os nomes antes de enviar."
		}
		"Edit" => "Edit",
		"Emoji name" => "Emoji name",
		"Emoji name: 2–32 letters, numbers, or underscores" => {
			"Emoji name: 2–32 letters, numbers, or underscores"
		}
		"Filter by Action" => "Filtrar por ação",
		"Filter by User" => "Filtrar por usuário",
		"First page" => "Primeira página",
		"For example: 🐀" => "For example: 🐀",
		"Image" => "Image",
		"Inactive for" => "Inactive for",
		"Keep Editing" => "Continuar editando",
		"Kick Member" => "Kick Member",
		"Leave blank to use their username." => "Leave blank to use their username.",
		"Load More" => "Carregar mais",
		"Loading audit log…" => "Carregando registro de auditoria…",
		"MEMBERS" => "MEMBROS",
		"Manage >" => "Manage >",
		"Member details" => "Member details",
		"Members use the color of their highest role on the roles list." => {
			"Members use the color of their highest role on the roles list."
		}
		"NONE" => "NENHUM",
		"Next page" => "Próxima página",
		"No active invite links" => "Nenhum link de convite ativo",
		"No additional details were provided for this event." => {
			"Nenhum detalhe adicional foi fornecido para este evento."
		}
		"No audit log entries match these filters." => {
			"Nenhuma entrada do registro de auditoria corresponde a estes filtros."
		}
		"No custom stickers yet." => "Nenhuma figurinha personalizada ainda.",
		"No integrations in this server." => "Nenhuma integração neste servidor.",
		"No members match this search." => "No members match this search.",
		"Permissions" => "Permissões",
		"Posts from these followed channels are delivered to your server." => {
			"Posts from these followed channels are delivered to your server."
		}
		"Preparing emoji images..." => "Preparing emoji images...",
		"Preparing sticker artwork…" => "Preparing sticker artwork…",
		"Prune" => "Prune",
		"Prune Members" => "Prune Members",
		"Recent Members" => "Membros recentes",
		"Related emoji" => "Related emoji",
		"Reload" => "Recarregar",
		"Reload Invites" => "Recarregar convites",
		"Reload Roles" => "Recarregar cargos",
		"Reload integrations" => "Recarregar integrações",
		"Reload integrations before making more changes. Your draft will be kept." => {
			"Reload integrations before making more changes. Your draft will be kept."
		}
		"Remove Integration" => "Remove Integration",
		"Review sticker" => "Review sticker",
		"Review uploads" => "Review uploads",
		"Revoke Invite" => "Revogar convite",
		"Revoke invite" => "Revoke invite",
		"Role Style" => "Estilo do cargo",
		"Role color" => "Cor do cargo",
		"Role icon" => "Ícone do cargo",
		"Sample message" => "Mensagem de exemplo",
		"Search Roles" => "Buscar cargos",
		"Search by username or ID" => "Buscar por nome de usuário ou ID",
		"Search members" => "Buscar membros",
		"Search permissions" => "Buscar permissões",
		"Second gradient color" => "Second gradient color",
		"Send updates from your apps and services to a channel in this server." => {
			"Send updates from your apps and services to a channel in this server."
		}
		"Server Members" => "Membros do servidor",
		"Showing the first 50 integrations returned by Discord." => {
			"Showing the first 50 integrations returned by Discord."
		}
		"Static PNG, JPEG and WebP artwork is supported up to 8 MB. The prepared PNG must fit within Discord's 512 KB limit." => {
			"Arte PNG, JPEG e WebP estática é aceita até 8 MB. O PNG preparado deve caber no limite de 512 KB do Discord."
		}
		"The audit log reached its local entry or memory limit. Adjust the filters to find other events." => {
			"O registro de auditoria atingiu o limite local de entradas ou memória. Ajuste os filtros para encontrar outros eventos."
		}
		"This integration is no longer available." => "This integration is no longer available.",
		"This is how members with this role appear." => {
			"This is how members with this role appear."
		}
		"Unknown" => "Unknown",
		"Upload" => "Upload",
		"Upload Emoji" => "Enviar emoji",
		"Upload Sticker" => "Enviar figurinha",
		"Uploaded By" => "Uploaded By",
		"Use roles to group your server members and assign permissions." => {
			"Use cargos para agrupar membros do servidor e atribuir permissões."
		}
		"Use their username" => "Use their username",
		"Your stickers" => "Suas figurinhas",
		"←  BACK" => "←  VOLTAR",
		"Tools" => "Ferramentas",
		"Extension tool" => "Ferramenta de extensão",
		"Review the result. App actions and draft changes need your approval." => {
			"Revise o resultado. Ações do app e alterações no rascunho precisam da sua aprovação."
		}
		"Proposed composer text" => "Texto proposto para o compositor",
		"Proposed app action" => "Ação proposta do app",
		"Apply to Draft" => "Aplicar ao rascunho",
		"(edited)" => "(editado)",
		". See all " => ". Ver tudo ",
		"Application interaction pending…" => "Interação com o aplicativo pendente…",
		"Archived" => "Arquivado",
		"Archived posts need a connected session with history access." => {
			"Publicações arquivadas exigem uma sessão conectada com acesso ao histórico."
		}
		"Choose a conversation to see its people." => {
			"Escolha uma conversa para ver quem participa."
		}
		"Clear this draft" => "Limpar este rascunho",
		"Copy edit text" => "Copiar o texto editado",
		"Dismiss message" => "Dispensar a mensagem",
		"Display limited · Copy message for the full text" => {
			"Exibição limitada · Copie a mensagem para ler o texto todo"
		}
		"Draft budget full. Clear an existing draft to continue." => {
			"Limite de rascunhos cheio. Apague um rascunho existente para continuar."
		}
		"Enter a message..." => "Escreva uma mensagem...",
		"Hide spoilers" => "Ocultar spoilers",
		"Latest message unavailable" => "Última mensagem indisponível",
		"Load archived posts" => "Carregar publicações arquivadas",
		"Load more posts" => "Carregar mais publicações",
		"Loading archived posts…" => "Carregando publicações arquivadas…",
		"Loading posts…" => "Carregando publicações…",
		"Message deleted" => "Mensagem apagada",
		"No conversation selected" => "Nenhuma conversa selecionada",
		"No older archived posts reported." => "Nenhuma publicação arquivada antiga informada.",
		"No people returned for this view." => "Ninguém apareceu nesta visão.",
		"OFFLINE PREVIEW" => "PRÉVIA OFFLINE",
		"Older archived posts" => "Publicações arquivadas antigas",
		"Only you can see this  •" => "Só você vê isto  •",
		"Open this channel’s threads" => "Abrir os tópicos deste canal",
		"Pick a channel or direct message from the list." => {
			"Escolha um canal ou mensagem direta na lista."
		}
		"Posting…" => "Publicando…",
		"Profile and status" => "Perfil e status",
		"Remove Message" => "Remover a mensagem",
		"Replying to " => "Respondendo a ",
		"Retry shortcuts" => "Tentar os atalhos de novo",
		"Reveal spoiler media" => "Mostrar a mídia com spoiler",
		"Search loaded conversations (Ctrl/Cmd+K)" => "Buscar conversas carregadas (Ctrl/Cmd+K)",
		"Search or create a post..." => "Buscar ou criar uma publicação...",
		"Search this conversation" => "Buscar nesta conversa",
		"Synthetic data · no network or local storage" => {
			"Dados sintéticos · sem rede ou armazenamento local"
		}
		"This is the beginning of the conversation." => "Este é o início da conversa.",
		"Thread started from this message" => "Tópico criado a partir desta mensagem",
		"Title" => "Título",
		"Toggle Deleted Highlight" => "Alternar o destaque das apagadas",
		"View original" => "Ver o original",
		"View original message" => "Ver a mensagem original",
		"[Deleted message had no text]" => "[A mensagem apagada não tinha texto]",
		"used" => "usado",
		"· Save requested, check the connection before retrying" => {
			"· Salvamento pedido, confira a conexão antes de tentar de novo"
		}
		"↪ Forwarded" => "↪ Encaminhada",
		"Cancel download" => "Cancelar o download",
		"Choose files…" => "Escolher arquivos…",
		"Choose where to save this file · up to 100 MiB" => {
			"Escolha onde salvar este arquivo · até 100 MiB"
		}
		"Clear selection" => "Limpar a seleção",
		"Copy activity" => "Copiar a atividade",
		"Copy download link" => "Copiar o link de download",
		"Copy link" => "Copiar o link",
		"Copy webhook ID" => "Copiar o ID do webhook",
		"Dismiss" => "Dispensar",
		"Edit profile" => "Editar o perfil",
		"Loading profile…" => "Carregando o perfil…",
		"Mute this direct message's notifications until you unmute it." => {
			"Silencie as notificações desta mensagem direta até tirar o silenciamento."
		}
		"No matching options loaded" => "Nenhuma opção correspondente carregada",
		"Offline preview · synthetic" => "Prévia offline · sintética",
		"Open media" => "Abrir a mídia",
		"Open original…" => "Abrir o original…",
		"Preview" => "Prévia",
		"Refine your search to see more results" => "Aperte a busca para ver mais resultados",
		"Retry profile" => "Tentar o perfil de novo",
		"Reveal spoiler attachment" => "Mostrar o anexo com spoiler",
		"Reveal spoiler component" => "Mostrar o componente com spoiler",
		"Reveal spoiler media" => "Mostrar a mídia com spoiler",
		"Scroll to zoom · Drag to pan · Double-click to reset" => {
			"Role para ampliar · Arraste para mover · Duplo clique para restaurar"
		}
		"Search options" => "Opções de busca",
		"Show remaining roles" => "Mostrar os cargos restantes",
		"Submitting…" => "Enviando…",
		"This account was deleted. The conversation stays so you can read it." => {
			"Esta conta foi apagada. A conversa continua para você ler."
		}
		"Type to search members; available roles and channels are listed" => {
			"Digite para buscar membros; os cargos e canais disponíveis aparecem na lista"
		}
		"View banner" => "Ver o banner",
		"View profile picture" => "Ver a foto de perfil",
		_ => return None,
	})
}

fn spanish(key: &str) -> Option<&'static str> {
	Some(match key {
		"Voice Connected" => "Voz conectada",
		"Voice preview" => "Vista previa de voz",
		"Connecting…" => "Conectando…",
		"Call failed" => "La llamada falló",
		"A selected message could not be deleted and is back in the conversation" => {
			"Un mensaje seleccionado no se pudo borrar y volvió a la conversación"
		}
		"User settings" => "Ajustes de usuario",
		"App settings" => "Ajustes de la aplicación",
		"Customization" => "Personalización",
		"My Account" => "Mi cuenta",
		"Profile" => "Perfil",
		"Mention" => "Mencionar",
		"Add Note" => "Añadir nota",
		"Edit Friend Nickname" => "Editar apodo de amigo",
		"Add Friend Nickname" => "Añadir apodo de amigo",
		"Private nicknames are available for confirmed friends." => {
			"Los apodos privados están disponibles para amigos confirmados."
		}
		"Pin DM" => "Fijar MD",
		"Unpin DM" => "Desfijar MD",
		"Pinned direct messages are saved on this device." => {
			"Los mensajes directos fijados se guardan en este dispositivo."
		}
		"Mute Conversation" => "Silenciar conversación",
		"Unmute Conversation" => "Reactivar conversación",
		"Mute this direct message's notifications until you unmute it." => {
			"Silencia las notificaciones de esta conversación hasta que la reactives."
		}
		"Close DM" => "Cerrar MD",
		"Remove this conversation from your DM list. Messages are kept." => {
			"Quita esta conversación de tu lista. Los mensajes se conservan."
		}
		"No open direct message with this user." => {
			"Ninguna conversación abierta con este usuario."
		}
		"Block" => "Bloquear",
		"Unblock" => "Desbloquear",
		"Change Nickname" => "Cambiar apodo",
		"Nickname" => "Apodo",
		"Roles" => "Roles",
		"Kick" => "Expulsar",
		"Save" => "Guardar",
		"This removes the member from this server. They can rejoin with a new invite." => {
			"Esto elimina al miembro de este servidor. Puede volver con una nueva invitación."
		}
		"General" => "General",
		"Appearance" => "Apariencia",
		"Chat" => "Chat",
		"Messaging Permissions" => "Permisos de mensajería",
		"Notifications" => "Notificaciones",
		"Game Activity" => "Actividad de juegos",
		"Voice & Video" => "Voz y video",
		"Keybinds" => "Atajos de teclado",
		"Enable global shortcuts" => "Activar atajos globales",
		"Mute, deafen and push-to-talk stay off until you turn this on. They then work even when Nivra is in the background." => {
			"Silencio, ensordecer y pulsar para hablar siguen apagados hasta que actives esto. Después funcionan aunque Nivra esté en segundo plano."
		}
		"Data & Privacy" => "Datos y privacidad",
		"Updates" => "Actualizaciones",
		"Extensions" => "Extensiones",
		"Themes" => "Temas",
		"The Discord account signed in on this device." => {
			"La cuenta de Discord conectada en este dispositivo."
		}
		"Choose how you appear across Discord." => "Elige cómo apareces en Discord.",
		"Startup, window and graphics behavior on this device." => {
			"Inicio, ventana y gráficos en este dispositivo."
		}
		"Theme, colours, window effects and layout." => {
			"Tema, colores, efectos de ventana y diseño."
		}
		"How messages, media, links and scrolling behave." => {
			"Cómo se comportan los mensajes, medios, enlaces y desplazamiento."
		}
		"Control who can contact you and how messages are filtered." => {
			"Controla quién puede contactarte y cómo se filtran los mensajes."
		}
		"Choose which notifications you receive and how they appear." => {
			"Elige qué notificaciones recibes y cómo aparecen."
		}
		"Show others what you are playing." => "Muestra a los demás a qué estás jugando.",
		"Microphone, speakers, camera and voice processing." => {
			"Micrófono, altavoces, cámara y procesamiento de voz."
		}
		"Keyboard shortcuts for Nivra." => "Atajos de teclado de Nivra.",
		"What Nivra keeps on this device." => "Lo que Nivra guarda en este dispositivo.",
		"Keep Nivra up to date on this device." => "Mantén Nivra actualizado en este dispositivo.",
		"Manage community plugins." => "Administra plugins de la comunidad.",
		"Choose a community theme." => "Elige un tema de la comunidad.",
		"Language" => "Idioma",
		"App language" => "Idioma de la aplicación",
		"Changes apply immediately and are saved on this device." => {
			"Los cambios se aplican inmediatamente y se guardan en este dispositivo."
		}
		"Startup" => "Inicio",
		"Open Nivra when your computer starts" => "Abrir Nivra al iniciar el equipo",
		"Nivra signs in and connects in the background." => {
			"Nivra inicia sesión y se conecta en segundo plano."
		}
		"Start minimized" => "Iniciar minimizado",
		"Start in the background, out of your way." => {
			"Iniciar en segundo plano, sin ocupar la pantalla."
		}
		"Automatic startup is available on Windows and macOS." => {
			"El inicio automático está disponible en Windows y macOS."
		}
		"Window" => "Ventana",
		"Hide Nivra title bar" => "Ocultar la barra de título de Nivra",
		"Use the system title bar and window buttons instead." => {
			"Usa la barra de título y los botones de ventana del sistema."
		}
		"Keep Nivra in the menu bar" => "Mantener Nivra en la barra de menús",
		"Keep Nivra in the system tray" => "Mantener Nivra en la bandeja del sistema",
		"Nivra keeps running in the system tray" => "Nivra sigue en la bandeja",
		"The tray is unavailable on this platform." => {
			"La bandeja del sistema no está disponible en esta plataforma."
		}
		"Graphics" => "Gráficos",
		"Render with" => "Renderizar con",
		"Search" => "Buscar",
		"Close settings (Esc)" => "Cerrar ajustes (Esc)",
		"Unofficial · not endorsed by Discord" => "No oficial · no respaldado por Discord",
		"Exit preview" => "Salir de la vista previa",
		"Log out" => "Cerrar sesión",
		"Your account" => "Tu cuenta",
		"Offline preview · synthetic account" => "Vista previa sin conexión · cuenta sintética",
		"Signed in with your Discord account" => "Sesión iniciada con tu cuenta de Discord",
		"Display name" => "Nombre para mostrar",
		"Email, password and security" => "Correo, contraseña y seguridad",
		"Managed in Discord" => "Administrado en Discord",
		"Edit profile" => "Editar perfil",
		"Session" => "Sesión",
		"Closes the offline fixture. Nothing is stored for the preview." => {
			"Cierra la vista previa sin conexión. No se guarda nada para la vista previa."
		}
		"Removes the saved login and clears this account's local cache and drafts." => {
			"Elimina el inicio de sesión guardado y borra la caché local y los borradores de esta cuenta."
		}
		"Theme" => "Tema",
		"Accent" => "Color de acento",
		"Primary color" => "Color principal",
		"The active theme brings its own accent; it takes over while the theme is in use." => {
			"El tema activo trae su propio color de acento y se usa mientras el tema esté activo."
		}
		"Used for buttons, selection and message highlights." => {
			"Se usa en botones, selección y resaltados de mensajes."
		}
		"Reset" => "Restablecer",
		"Choose primary color" => "Elegir color principal",
		"Window effects" => "Efectos de ventana",
		"Transparency & blur" => "Transparencia y desenfoque",
		"Restart Nivra after changing this. Themes can customize effects while enabled." => {
			"Reinicia Nivra después de cambiar esto. Los temas pueden personalizar los efectos mientras estén activos."
		}
		"Transparency" => "Transparencia",
		"Blur" => "Desenfoque",
		"Zero disables blur; the native compositor controls its exact strength." => {
			"Cero desactiva el desenfoque; el compositor nativo controla su intensidad exacta."
		}
		"Apply to all surfaces" => "Aplicar a todas las superficies",
		"Include sidebars, server rail, headers, and composer." => {
			"Incluye barras laterales, barra de servidores, encabezados y compositor."
		}
		"Channel list" => "Lista de canales",
		"Show hidden channels" => "Mostrar canales ocultos",
		"Show channels you cannot currently access." => {
			"Muestra canales a los que no puedes acceder actualmente."
		}
		"Colour preset" => "Preajuste de color",
		"Share game activity" => "Compartir actividad de juego",
		"Detect running games and ask Discord to share them as activity." => {
			"Detecta juegos en ejecución y pide a Discord que los comparta como actividad."
		}
		"Enable on Discord" => "Activar en Discord",
		"Check again" => "Comprobar de nuevo",
		"Looking for a running game" => "Buscando un juego en ejecución",
		"Activity sharing is off" => "El uso compartido de actividad está desactivado",
		"Synthetic activity, never shared or saved." => {
			"Actividad sintética, nunca compartida ni guardada."
		}
		"Local storage" => "Almacenamiento local",
		"Clear cache" => "Borrar caché",
		"Removes cached messages and media. Drafts and your login stay." => {
			"Elimina mensajes y medios en caché. Los borradores y tu inicio de sesión se conservan."
		}
		"Messages and drafts are cached on this device inside bounded, account-isolated files. Cache data is not encrypted by Nivra; saved login tokens use the OS credential store." => {
			"Los mensajes y borradores se guardan en caché en este dispositivo, en archivos limitados y aislados por cuenta. Nivra no cifra los datos de caché; los tokens de inicio de sesión guardados usan el almacén de credenciales del sistema."
		}
		"Your privacy" => "Tu privacidad",
		"Watch" => "Ver",
		"Open on YouTube" => "Abrir en YouTube",
		"Open on Vimeo" => "Abrir en Vimeo",
		"Play" => "Reproducir",
		"Pause" => "Pausar",
		"Resume" => "Continuar",
		"Replay" => "Ver de nuevo",
		"Retry" => "Reintentar",
		"Cancel" => "Cancelar",
		"Fullscreen" => "Pantalla completa",
		"Exit fullscreen (Esc)" => "Salir de pantalla completa (Esc)",
		"Seek video" => "Mover en el video",
		"Download video" => "Descargar video",
		"Save this video to your computer" => "Guardar este video en tu computadora",
		"Video attachment unavailable" => "El archivo adjunto de video no está disponible",
		"This file is not a video" => "Este archivo no es un video",
		"Unsupported embed video provider or URL" => "Este video no se puede reproducir aquí",
		"Video preview limit: 100 MiB" => "Límite de vista previa de video: 100 MiB",
		"Video server does not support buffering; download to play externally" => {
			"El servidor de video no permite reproducirlo aquí; descarga el archivo para verlo"
		}
		"Video download failed or changed; reload the conversation" => {
			"La descarga del video falló o cambió; recarga la conversación"
		}
		"Video link expired; reload the conversation" => {
			"El enlace del video expiró; recarga la conversación"
		}
		"Video worker stopped; restart Nivra" => {
			"El reproductor de video se detuvo; reinicia Nivra"
		}
		"Could not start video worker" => "No se pudo iniciar el reproductor de video",
		"Video audio output stopped" => "La salida de audio del video se detuvo",
		"Unsupported video audio timing" => "El audio de este video no se puede sincronizar",
		"Video buffering stalled; retry or download to play externally" => {
			"El video se atascó al cargar; reintenta o descarga el archivo"
		}
		"This video format or codec is not supported on this system." => {
			"Este formato o códec de video no funciona en esta computadora."
		}
		"The video could not be decoded safely." => {
			"Este video no se pudo decodificar de forma segura."
		}
		"Inline playback supports videos up to 1080p." => {
			"La reproducción aquí funciona hasta 1080p."
		}
		"Videos longer than two hours are not supported." => {
			"Los videos de más de dos horas no son compatibles."
		}
		"This video cannot seek to that position." => "Este video no puede saltar a esa posición.",
		"This video format or codec is not supported by Windows." => {
			"Windows no admite este formato o códec de video."
		}
		"This video format or codec is not supported by macOS." => {
			"macOS no admite este formato o códec de video."
		}
		"This video format or codec is not supported by GStreamer." => {
			"GStreamer no admite este formato o códec de video."
		}
		"Nivra does not collect telemetry or upload diagnostics. Discord retains service-side data according to its own policies." => {
			"Nivra no recopila telemetría ni envía diagnósticos. Discord conserva los datos del servicio según sus propias políticas."
		}
		"Offline preview · changes stay in this session and are never sent." => {
			"Vista previa sin conexión · los cambios permanecen en esta sesión y nunca se envían."
		}
		"Closing the window keeps Nivra in the menu bar. Quit from its menu to exit." => {
			"Cerrar la ventana mantiene Nivra en la barra de menús. Sal desde su menú para terminar."
		}
		"Closing keeps Nivra running. Use the tray to show, minimize or quit." => {
			"Cerrar mantiene Nivra en ejecución. Usa la bandeja para mostrar, minimizar o salir."
		}
		"Closing the window keeps Nivra in the notification area. Quit from its menu to exit." => {
			"Cerrar la ventana mantiene Nivra en el área de notificación. Sal desde su menú para terminar."
		}
		"Takes effect the next time Nivra starts." => {
			"Se aplica la próxima vez que se inicie Nivra."
		}
		"Online" => "En línea",
		"Idle" => "Ausente",
		"Do Not Disturb" => "No molestar",
		"Invisible" => "Invisible",
		"Don't clear" => "No borrar",
		"30 minutes" => "30 minutos",
		"1 hour" => "1 hora",
		"4 hours" => "4 horas",
		"Today" => "Hoy",
		"Switch to" => "Cambiar a",
		"Forget" => "Olvidar",
		"Nivra clears it" => "Nivra borra el estado",
		"You" => "Tú",
		"Custom status" => "Estado personalizado",
		"Shown next to your name across Discord." => "Se muestra junto a tu nombre en Discord.",
		"Switch accounts" => "Cambiar de cuenta",
		"Add an account" => "Añadir una cuenta",
		"Forget this account on this device" => "Olvidar esta cuenta en este dispositivo",
		"Loading profile…" => "Cargando perfil…",
		"Reload profile" => "Recargar perfil",
		"You will not receive desktop notifications" => "No recibirás notificaciones de escritorio",
		"You will appear offline" => "Aparecerás sin conexión",
		"Edit custom status" => "Editar estado personalizado",
		"Set a custom status" => "Establecer un estado personalizado",
		"No custom status" => "Sin estado personalizado",
		"Status text" => "Texto del estado",
		"What's on your mind?" => "¿Qué tienes en mente?",
		"Clear after" => "Borrar después",
		"Use up to 128 characters without control characters." => {
			"Usa hasta 128 caracteres sin caracteres de control."
		}
		"Clear" => "Borrar",
		"Apply" => "Aplicar",
		"No settings found" => "No se encontraron ajustes",
		"Try theme, notifications, voice, or cache." => "Prueba tema, notificaciones, voz o caché.",
		"Use a different account" => "Usar otra cuenta",
		"Waiting for Discord…" => "Esperando a Discord…",
		"Use another account" => "Usar otra cuenta",
		"Continue with Discord" => "Continuar con Discord",
		"Early preview" => "Vista previa",
		"Checking your saved login" => "Revisando tu inicio de sesión guardado",
		"Connecting to Discord" => "Conectando a Discord",
		"Sign in to Discord" => "Iniciar sesión en Discord",
		"discord.com · temporary login window · passwords and 2FA never leave the page" => {
			"discord.com · ventana temporal de inicio de sesión · la contraseña y el 2FA nunca salen de la página"
		}
		"Sign in with a session token" => "Iniciar sesión con un token de sesión",
		"For owners who already hold a valid Discord session token, for example from another signed-in Nivra install. Passwords and 2FA are never used here; this bypasses Discord's hosted login page entirely." => {
			"Para quien ya tiene un token de sesión válido de Discord, por ejemplo de otra instalación de Nivra con la sesión iniciada. Aquí nunca se usa la contraseña ni el 2FA; esto evita por completo la página de inicio de sesión de Discord."
		}
		"Session token" => "Token de sesión",
		"Connect with this token" => "Conectar con este token",
		"About Nivra" => "Acerca de Nivra",
		"Forget saved login" => "Olvidar el inicio de sesión guardado",
		"Messaging, reactions, search and read markers have offline tests. Real Discord interoperability is still unverified; attachment uploads and advanced search remain incomplete." => {
			"Los mensajes, las reacciones, la búsqueda y las marcas de lectura tienen pruebas sin conexión. La interoperabilidad real con Discord todavía no está verificada; el envío de archivos adjuntos y la búsqueda avanzada siguen incompletos."
		}
		"Messages and drafts are cached locally. Login tokens use the operating system credential store." => {
			"Los mensajes y borradores se guardan en caché local. Los tokens de inicio de sesión usan el almacén de credenciales del sistema."
		}
		"Unofficial clients may put your Discord account at risk." => {
			"Los clientes no oficiales pueden poner tu cuenta de Discord en riesgo."
		}
		"Explore the offline preview" => "Explorar la vista previa sin conexión",
		"Sample conversations. No Discord connection." => {
			"Conversaciones de ejemplo. Sin conexión con Discord."
		}
		"or" => "o",
		"Sign in again" => "Iniciar sesión de nuevo",
		"Sign in before calling" => "Inicia sesión antes de llamar",
		"Sign in before changing your profile picture" => {
			"Inicia sesión antes de cambiar tu foto de perfil"
		}
		"Sign in through Discord; saved-login lookup stopped" => {
			"Iniciando sesión con Discord; se detuvo la búsqueda de la sesión guardada"
		}
		"Waiting for Discord login" => "Esperando el inicio de sesión de Discord",
		"Platform login webview unavailable; see platform-support.md" => {
			"La ventana de inicio de sesión no abrió; mira platform-support.md"
		}
		"Platform login webview unavailable" => "La ventana de inicio de sesión no abrió",
		"Interface language" => "Idioma de la interfaz",
		"Loading discord.com…" => "Abriendo discord.com…",
		"Welcome back" => "Bienvenido de nuevo",
		"Welcome to Nivra" => "Bienvenido a Nivra",
		"Continue with a saved account, or sign in with another one." => {
			"Continúa con una cuenta guardada o entra con otra."
		}
		"Sign in with Discord." => "Entra con Discord.",
		"Saved accounts" => "Cuentas guardadas",
		"This is my account" => "Esta es mi cuenta",
		"Check this to continue." => "Marca esto para continuar.",
		"Independent and open source. Not affiliated with Discord." => {
			"Independiente y de código abierto. No afiliado a Discord."
		}
		"Message" => "Mensaje",
		"Unread messages" => "Mensajes no leídos",
		"Mark as read" => "Marcar como leído",
		"Jump to unread" => "Ir a no leídos",
		"Copy" => "Copiar",
		"Copy message" => "Copiar mensaje",
		"Copy download link" => "Copiar enlace de descarga",
		"Reply" => "Responder",
		"Forward" => "Reenviar",
		"Forward message" => "Reenviar mensaje",
		"Create Thread…" => "Crear hilo…",
		"View reactions" => "Ver reacciones",
		"Mark read through here" => "Marcar como leído hasta aquí",
		"Mark Unread" => "Marcar como no leído",
		"Unpin message" => "Desfijar mensaje",
		"Pin message" => "Fijar mensaje",
		"Edit message" => "Editar mensaje",
		"Enter to save · Shift+Enter for a new line · Esc to cancel" => {
			"Enter guarda · Mayús+Enter salta de línea · Esc cancela"
		}
		"Remove from delete selection" => "Quitar de la selección",
		"Select for batch delete" => "Seleccionar para borrar",
		"You can select up to 5 messages at a time." => {
			"Puedes seleccionar hasta 5 mensajes a la vez."
		}
		"Select" => "Seleccionar",
		"selected" => "seleccionadas",
		"Shift+click selects a range · drag paints · Esc exits" => {
			"Mayús+clic selecciona un intervalo · arrastrar pinta · Esc sale"
		}
		"Esc stops the rest" => "Esc detiene el resto",
		"Stop" => "Detener",
		"Delete" => "Borrar",
		"Download" => "Descargar",
		"Save .txt" => "Guardar .txt",
		"Save .md" => "Guardar .md",
		"Export chat" => "Exportar conversación",
		"Exporting chat" => "Exportando conversación",
		"Export cancelled" => "Exportación cancelada",
		"Select all visible" => "Seleccionar visibles",
		"Select messages to enable actions" => "Selecciona mensajes para activar las acciones",
		"None of the selected messages can be deleted" => {
			"Ninguno de los mensajes seleccionados se puede borrar"
		}
		"Maximum 5 messages per delete" => "Máximo 5 mensajes por borrado",
		"No attachments in the selection" => "Sin adjuntos en la selección",
		"Maximum 15 attachments per download" => "Máximo 15 adjuntos por descarga",
		"You can delete up to 5 at a time" => "Se pueden borrar hasta 5 a la vez",
		"Only your messages can be deleted here" => "Solo tus mensajes se pueden borrar aquí",
		"You can download up to 15 attachments at a time" => {
			"Se pueden descargar hasta 15 adjuntos a la vez"
		}
		"Download attachments" => "Descargar adjuntos",
		"Copy text" => "Copiar texto",
		"Downloads" => "Descargas",
		"Choose a folder…" => "Elige una carpeta…",
		"Queued" => "En cola",
		"Downloading" => "Descargando",
		"Done" => "Listo",
		"Failed" => "Falló",
		"Cancelled" => "Cancelado",
		"Open folder" => "Abrir carpeta",
		"Selection copied" => "Selección copiada",
		"Selection saved" => "Selección guardada",
		"Download complete" => "Descarga completa",
		"Download cancelled" => "Descarga cancelada",
		"Delete message…" => "Borrar mensaje…",
		"Delete message immediately" => "Borrar mensaje ahora",
		"Message history is unavailable with current permission information." => {
			"El historial no está disponible con los permisos actuales."
		}
		"Mute" => "Silenciar",
		"Unmute" => "Activar micrófono",
		"Deafen" => "Ensordecer",
		"Undeafen" => "Activar audio",
		"Disconnect" => "Desconectar",
		"Dismiss call" => "Cerrar llamada",
		"Reconnect to call" => "Reconectar a la llamada",
		"Recent call" => "Llamada reciente",
		"You were in this call recently" => "Estuviste en esta llamada hace poco",
		"Dismiss" => "Descartar",
		"Reconnecting" => "Reconectando",
		"attempt" => "intento",
		"Reconnect now" => "Reconectar ahora",
		"Connected" => "Conectado",
		"Reconnecting…" => "Reconectando…",
		"No connection" => "Sin conexión",
		"Ping" => "Ping",
		"Connected for" => "Conectado durante",
		"Connection recovery" => "Recuperación de conexión",
		"Rejoin calls after brief disconnects" => "Reunirse a la llamada tras cortes breves",
		"Automatically returns to the same call when Discord reconnects within 15 seconds." => {
			"Vuelve automáticamente a la misma llamada cuando Discord se reconecta en 15 segundos."
		}
		"This can put you back on voice without an extra tap after short outages." => {
			"Puede devolverte a voz sin un toque extra tras cortes breves."
		}
		"Only enable this if you are comfortable rejoining voice automatically on this device." => {
			"Actívalo solo si aceptas volver a voz automáticamente en este dispositivo."
		}
		"Continue" => "Continuar",
		"Enable auto-rejoin" => "Activar reingreso automático",
		"Share your screen" => "Compartir pantalla",
		"Stop sharing" => "Dejar de compartir",
		"Turn on camera" => "Activar cámara",
		"Turn off camera" => "Desactivar cámara",
		"Turn on microphone" => "Activar micrófono",
		"Turn off microphone" => "Desactivar micrófono",
		"Turn on incoming audio" => "Escuchar la llamada",
		"Turn off incoming audio" => "Dejar de escuchar la llamada",
		"Speaking is unavailable in this channel." => "No se puede hablar en este canal.",
		"Voice settings" => "Ajustes de voz",
		"Microphone and speaker settings" => "Micrófono y altavoces",
		"Noise suppression" => "Supresión de ruido",
		"Removes background noise from your microphone before anyone else hears it." => {
			"Quita el ruido de fondo de tu micrófono antes de que los demás lo escuchen."
		}
		"Bot" => "Bot",
		"Screens" => "Pantallas",
		"Apps" => "Apps",
		"Options" => "Opciones",
		"None open" => "Ninguna abierta",
		"open" => "abiertas",
		"Resolution" => "Resolución",
		"Frame rate" => "Fotogramas/s",
		"Choose a screen or window" => "Elige una pantalla o ventana",
		"Looking for screens and windows…" => "Buscando pantallas y ventanas…",
		"Offline preview · no screen is captured" => {
			"Vista previa sin conexión · no se captura ninguna pantalla"
		}
		"In a call" => "En llamada",
		"In a call · microphone muted" => "En llamada · micrófono silenciado",
		"In a call · deafened" => "En llamada · audio desactivado",
		"unread mentions" => "menciones sin leer",
		"User volume" => "Volumen de la persona",
		"Reset volume" => "Restablecer volumen",
		"Silent" => "Sin sonido",
		"Normal" => "Normal",
		"Louder than normal" => "Más alto de lo normal",
		"5% quieter" => "5% más bajo",
		"5% louder" => "5% más alto",
		"Bots start at 50% to protect your hearing. You can still raise it here." => {
			"Los bots empiezan al 50% para proteger tu audición. Aún puedes subirlo aquí."
		}
		"Start bots at 50% volume" => "Los bots empiezan al 50% de volumen",
		"Protects your hearing from bots that join very loud. Right-click a bot in the call to change its volume." => {
			"Protege tu audición de bots que entran muy fuertes. Haz clic derecho en un bot de la llamada para cambiar su volumen."
		}
		"Light" => "Ligera",
		"Maximum" => "Máxima",
		"Recommended" => "Recomendado",
		"All voice settings" => "Todos los ajustes de voz",
		"No filter. For studio microphones or when playing music." => {
			"Sin filtro. Para micrófonos de estudio o cuando pones música."
		}
		"Steady hum like fans or air conditioning. Lightest on your PC." => {
			"Zumbido constante, como ventilador o aire acondicionado. Lo más ligero para el PC."
		}
		"Keyboard, clicks and everyday home noise. Works well for most people." => {
			"Teclado, clics y ruidos cotidianos de casa. Funciona bien para la mayoría."
		}
		"Very noisy home, or friends complain about your background noise. Uses more of your PC." => {
			"Casa muy ruidosa o amigos que se quejan de tu ruido. Usa más el PC."
		}
		"PC usage: none" => "Uso del PC: ninguno",
		"PC usage: very low" => "Uso del PC: muy bajo",
		"PC usage: low" => "Uso del PC: bajo",
		"PC usage: medium" => "Uso del PC: medio",
		"Friends complaining about noise? Choose Maximum. If your voice cuts out or your PC slows down, go back to Standard." => {
			"¿Tus amigos se quejan del ruido? Elige Máxima. Si tu voz se corta o el PC va lento, vuelve a Estándar."
		}
		"Your PC couldn't keep up with Maximum, so Nivra switched to Standard to keep your voice smooth." => {
			"Tu PC no pudo con Máxima, así que Nivra volvió a Estándar para que tu voz no se corte."
		}
		"The defaults work for most people. Change these only if something sounds wrong." => {
			"Los valores predeterminados sirven a la mayoría. Cámbialos solo si algo suena mal."
		}
		"Click to turn on or off · right-click to choose the level" => {
			"Haz clic para activar o desactivar · clic derecho para elegir el nivel"
		}
		"Noise suppression is unavailable in this build or preview." => {
			"La supresión de ruido no está disponible en esta versión o vista previa."
		}
		"Share a screen or window" => "Compartir una pantalla o ventana",
		"Stop sharing your screen" => "Dejar de compartir la pantalla",
		"Stop sharing your camera" => "Dejar de compartir la cámara",
		"Share your selected camera with this call" => {
			"Compartir la cámara elegida en esta llamada"
		}
		"Turn off noise suppression" => "Desactivar supresión de ruido",
		"Turn on noise suppression" => "Activar supresión de ruido",
		"Voice processing" => "Procesamiento de voz",
		"Voice processing & input mode" => "Procesamiento de voz y modo de entrada",
		"Mode" => "Modo",
		"Off" => "Desactivado",
		"Standard" => "Estándar",
		"Echo cancellation" => "Cancelación de eco",
		"Recommended when speakers can be picked up by your microphone." => {
			"Recomendado cuando el altavoz puede entrar por el micrófono."
		}
		"Automatic microphone volume" => "Volumen automático del micrófono",
		"Keeps speech at a more consistent loudness without changing your output volume." => {
			"Mantiene la voz en un volumen más estable, sin cambiar el volumen de salida."
		}
		"Push to talk" => "Pulsar para hablar",
		"When enabled, your microphone transmits only while the configured shortcut is held." => {
			"Con esto activado, el micrófono solo transmite mientras mantienes el atajo."
		}
		"Mute and deafen always take priority." => {
			"Silenciar el micrófono y el audio siempre tiene prioridad."
		}
		"Hold your configured shortcut when you want to speak." => {
			"Mantén el atajo configurado cuando quieras hablar."
		}
		"Deafen turns off incoming audio and mutes your microphone with it." => {
			"Ensordecer apaga el audio de la llamada y silencia el micrófono a la vez."
		}
		"Advanced input settings" => "Ajustes avanzados de entrada",
		"Voice activity threshold" => "Umbral de actividad de voz",
		"Only transmit sound above the threshold." => {
			"Solo transmite sonido por encima del umbral."
		}
		"Open voice activity; mute and push to talk still apply." => {
			"Micrófono abierto; silenciar y pulsar para hablar siguen aplicando."
		}
		"Input level" => "Nivel de entrada",
		"Light suppression strength" => "Intensidad del nivel Ligera",
		"Higher levels remove more noise but can affect natural voice detail." => {
			"Los niveles más altos quitan más ruido, pero pueden cambiar el detalle natural de la voz."
		}
		"Low" => "Bajo",
		"Moderate" => "Moderado",
		"High" => "Alto",
		"Very high" => "Muy alto",
		"Recommended defaults" => "Valores recomendados",
		"Raw microphone" => "Micrófono sin procesar",
		"Devices & levels" => "Dispositivos y volumen",
		"Input device" => "Dispositivo de entrada",
		"Output device" => "Dispositivo de salida",
		"Microphone gain" => "Ganancia del micrófono",
		"Speaker volume" => "Volumen del altavoz",
		"100% is the original level. Higher levels may distort." => {
			"100% es el nivel original. Por encima puede distorsionar."
		}
		"Rescan devices" => "Buscar dispositivos de nuevo",
		"Reset levels" => "Restablecer volumen",
		"System default follows your operating-system choice. Select a device only when you want Nivra to stay pinned to it." => {
			"El predeterminado del sistema sigue la elección del sistema operativo. Elige un dispositivo solo si quieres que Nivra se quede en él."
		}
		"System default (recommended)" => "Predeterminado del sistema (recomendado)",
		"Device unavailable — choose another" => "Dispositivo no disponible — elige otro",
		"Looking for audio devices..." => "Buscando dispositivos de audio...",
		"Looking for audio devices…" => "Buscando dispositivos de audio…",
		"Could not start audio device discovery" => {
			"No se pudieron buscar los dispositivos de audio"
		}
		"Audio devices loaded · headphones avoid microphone echo" => {
			"Dispositivos de audio cargados · los auriculares evitan el eco del micrófono"
		}
		"Audio device discovery stopped" => "La búsqueda de dispositivos de audio se detuvo",
		"One selected audio device is unavailable. Choose System default or rescan devices." => {
			"Un dispositivo de audio elegido no está disponible. Usa el predeterminado del sistema o busca de nuevo."
		}
		"Microphone unavailable · choose another input. You are still connected." => {
			"Micrófono no disponible · elige otra entrada. Sigues en la llamada."
		}
		"Microphone unavailable · still connected. Choose another input in Audio settings." => {
			"Micrófono no disponible · sigues en la llamada. Elige otra entrada en Audio."
		}
		"Camera" => "Cámara",
		"Voice privacy code" => "Código de privacidad de voz",
		"Call without end-to-end encryption" => "Llamada sin cifrado de extremo a extremo",
		"Compare with the other participants. This code changes with the encrypted call group." => {
			"Compáralo con los demás participantes. Este código cambia con el grupo cifrado de la llamada."
		}
		"Audio preferences are saved on this device. Your microphone starts only when you join a call or start testing." => {
			"Las preferencias de audio se guardan en este dispositivo. El micrófono solo se enciende al entrar en una llamada o al empezar una prueba."
		}
		"Install a voice-enabled build to use these controls." => {
			"Usa una versión con voz para estos controles."
		}
		"Friends" => "Amigos",
		"Add Friend" => "Añadir amigo",
		"You can add friends with their Discord username." => {
			"Puedes añadir amigos con su nombre de usuario de Discord."
		}
		"Username" => "Nombre de usuario",
		"Enter a username" => "Escribe un nombre de usuario",
		"Sending…" => "Enviando…",
		"Send Friend Request" => "Enviar solicitud de amistad",
		"Offline demo · actions are simulated." => {
			"Vista previa sin conexión · las acciones son simuladas."
		}
		"Reconnect before sending a friend request." => {
			"Reconéctate antes de enviar una solicitud de amistad."
		}
		"All" => "Todos",
		"Pending" => "Pendientes",
		"Blocked & Ignored" => "Bloqueados e ignorados",
		"All friends" => "Todos los amigos",
		"Blocked & ignored" => "Bloqueados e ignorados",
		"Blocked and ignored users are not available yet." => {
			"Los usuarios bloqueados e ignorados todavía no están disponibles."
		}
		"Friends are not available yet." => "Los amigos todavía no están disponibles.",
		"No blocked or ignored users match your search." => {
			"Ningún usuario bloqueado o ignorado coincide con la búsqueda."
		}
		"No friends match your search." => "Ningún amigo coincide con la búsqueda.",
		"No blocked or ignored users." => "No hay usuarios bloqueados o ignorados.",
		"No friends yet." => "Todavía no hay amigos.",
		"No friends are currently online." => "Ningún amigo está en línea ahora.",
		"Some friends’ online status and activity couldn’t be loaded. The Online list may be incomplete." => {
			"No se pudo cargar el estado y la actividad de algunos amigos. La lista En línea puede estar incompleta."
		}
		"Dismiss friend status warning" => "Descartar aviso de estado de amigos",
		"Direct Messages" => "Mensajes directos",
		"Spam Filters" => "Filtros de spam",
		"Friend Requests" => "Solicitudes de amistad",
		"Connected Games" => "Juegos conectados",
		"Direct Message (DM) Permissions" => "Permisos de mensaje directo (MD)",
		"Friend Request Permissions" => "Permisos de solicitud de amistad",
		"Messaging in Connected Games" => "Mensajería en juegos conectados",
		"Automatically filter suspected spam messages" => {
			"Filtrar automáticamente mensajes sospechosos de spam"
		}
		"Discord can filter out some messages that contain spam. These messages go to your Spam inbox." => {
			"Discord puede filtrar algunos mensajes con spam. Van a tu bandeja de spam."
		}
		"Filter all spam" => "Filtrar todo el spam",
		"Filter messages from non-friends" => "Filtrar mensajes de no amigos",
		"Don't filter spam" => "No filtrar spam",
		"Your account uses a custom spam filter setting. Select an option to replace it." => {
			"Tu cuenta usa un filtro de spam personalizado. Selecciona una opción para reemplazarlo."
		}
		"All servers" => "Todos los servidores",
		"Server" => "Servidor",
		"Some servers have different preferences. Choose a server to review its settings." => {
			"Algunos servidores tienen preferencias distintas. Elige un servidor para revisar sus ajustes."
		}
		"Changes apply to all current servers and set the default for newly joined servers." => {
			"Los cambios valen para los servidores actuales y quedan como ajuste de los nuevos."
		}
		"Changes apply to this server only." => "Los cambios valen solo para este servidor.",
		"Allow DMs from other server members" => "Permitir MD de otros miembros del servidor",
		"Filter messages from server members I may not know" => {
			"Filtrar mensajes de miembros que quizá no conozca"
		}
		"Move messages from people you may not know into Message Requests." => {
			"Mueve los mensajes de personas que quizá no conozcas a Solicitudes de mensaje."
		}
		"There are too many servers to update together. Choose an individual server." => {
			"Hay demasiados servidores para actualizar juntos. Elige uno."
		}
		"Saving…" => "Guardando…",
		"Loading your preferences…" => "Cargando tus preferencias…",
		"Try again" => "Reintentar",
		"Allow friend requests from" => "Permitir solicitudes de amistad de",
		"Control who can send you friend requests and how they appear." => {
			"Controla quién puede mandarte solicitudes de amistad y cómo aparecen."
		}
		"Everyone" => "Todos",
		"Friends of friends" => "Amigos de amigos",
		"Server members" => "Miembros del servidor",
		"Only from servers where you also allow Direct Messages." => {
			"Solo de servidores donde también permites mensajes directos."
		}
		"Show personalized messages" => "Mostrar mensajes personalizados",
		"Show personalized messages on incoming friend requests. If you accept, the message will still appear in your DMs." => {
			"Muestra mensajes personalizados en las solicitudes recibidas. Si aceptas, sigue en tus MD."
		}
		"Settings for games that use Discord to power their social experiences." => {
			"Ajustes de juegos que usan Discord en sus experiencias sociales."
		}
		"Allow friends from games to send direct messages and invites" => {
			"Permitir que amigos de juegos manden mensajes e invitaciones"
		}
		"Let friends from connected games send DMs and invite you to play, even when the game isn't open." => {
			"Deja que amigos de juegos conectados manden MD y te inviten a jugar, incluso con el juego cerrado."
		}
		"Show Direct Messages in games" => "Mostrar mensajes directos en los juegos",
		"Read and respond to DMs directly from in-game chats." => {
			"Lee y responde MD directamente desde los chats del juego."
		}
		"Show all DMs" => "Mostrar todos los MD",
		"Show only DMs from people who also play the game" => {
			"Mostrar solo MD de quienes también juegan"
		}
		"Don't show DMs" => "No mostrar MD",
		"Your account uses a custom in-game DM setting. Select an option to replace it." => {
			"Tu cuenta usa un ajuste personalizado de MD en el juego. Selecciona una opción para reemplazarlo."
		}
		"Mark As Read" => "Marcar como leído",
		"Server Settings" => "Ajustes del servidor",
		"Create invite" => "Crear invitación",
		"Leave server" => "Salir del servidor",
		"Leave server?" => "¿Salir del servidor?",
		"Are you sure you want to leave" => "¿Seguro que quieres salir de",
		"You will not be able to rejoin this server unless you are re-invited." => {
			"No podrás volver a este servidor sin una nueva invitación."
		}
		"Leaving…" => "Saliendo…",
		"Leave Server" => "Salir del servidor",
		"Close" => "Cerrar",
		"Showing the beginning of a large file." => "Mostrando el comienzo de un archivo grande.",
		"Show more" => "Mostrar más",
		"Offline preview · no server changes" => {
			"Vista previa sin conexión · sin cambios en el servidor"
		}
		"Remove From Favorites" => "Quitar de favoritos",
		"Add To Favorites" => "Añadir a favoritos",
		"Favorites are saved on this device." => "Los favoritos se guardan en este dispositivo.",
		"Invite to Channel" => "Invitar al canal",
		"Copy Link" => "Copiar enlace",
		"Unmute Channel" => "Reactivar canal",
		"Mute Channel" => "Silenciar canal",
		"For 15 Minutes" => "Durante 15 minutos",
		"For 1 Hour" => "Durante 1 hora",
		"For 3 Hours" => "Durante 3 horas",
		"For 8 Hours" => "Durante 8 horas",
		"For 24 Hours" => "Durante 24 horas",
		"Until I Turn It Back On" => "Hasta que yo lo reactive",
		"Copy Channel ID" => "Copiar ID del canal",
		"Hide Muted Channels" => "Ocultar canales silenciados",
		"Restart to update" => "Reiniciar para actualizar",
		"Updating…" => "Actualizando…",
		"Update available" => "Actualización disponible",
		"Dismiss update" => "Descartar actualización",
		"Download update" => "Descargar actualización",
		"Check for updates" => "Buscar actualizaciones",
		"Update checks are disabled in debug builds." => {
			"Las comprobaciones de actualización están desactivadas en compilaciones de depuración."
		}
		"Finish the current update before checking again." => {
			"Termina la actualización actual antes de volver a comprobar."
		}
		"Search themes" => "Buscar temas",
		"Search extensions" => "Buscar extensiones",
		"Checking for packages and updates" => "Buscando paquetes y actualizaciones",
		"Working on your last action" => "Procesando tu última acción",
		"Clear search" => "Limpiar búsqueda",
		"Create theme" => "Crear tema",
		"More" => "Más",
		"Import theme…" => "Importar tema…",
		"Refresh catalog" => "Actualizar catálogo",
		"Look for new packages and updates. Nothing installs on its own." => {
			"Busca paquetes y actualizaciones. Nada se instala solo."
		}
		"Import package…" => "Importar paquete…",
		"Open a package file from this computer." => "Abrir un archivo de paquete de este equipo.",
		"No matches" => "Sin resultados",
		"No themes yet" => "Aún no hay temas",
		"No extensions yet" => "Aún no hay extensiones",
		"Refresh the catalog or import a creator's package to get started." => {
			"Actualiza el catálogo o importa el paquete de un creador para empezar."
		}
		"Try a different name or creator." => "Prueba otro nombre o creador.",
		"Preview" => "Vista previa",
		"View preview" => "Ver vista previa",
		"Loading preview..." => "Cargando vista previa...",
		"Preview not loaded" => "Vista previa no cargada",
		"Preview unavailable" => "Vista previa no disponible",
		"Previewing theme" => "Viendo tema",
		"Theme preview" => "Vista previa del tema",
		"Changes are not saved yet" => "Cambios aún no guardados",
		"Back to themes" => "Volver a los temas",
		"Back to theme editor" => "Volver al editor",
		"Customize" => "Personalizar",
		"Use theme" => "Usar tema",
		"Apply this installed theme to the app." => "Aplica este tema instalado a la app.",
		"Edit theme" => "Editar tema",
		"Open tool" => "Abrir herramienta",
		"Disable" => "Desactivar",
		"Update" => "Actualizar",
		"Active" => "Activo",
		"Enabled" => "Activado",
		"Remove" => "Quitar",
		"by" => "por",
		"Plugin" => "Plugin",
		"Cleanup pending" => "Limpieza pendiente",
		"Retry cleanup" => "Reintentar limpieza",
		"Add a new tool to your conversations." => {
			"Añade una herramienta nueva a tus conversaciones."
		}
		"Install theme" => "Instalar tema",
		"Review & enable" => "Revisar y activar",
		"Review the new release before it replaces this version." => {
			"Revisa la nueva versión antes de que reemplace esta."
		}
		"Remove this theme and delete its local data." => {
			"Quita este tema y borra sus datos locales."
		}
		"Finish removing this extension and its local data." => {
			"Termina de quitar esta extensión y sus datos locales."
		}
		"Removes this extension and deletes its local data." => {
			"Quita esta extensión y borra sus datos locales."
		}
		"Selecting artwork sends it as an image attachment." => {
			"Elegir arte lo envía como adjunto de imagen."
		}
		"Example deleted-message appearance" => "Ejemplo de mensaje eliminado",
		"Creator preview" => "Vista previa del creador",
		"Close preview" => "Cerrar vista previa",
		"Reviewed" => "Revisado",
		"Unreviewed" => "Sin revisar",
		"View source" => "Ver código fuente",
		"Unreviewed package — its source has not been reviewed for the catalog." => {
			"Paquete sin revisar — su código fuente no se revisó para el catálogo."
		}
		"No access to conversations or composer text." => {
			"Sin acceso a conversaciones ni al texto del editor."
		}
		"Allow this extension to" => "Permitir que esta extensión",
		"Enable this theme" => "Activar este tema",
		"Enable this extension" => "Activar esta extensión",
		"Everything it may touch is listed below." => "Todo lo que puede tocar está listado abajo.",
		"Allow every listed permission to continue." => {
			"Permite todos los permisos listados para continuar."
		}
		"Enable explicit emoji and sticker image attachment selection" => {
			"Permitir elegir imágenes de emoji y stickers"
		}
		"Customize app colors, typography and control styling" => {
			"Personalizar colores, tipografía y controles de la app"
		}
		"Read live message events and text in the active conversation" => {
			"Leer eventos y texto de la conversación activa"
		}
		"Read the message I choose for an action" => "Leer el mensaje que yo elija",
		"Read my draft and propose text changes" => "Leer mi borrador y sugerir textos",
		"Store up to 1 MiB of local data for this account" => {
			"Guardar hasta 1 MiB de datos locales de esta cuenta"
		}
		"Read my account and current conversation details" => {
			"Leer mi cuenta y detalles de la conversación actual"
		}
		"Read my loaded profile, including biography and pronouns" => {
			"Leer mi perfil, incluyendo bio y pronombres"
		}
		"Read my loaded server names and identifiers" => {
			"Leer nombres e identificadores de mis servidores"
		}
		"Read current channel metadata, recipients and permissions" => {
			"Leer metadatos, participantes y permisos del canal"
		}
		"Receive changes to separately granted account and conversation data" => {
			"Recibir cambios de datos con permiso separado"
		}
		"Read loaded embed text, stickers and message reference metadata" => {
			"Leer embeds, stickers y referencias de mensajes"
		}
		"Read loaded forum and thread summaries" => "Leer resúmenes de foros e hilos",
		"Read current typing users and loaded pins; observe reactions" => {
			"Leer quién escribe, fijados y reacciones"
		}
		"Read loaded channel topics, categories, thread details and permissions" => {
			"Leer temas, categorías, hilos y permisos"
		}
		"Read loaded server members, roles and server profiles" => {
			"Leer miembros, roles y perfiles de los servidores"
		}
		"Read the list of loaded, readable conversations" => {
			"Leer la lista de conversaciones legibles"
		}
		"Read loaded message replies, mentions, attachment metadata and reactions" => {
			"Leer respuestas, menciones, adjuntos y reacciones"
		}
		"Read my loaded friends, requests, blocked and ignored users" => {
			"Leer amigos, solicitudes, bloqueados e ignorados"
		}
		"Read loaded messages in the active conversation" => {
			"Leer mensajes de la conversación activa"
		}
		"Read loaded members of the active conversation" => {
			"Leer miembros de la conversación activa"
		}
		"Read loaded user presence status" => "Leer estado de presencia",
		"Read current call state and participant identifiers" => {
			"Leer estado de la llamada y participantes"
		}
		"Read unread and mention counts in the active conversation" => {
			"Leer no leídos y menciones de la conversación activa"
		}
		"Enable" => "Activar",
		"PNG or JPEG, up to 2 MiB. This image does not change the chat background." => {
			"PNG o JPEG, hasta 2 MiB. No cambia el fondo del chat."
		}
		"Disabling removes the extension and its local data. Re-enabling starts fresh." => {
			"Desactivar quita la extensión y sus datos locales. Reactivar empieza de cero."
		}
		"Save Changes" => "Guardar cambios",
		"Back" => "Atrás",
		"Working…" => "Trabajando…",
		"Theme details" => "Detalles del tema",
		"How your theme appears in the gallery." => "Cómo aparece tu tema en la galería.",
		"Theme name" => "Nombre del tema",
		"My theme" => "Mi tema",
		"Theme name is required." => "El nombre del tema es obligatorio.",
		"Created by" => "Creado por",
		"Your name" => "Tu nombre",
		"Creator name is required." => "El nombre del creador es obligatorio.",
		"Card cover" => "Portada de la tarjeta",
		"Choose the image shown on your theme card in Themes." => {
			"Elige la imagen de la tarjeta del tema en Temas."
		}
		"App background" => "Fondo de la app",
		"Use one image behind your conversations and sidebars." => {
			"Usa una imagen detrás de las conversaciones y barras laterales."
		}
		"This older theme uses its original image placement." => {
			"Este tema antiguo usa la colocación original de la imagen."
		}
		"Use image across the app" => "Usar imagen en toda la app",
		"Preview in app" => "Vista previa en la app",
		"Save and apply" => "Guardar y aplicar",
		"Basics" => "Básico",
		"Background" => "Fondo",
		"Colors" => "Colores",
		"Advanced" => "Avanzado",
		"Discard unsaved theme?" => "¿Descartar tema sin guardar?",
		"Your changes have not been saved." => "Tus cambios no se han guardado.",
		"Discard changes" => "Descartar cambios",
		"Keep editing" => "Seguir editando",
		"Custom cover" => "Portada personalizada",
		"Automatic preview" => "Vista previa automática",
		"Replace cover" => "Cambiar portada",
		"Choose cover" => "Elegir portada",
		"Background image" => "Imagen de fondo",
		"No image selected" => "Ninguna imagen",
		"Replace image" => "Cambiar imagen",
		"Choose image" => "Elegir imagen",
		"Editing" => "Editando",
		"Dark" => "Oscuro",
		"Image opacity" => "Opacidad de la imagen",
		"Image fit" => "Ajuste de la imagen",
		"Fill area" => "Llenar área",
		"Fit entire image" => "Imagen completa",
		"Section opacity" => "Opacidad de la sección",
		"Select an area, then choose how much of the image shows through." => {
			"Elige un área y cuánto de la imagen se ve."
		}
		"Window gradient" => "Degradado de la ventana",
		"More colors" => "Más colores",
		"Surface opacity" => "Opacidad de la superficie",
		"0% shows the image. 100% is a solid section color." => {
			"0% muestra la imagen. 100% es color sólido."
		}
		"Selected section" => "Sección seleccionada",
		"Top bars" => "Barras superiores",
		"Server list" => "Lista de servidores",
		"People & channels" => "Personas y canales",
		"Message list" => "Lista de mensajes",
		"Member list" => "Lista de miembros",
		"Message input area" => "Área de escritura",
		"Window title and conversation header" => "Título de la ventana y encabezado",
		"The left server rail" => "La barra de servidores a la izquierda",
		"Direct messages and channel navigation" => "MD y navegación de canales",
		"The conversation timeline" => "La línea de la conversación",
		"The member and search pane on the right" => "El panel de miembros y búsqueda a la derecha",
		"The area around the message box" => "El área alrededor del cuadro de mensaje",
		"Window background" => "Fondo de la ventana",
		"Sidebar" => "Barra lateral",
		"Message area" => "Área de mensajes",
		"Cards & message input" => "Tarjetas y campo de mensaje",
		"Hover" => "Pasar el cursor",
		"Selection" => "Selección",
		"Borders" => "Bordes",
		"Headings" => "Títulos",
		"Body text" => "Texto del cuerpo",
		"Secondary text" => "Texto secundario",
		"Text on accent" => "Texto en el acento",
		"Success" => "Éxito",
		"Warning" => "Aviso",
		"Error & danger" => "Error y peligro",
		"Mention background" => "Fondo de mención",
		"Mention text" => "Texto de mención",
		"Buttons, selection and highlights" => "Botones, selección y resaltados",
		"Messages and regular labels" => "Mensajes y etiquetas comunes",
		"Timestamps and supporting text" => "Horas y textos de apoyo",
		"Channel, conversation and member lists" => "Listas de canales, conversaciones y miembros",
		"Background behind your messages" => "Fondo detrás de tus mensajes",
		"Use the default color for this appearance" => "Usar el color original de esta apariencia",
		"Use #RRGGBB or #RRGGBBAA." => "Usa #RRGGBB o #RRGGBBAA.",
		"Horizontal" => "Horizontal",
		"Vertical" => "Vertical",
		"Use the built-in value" => "Usar el valor original",
		"Text, spacing & corners" => "Texto, espaciado y esquinas",
		"These settings apply to dark and light appearances." => {
			"Estos ajustes valen para apariencia clara y oscura."
		}
		"Buttons" => "Botones",
		"Small text" => "Texto pequeño",
		"Code" => "Código",
		"Control height" => "Altura de los controles",
		"Item spacing" => "Espacio entre elementos",
		"Button padding" => "Relleno de los botones",
		"Control corners" => "Esquinas de los controles",
		"Window corners" => "Esquinas de la ventana",
		"Menu corners" => "Esquinas de los menús",
		"Sharing & export" => "Compartir y exportar",
		"The license and version are required. A source URL is optional for local themes." => {
			"La licencia y la versión son obligatorias. La URL de origen es opcional para temas locales."
		}
		"License" => "Licencia",
		"Version" => "Versión",
		"Source URL" => "URL de origen",
		"Optional" => "Opcional",
		"Use a valid HTTPS source URL or leave this blank." => {
			"Usa una URL de origen HTTPS válida o déjalo en blanco."
		}
		"License and version are required." => "La licencia y la versión son obligatorias.",
		"Only share images you own or have permission to use. Keep required attribution." => {
			"Solo comparte imágenes tuyas o con permiso. Mantén los créditos exigidos."
		}
		"Export theme" => "Exportar tema",
		"Add a theme name and creator name before saving." => {
			"Añade nombre del tema y del creador antes de guardar."
		}
		"Add a license and version before saving." => "Añade licencia y versión antes de guardar.",
		"Check the license, version, and optional source URL." => {
			"Revisa la licencia, la versión y la URL de origen opcional."
		}
		"Correct the highlighted color value." => "Corrige el color resaltado.",
		"Keep image and section opacity between 0% and 100%." => {
			"Mantén la opacidad de la imagen y las secciones entre 0% y 100%."
		}
		"Correct the highlighted gradient value." => "Corrige el degradado resaltado.",
		"Check the remaining theme settings before saving." => {
			"Revisa el resto de ajustes del tema antes de guardar."
		}
		"Reload server settings" => "Recargar ajustes",
		"Loading server settings…" => "Cargando ajustes…",
		"Reconnect to load server settings." => "Reconéctate para cargar los ajustes.",
		"Load server settings" => "Cargar ajustes",
		"Discard unsaved changes?" => "¿Descartar cambios sin guardar?",
		"Your changes to this server will be lost." => "Tus cambios en este servidor se perderán.",
		"Wait for the current save to finish before closing." => {
			"Espera a que termine el guardado actual antes de cerrar."
		}
		"Delete server" => "Eliminar servidor",
		"This action cannot be undone." => "Esta acción no se puede deshacer.",
		"Enter server name" => "Escribe el nombre del servidor",
		"Deleting…" => "Eliminando…",
		"Delete Server" => "Eliminar servidor",
		"Server Profile" => "Perfil del servidor",
		"Customize how your server appears in invite links and, if enabled, in Server Discovery and Announcement Channel messages." => {
			"Personaliza cómo aparece tu servidor en invitaciones y, si está activado, en Descubrimiento y mensajes de anuncios."
		}
		"Name" => "Nombre",
		"Icon" => "Icono",
		"We recommend an image of at least 512×512." => {
			"Recomendamos una imagen de al menos 512×512."
		}
		"Preparing icon…" => "Preparando icono…",
		"Change Server Icon" => "Cambiar icono del servidor",
		"Remove Icon" => "Quitar icono",
		"Banner" => "Banner",
		"Traits" => "Características",
		"Add up to 5 traits to show off your server's interests and personality." => {
			"Añade hasta 5 características para mostrar los intereses del servidor."
		}
		"Trait name" => "Nombre de la característica",
		"Remove trait" => "Quitar característica",
		"Description" => "Descripción",
		"How did your server get started? Why should people join?" => {
			"¿Cómo empezó tu servidor? ¿Por qué unirse?"
		}
		"Tell the world a bit about this server." => "Cuenta un poco sobre este servidor.",
		"Saving changes…" => "Guardando cambios…",
		"Careful — you have unsaved changes!" => "Cuidado — ¡hay cambios sin guardar!",
		"Use a server name of 2–100 characters, a description of up to 300 characters, and valid traits without control characters." => {
			"Usa un nombre de 2–100 caracteres, descripción de hasta 300 y características válidas."
		}
		"Could not save these changes. Check the selected channels, reconnect, or reload the server settings and try again." => {
			"No se pudo guardar. Revisa los canales, reconéctate o recarga e inténtalo de nuevo."
		}
		"Reload the server settings before saving again. Your edits will be kept." => {
			"Recarga los ajustes antes de guardar de nuevo. Tus ediciones se conservan."
		}
		"Reconnect to save changes." => "Reconéctate para guardar los cambios.",
		"MODERATION" => "MODERACIÓN",
		"APPS" => "APPS",
		"EXPRESSION" => "EXPRESIÓN",
		"PEOPLE" => "PERSONAS",
		"Engagement" => "Participación",
		"Manage settings that help keep your server active." => {
			"Ajustes que mantienen tu servidor activo."
		}
		"System Messages" => "Mensajes del sistema",
		"Configure system event messages sent to your server." => {
			"Configura los mensajes de eventos del sistema."
		}
		"Send a random welcome message when someone joins this server." => {
			"Enviar bienvenida aleatoria cuando alguien se una."
		}
		"Prompt members to reply to welcome messages with a sticker." => {
			"Pedir que respondan bienvenidas con un sticker."
		}
		"Send a message when someone boosts this server." => {
			"Enviar mensaje cuando alguien impulse el servidor."
		}
		"Send helpful tips for server setup." => "Enviar consejos de configuración.",
		"System Messages Channel" => "Canal de mensajes del sistema",
		"This is the channel we send system event messages to." => {
			"A este canal enviamos los mensajes del sistema."
		}
		"Activity Feed Settings" => "Feed de actividad",
		"Shows a feed of activity from games and connected apps in this server." => {
			"Muestra actividad de juegos y apps conectados."
		}
		"Display Activity Feed in this server" => "Mostrar feed de actividad aquí",
		"Server default" => "Ajuste del servidor",
		"Default Notification Settings" => "Notificaciones predeterminadas",
		"This will determine whether members who have not explicitly set their notification settings receive a notification for every message sent in this server or not." => {
			"Define si los miembros sin ajuste propio reciben aviso de cada mensaje."
		}
		"All Messages" => "Todos los mensajes",
		"Only @mentions" => "Solo @menciones",
		"We highly recommend setting this to only @mentions for a Community Server." => {
			"Recomendamos solo @menciones para un servidor de comunidad."
		}
		"Inactive Channel" => "Canal de inactivos",
		"Inactive Timeout" => "Tiempo de inactividad",
		"Automatically move members to this channel and mute them when they have been idle for longer than the inactive timeout. This does not affect browsers." => {
			"Mueve a los inactivos a este canal y los silencia. No afecta navegadores."
		}
		"Unavailable channel" => "Canal no disponible",
		"No Inactive Channel" => "Sin canal de inactivos",
		"No System Messages Channel" => "Sin canal del sistema",
		"None" => "Ninguno",
		"No accessible channels available." => "Ningún canal accesible.",
		"Stickers" => "Stickers",
		"Members" => "Miembros",
		"Invites" => "Invitaciones",
		"Integrations" => "Integraciones",
		"Audit Log" => "Registro de auditoría",
		"Navigation" => "Navegación",
		"Move around Nivra without reaching for the mouse." => {
			"Muévete por Nivra sin usar el ratón."
		}
		"Messages" => "Mensajes",
		"Composer shortcuts are only active while you are writing." => {
			"Los atajos del compositor solo funcionan mientras escribes."
		}
		"Text Formatting" => "Formato de texto",
		"Apply or remove formatting in the composer." => "Aplica o quita formato en el compositor.",
		"Global availability" => "Disponibilidad global",
		"Show Keyboard Shortcuts List" => "Mostrar lista de atajos",
		"Switch Conversation" => "Cambiar de conversación",
		"Close Settings or Dialog" => "Cerrar ajustes o diálogo",
		"Send Message" => "Enviar mensaje",
		"Insert New Line" => "Insertar nueva línea",
		"Edit Last Editable Message" => "Editar el último mensaje editable",
		"Bold" => "Negrita",
		"Italic" => "Cursiva",
		"Underline" => "Subrayado",
		"Strikethrough" => "Tachado",
		"Inline Code" => "Código en línea",
		"Code Block" => "Bloque de código",
		"Spoiler" => "Spoiler",
		"Push to Talk" => "Pulsar para hablar",
		"Toggle Mute" => "Alternar silencio",
		"Toggle Deafen" => "Alternar audio",
		"Voice" => "Voz",
		"Control your microphone and incoming audio during a connected call." => {
			"Controla el micrófono y el audio de la llamada mientras estás conectado."
		}
		"Already bound to" => "Ya usado por",
		"Brazil" => "Brasil",
		"United States" => "Estados Unidos",
		"Canada" => "Canadá",
		"United Kingdom" => "Reino Unido",
		"Germany" => "Alemania",
		"Netherlands" => "Países Bajos",
		"France" => "Francia",
		"Spain" => "España",
		"Poland" => "Polonia",
		"Finland" => "Finlandia",
		"Sweden" => "Suecia",
		"Singapore" => "Singapur",
		"Japan" => "Japón",
		"Hong Kong" => "Hong Kong",
		"Australia" => "Australia",
		"India" => "India",
		"South Africa" => "Sudáfrica",
		"Chile" => "Chile",
		"Argentina" => "Argentina",
		"South Korea" => "Corea del Sur",
		"Europe" => "Europa",
		"Russia" => "Rusia",
		"People in this call will see what you pick." => {
			"Las personas en esta llamada verán lo que elijas."
		}
		"Looking for your screens…" => "Buscando tus pantallas…",
		"Entire screen" => "Pantalla completa",
		"Share audio" => "Compartir audio",
		"Also send sound from other apps. Your microphone stays as it is." => {
			"También envía el sonido de otras apps. El micrófono sigue igual."
		}
		"Share an app" => "Compartir una app",
		"No apps are open to share." => "No hay apps abiertas para compartir.",
		"App" => "App",
		"Refresh" => "Actualizar",
		"Quality" => "Calidad",
		"Show cursor" => "Mostrar cursor",
		"Include the pointer in the shared video." => "Incluye el puntero en el video compartido.",
		"Share Screen" => "Compartir pantalla",
		"Before you use Nivra" => "Antes de usar Nivra",
		"Nivra is an unofficial app for your own Discord account: it is not Discord, it is not endorsed by Discord, and Discord's rules still apply to your account. It also includes other people's work (libraries, fonts and icons) under their own licenses, and accepting here does not waive those licenses or shift their copyright. Continuing confirms that you understand both points." => {
			"Nivra es una aplicación no oficial para tu propia cuenta de Discord: no es Discord, Discord no la respalda y las reglas de Discord siguen aplicando a tu cuenta. También incluye trabajo de otras personas (bibliotecas, fuentes e iconos) bajo sus propias licencias, y aceptar aquí no deja sin efecto esas licencias ni transfiere sus derechos de autor. Al continuar, confirmas que entiendes ambos puntos."
		}
		"Your app preferences could not be read, so Nivra cannot tell whether you accepted this before." => {
			"No se pudieron leer las preferencias de la aplicación, así que Nivra no puede saber si ya aceptaste esto antes."
		}
		"View full licenses" => "Ver licencias completas",
		"Hide full licenses" => "Ocultar licencias completas",
		"Hide offline members" => "Ocultar miembros sin conexión",
		"Show offline members" => "Mostrar miembros sin conexión",
		"Display" => "Pantalla",
		"GLOBAL" => "GLOBAL",
		"Send as text file?" => "¿Enviar como archivo de texto?",
		"Your message is too long for chat, so it will be sent as a file." => {
			"Tu mensaje es demasiado largo para el chat y se enviará como archivo."
		}
		"File name" => "Nombre del archivo",
		"Send" => "Enviar",
		"That file name will not work." => "Ese nombre de archivo no va a funcionar.",
		"Licenses" => "Licencias",
		"Legal" => "Información legal",
		"Licenses for the libraries, fonts, icons and sounds included in Nivra." => {
			"Licencias de las bibliotecas, fuentes, iconos y sonidos incluidos en Nivra."
		}
		"Filter licenses" => "Filtrar licencias",
		"No licenses match this filter." => "Ninguna licencia coincide con este filtro.",
		"All licenses" => "Todas las licencias",
		"Source code for the MPL-2.0 components in this version is published on its release page as" => {
			"El código fuente de los componentes bajo MPL-2.0 de esta versión se publica en su página de lanzamiento como"
		}
		"Notification sounds" => "Sonidos de notificación",
		"Fonts" => "Fuentes",
		"Emoji" => "Emojis",
		"Icons" => "Iconos",
		"Core libraries" => "Bibliotecas principales",
		"Sign-in" => "Inicio de sesión",
		"Audio playback" => "Reproducción de audio",
		"Other dependencies" => "Otras dependencias",
		"App preferences were not saved. If your acceptance of the terms was not recorded, Nivra will ask again next launch." => {
			"Las preferencias de la aplicación no se guardaron. Si tu aceptación de los términos no quedó registrada, Nivra volverá a preguntar la próxima vez que se abra."
		}
		"Your session expired; sign in again to continue." => {
			"Tu sesión expiró; inicia sesión de nuevo para continuar."
		}
		"I understand — continue" => "Entendido — continuar",
		"Zoom" => "Zoom",
		"Scales text and controls across the app." => {
			"Escala el texto y los controles en toda la aplicación."
		}
		"Layout" => "Diseño",
		"Reset layout" => "Restablecer diseño",
		"Sidebar width" => "Ancho de la barra lateral",
		"Channel and conversation list width in wide windows." => {
			"Ancho de la lista de canales y conversaciones en ventanas anchas."
		}
		"Show People in wide windows" => "Mostrar Personas en ventanas anchas",
		"Keep the member list open whenever the window is wide enough." => {
			"Mantiene la lista de miembros abierta cuando la ventana es lo bastante ancha."
		}
		"Messages and media" => "Mensajes y medios",
		"Reset chat" => "Restablecer chat",
		"Animate GIFs" => "Animar GIFs",
		"Visible chat GIFs play automatically." => {
			"Los GIFs visibles del chat se reproducen solos."
		}
		"Hide image and GIF links" => "Ocultar enlaces de imagen y GIF",
		"Hide standalone links when their image or GIF preview is shown." => {
			"Oculta enlaces sueltos cuando se muestra la vista previa de la imagen o el GIF."
		}
		"Links" => "Enlaces",
		"Confirm before opening links" => "Confirmar antes de abrir enlaces",
		"Ask before opening external links. Discord links always open directly." => {
			"Pregunta antes de abrir enlaces externos. Los enlaces de Discord siempre se abren directo."
		}
		"Scrolling" => "Desplazamiento",
		"Smooth scrolling" => "Desplazamiento suave",
		"Animate wheel movement and jumps between messages." => {
			"Anima el movimiento de la rueda y los saltos entre mensajes."
		}
		"Scrolling speed" => "Velocidad de desplazamiento",
		"Mouse wheel and trackpad movement. 100% is the default." => {
			"Movimiento de la rueda y del trackpad. 100% es el valor predeterminado."
		}
		"Retry saving reading settings" => "Reintentar guardar las preferencias de lectura",
		"Overview" => "Resumen",
		"Sounds" => "Sonidos",
		"Badges" => "Insignias",
		"Enable Desktop Notifications" => "Activar notificaciones de escritorio",
		"For per-channel or per-server notifications, right-click the channel or server and select Notification Settings." => {
			"Para notificaciones por canal o servidor, haz clic derecho en el canal o servidor y elige Ajustes de notificaciones."
		}
		"Sound Volume" => "Volumen de los sonidos",
		"Adjusts the volume of all notification sounds and ringtones." => {
			"Ajusta el volumen de todos los sonidos de notificación y tonos."
		}
		"Disable All Notification Sounds" => "Desactivar todos los sonidos de notificación",
		"Disables notification sounds. Your individual sound preferences are saved and restored when you turn this off." => {
			"Desactiva los sonidos de notificación. Tus preferencias individuales se guardan y vuelven cuando lo apagas."
		}
		"New Message" => "Mensaje nuevo",
		"New Message in the channel I'm currently reading" => {
			"Mensaje nuevo en el canal que estoy leyendo"
		}
		"Incoming Ring" => "Tono entrante",
		"Outgoing Ring" => "Tono saliente",
		"Microphone Muted" => "Micrófono silenciado",
		"Microphone Unmuted" => "Micrófono activado",
		"Camera On" => "Cámara activada",
		"Screen Share Started" => "Pantalla compartida iniciada",
		"Call Joined" => "Se unió a la llamada",
		"User Left Call" => "Salió de la llamada",
		"Preview Sound" => "Escuchar sonido",
		"Ringtones, call devices and microphone processing." => {
			"Tonos, dispositivos de la llamada y procesamiento del micrófono."
		}
		"Open" => "Abrir",
		"Enable Unread Message Badge" => "Mostrar insignia de mensajes no leídos",
		"Shows a red badge on the app icon when you have unread messages." => {
			"Muestra una insignia roja en el icono de la app cuando hay mensajes no leídos."
		}
		"App icon badges are not available on this platform yet." => {
			"Las insignias del icono de la app aún no están disponibles en esta plataforma."
		}
		"Limit from your account" => "Límite de tu cuenta",
		"Limit from server boost" => "Límite del boost del servidor",
		"This file exceeds the upload limit here; compress it or share a link" => {
			"Este archivo supera el límite de subida aquí; comprímelo o comparte un enlace"
		}
		"Choose, drop, or paste files (Ctrl/Cmd/Option+V). Up to 10 files; each file must fit your upload limit. Send starts the upload." => {
			"Elige, suelta o pega archivos (Ctrl/Cmd+Option+V). Hasta 10 archivos; cada uno debe caber en tu límite de subida. Enviar inicia la subida."
		}
		"+" => "+",
		"< Integrations" => "< Integrations",
		"Add custom emoji that anyone can use in this server. Animated GIF emoji may be used by members with Discord Nitro." => {
			"Add custom emoji that anyone can use in this server. Animated GIF emoji may be used by members with Discord Nitro."
		}
		"Add custom stickers for members to use in this server. Artwork is cropped and resized to 320 × 320 pixels before upload." => {
			"Add custom stickers for members to use in this server. Artwork is cropped and resized to 320 × 320 pixels before upload."
		}
		"All Actions" => "Todas las acciones",
		"All Users" => "Todos los usuarios",
		"Bots and Apps" => "Bots and Apps",
		"Copy invite link" => "Copy invite link",
		"Create Invite Link" => "Create Invite Link",
		"Create Role" => "Crear rol",
		"Create an invite link to welcome people to this server." => {
			"Create an invite link to welcome people to this server."
		}
		"Custom role color" => "Custom role color",
		"Default Permissions\n@everyone · applies to all server members" => {
			"Permisos predeterminados\n@everyone · se aplica a todos los miembros del servidor"
		}
		"Default role color" => "Default role color",
		"Delete Emoji" => "Delete Emoji",
		"Delete Role" => "Eliminar rol",
		"Delete Sticker" => "Delete Sticker",
		"Description (optional)" => "Description (optional)",
		"Discard Changes" => "Descartar cambios",
		"Drag and drop up to 10 images onto this page, or choose files. Review their names before uploading." => {
			"Drag and drop up to 10 images onto this page, or choose files. Review their names before uploading."
		}
		"Edit" => "Edit",
		"Emoji name" => "Emoji name",
		"Emoji name: 2–32 letters, numbers, or underscores" => {
			"Emoji name: 2–32 letters, numbers, or underscores"
		}
		"Filter by Action" => "Filtrar por acción",
		"Filter by User" => "Filtrar por usuario",
		"First page" => "First page",
		"For example: 🐀" => "For example: 🐀",
		"Image" => "Image",
		"Inactive for" => "Inactive for",
		"Keep Editing" => "Seguir editando",
		"Kick Member" => "Kick Member",
		"Leave blank to use their username." => "Leave blank to use their username.",
		"Load More" => "Load More",
		"Loading audit log…" => "Loading audit log…",
		"MEMBERS" => "MIEMBROS",
		"Manage >" => "Manage >",
		"Member details" => "Member details",
		"Members use the color of their highest role on the roles list." => {
			"Members use the color of their highest role on the roles list."
		}
		"NONE" => "NONE",
		"Next page" => "Next page",
		"No active invite links" => "No active invite links",
		"No additional details were provided for this event." => {
			"No additional details were provided for this event."
		}
		"No audit log entries match these filters." => "No audit log entries match these filters.",
		"No custom stickers yet." => "No custom stickers yet.",
		"No integrations in this server." => "No integrations in this server.",
		"No members match this search." => "No members match this search.",
		"Permissions" => "Permissions",
		"Posts from these followed channels are delivered to your server." => {
			"Posts from these followed channels are delivered to your server."
		}
		"Preparing emoji images..." => "Preparing emoji images...",
		"Preparing sticker artwork…" => "Preparing sticker artwork…",
		"Prune" => "Prune",
		"Prune Members" => "Prune Members",
		"Recent Members" => "Recent Members",
		"Related emoji" => "Related emoji",
		"Reload" => "Recargar",
		"Reload Invites" => "Recargar invitaciones",
		"Reload Roles" => "Recargar roles",
		"Reload integrations" => "Reload integrations",
		"Reload integrations before making more changes. Your draft will be kept." => {
			"Reload integrations before making more changes. Your draft will be kept."
		}
		"Remove Integration" => "Remove Integration",
		"Review sticker" => "Review sticker",
		"Review uploads" => "Review uploads",
		"Revoke Invite" => "Revocar invitación",
		"Revoke invite" => "Revoke invite",
		"Role Style" => "Role Style",
		"Role color" => "Role color",
		"Role icon" => "Role icon",
		"Sample message" => "Sample message",
		"Search Roles" => "Buscar roles",
		"Search by username or ID" => "Search by username or ID",
		"Search members" => "Search members",
		"Search permissions" => "Search permissions",
		"Second gradient color" => "Second gradient color",
		"Send updates from your apps and services to a channel in this server." => {
			"Send updates from your apps and services to a channel in this server."
		}
		"Server Members" => "Server Members",
		"Showing the first 50 integrations returned by Discord." => {
			"Showing the first 50 integrations returned by Discord."
		}
		"Static PNG, JPEG and WebP artwork is supported up to 8 MB. The prepared PNG must fit within Discord's 512 KB limit." => {
			"Static PNG, JPEG and WebP artwork is supported up to 8 MB. The prepared PNG must fit within Discord's 512 KB limit."
		}
		"The audit log reached its local entry or memory limit. Adjust the filters to find other events." => {
			"The audit log reached its local entry or memory limit. Adjust the filters to find other events."
		}
		"This integration is no longer available." => "This integration is no longer available.",
		"This is how members with this role appear." => {
			"This is how members with this role appear."
		}
		"Unknown" => "Unknown",
		"Upload" => "Upload",
		"Upload Emoji" => "Subir emoji",
		"Upload Sticker" => "Subir sticker",
		"Uploaded By" => "Uploaded By",
		"Use roles to group your server members and assign permissions." => {
			"Usa roles para agrupar miembros del servidor y asignar permisos."
		}
		"Use their username" => "Use their username",
		"Your stickers" => "Your stickers",
		"←  BACK" => "←  BACK",
		"Tools" => "Herramientas",
		"Extension tool" => "Herramienta de extensión",
		"Review the result. App actions and draft changes need your approval." => {
			"Revisa el resultado. Las acciones de la app y los cambios del borrador necesitan tu aprobación."
		}
		"Proposed composer text" => "Texto propuesto del compositor",
		"Proposed app action" => "Acción propuesta de la app",
		"Apply to Draft" => "Aplicar al borrador",
		"(edited)" => "(editado)",
		". See all " => ". Ver todo ",
		"Application interaction pending…" => "Interacción con la aplicación pendiente…",
		"Archived" => "Archivado",
		"Archived posts need a connected session with history access." => {
			"Las publicaciones archivadas necesitan una sesión conectada con acceso al historial."
		}
		"Choose a conversation to see its people." => {
			"Elige una conversación para ver quién participa."
		}
		"Clear this draft" => "Borrar este borrador",
		"Copy edit text" => "Copiar el texto editado",
		"Dismiss message" => "Descartar el mensaje",
		"Display limited · Copy message for the full text" => {
			"Vista limitada · Copia el mensaje para leer el texto completo"
		}
		"Draft budget full. Clear an existing draft to continue." => {
			"Límite de borradores lleno. Borra un borrador existente para continuar."
		}
		"Enter a message..." => "Escribe un mensaje...",
		"Hide spoilers" => "Ocultar spoilers",
		"Latest message unavailable" => "Último mensaje no disponible",
		"Load archived posts" => "Cargar publicaciones archivadas",
		"Load more posts" => "Cargar más publicaciones",
		"Loading archived posts…" => "Cargando publicaciones archivadas…",
		"Loading posts…" => "Cargando publicaciones…",
		"Message deleted" => "Mensaje eliminado",
		"No conversation selected" => "Ninguna conversación seleccionada",
		"No older archived posts reported." => {
			"No se encontraron publicaciones archivadas más antiguas."
		}
		"No people returned for this view." => "No se encontró nadie para esta vista.",
		"OFFLINE PREVIEW" => "VISTA SIN CONEXIÓN",
		"Older archived posts" => "Publicaciones archivadas antiguas",
		"Only you can see this  •" => "Solo tú ves esto  •",
		"Open this channel’s threads" => "Abrir los hilos de este canal",
		"Pick a channel or direct message from the list." => {
			"Elige un canal o mensaje directo de la lista."
		}
		"Posting…" => "Publicando…",
		"Profile and status" => "Perfil y estado",
		"Remove Message" => "Quitar el mensaje",
		"Replying to " => "Respondiendo a ",
		"Retry shortcuts" => "Reintentar los atajos",
		"Reveal spoiler media" => "Mostrar el archivo con spoiler",
		"Search loaded conversations (Ctrl/Cmd+K)" => "Buscar conversaciones cargadas (Ctrl/Cmd+K)",
		"Search or create a post..." => "Buscar o crear una publicación...",
		"Search this conversation" => "Buscar en esta conversación",
		"Synthetic data · no network or local storage" => {
			"Datos sintéticos · sin red ni almacenamiento local"
		}
		"This is the beginning of the conversation." => "Este es el comienzo de la conversación.",
		"Thread started from this message" => "Hilo creado a partir de este mensaje",
		"Title" => "Título",
		"Toggle Deleted Highlight" => "Alternar el resaltado de los eliminados",
		"View original" => "Ver el original",
		"View original message" => "Ver el mensaje original",
		"[Deleted message had no text]" => "[El mensaje eliminado no tenía texto]",
		"used" => "usado",
		"· Save requested, check the connection before retrying" => {
			"· Guardado solicitado, revisa la conexión antes de reintentar"
		}
		"↪ Forwarded" => "↪ Reenviado",
		"Cancel download" => "Cancelar la descarga",
		"Choose files…" => "Elegir archivos…",
		"Choose where to save this file · up to 100 MiB" => {
			"Elige dónde guardar este archivo · hasta 100 MiB"
		}
		"Clear selection" => "Borrar la selección",
		"Copy activity" => "Copiar la actividad",
		"Copy download link" => "Copiar el enlace de descarga",
		"Copy link" => "Copiar el enlace",
		"Copy webhook ID" => "Copiar el ID del webhook",
		"Dismiss" => "Descartar",
		"Edit profile" => "Editar el perfil",
		"Loading profile…" => "Cargando el perfil…",
		"Mute this direct message's notifications until you unmute it." => {
			"Silencia las notificaciones de este mensaje directo hasta que quites el silencio."
		}
		"No matching options loaded" => "Ninguna opción coincidente cargada",
		"Offline preview · synthetic" => "Vista sin conexión · sintética",
		"Open media" => "Abrir el archivo",
		"Open original…" => "Abrir el original…",
		"Preview" => "Vista previa",
		"Refine your search to see more results" => "Ajusta la búsqueda para ver más resultados",
		"Retry profile" => "Reintentar el perfil",
		"Reveal spoiler attachment" => "Mostrar el archivo adjunto con spoiler",
		"Reveal spoiler component" => "Mostrar el componente con spoiler",
		"Reveal spoiler media" => "Mostrar el archivo con spoiler",
		"Scroll to zoom · Drag to pan · Double-click to reset" => {
			"Desplaza para ampliar · Arrastra para mover · Doble clic para restablecer"
		}
		"Search options" => "Opciones de búsqueda",
		"Show remaining roles" => "Mostrar los roles restantes",
		"Submitting…" => "Enviando…",
		"This account was deleted. The conversation stays so you can read it." => {
			"Esta cuenta se eliminó. La conversación se conserva para que puedas leerla."
		}
		"Type to search members; available roles and channels are listed" => {
			"Escribe para buscar miembros; los roles y canales disponibles se listan"
		}
		"View banner" => "Ver el banner",
		"View profile picture" => "Ver la foto de perfil",
		_ => return None,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn migrated_ui_catalog_is_complete_for_both_locales() {
		const KEYS: &[&str] = &[
			"Online",
			"Idle",
			"Do Not Disturb",
			"Invisible",
			"Don't clear",
			"30 minutes",
			"1 hour",
			"4 hours",
			"Today",
			"Switch to",
			"Forget",
			"Nivra clears it",
			"You",
			"Custom status",
			"Shown next to your name across Discord.",
			"Switch accounts",
			"Add an account",
			"Forget this account on this device",
			"Loading profile…",
			"Reload profile",
			"You will not receive desktop notifications",
			"You will appear offline",
			"Edit custom status",
			"Set a custom status",
			"No custom status",
			"Status text",
			"What's on your mind?",
			"Clear after",
			"Use up to 128 characters without control characters.",
			"Clear",
			"Apply",
			"Search",
			"Close settings (Esc)",
			"Unofficial · not endorsed by Discord",
			"Exit preview",
			"Log out",
			"Your account",
			"Offline preview · synthetic account",
			"Signed in with your Discord account",
			"Display name",
			"Email, password and security",
			"Managed in Discord",
			"Edit profile",
			"Session",
			"Closes the offline fixture. Nothing is stored for the preview.",
			"Removes the saved login and clears this account's local cache and drafts.",
			"Theme",
			"Accent",
			"Primary color",
			"The active theme brings its own accent; it takes over while the theme is in use.",
			"Used for buttons, selection and message highlights.",
			"Reset",
			"Choose primary color",
			"Window effects",
			"Transparency & blur",
			"Restart Nivra after changing this. Themes can customize effects while enabled.",
			"Transparency",
			"Blur",
			"Zero disables blur; the native compositor controls its exact strength.",
			"Apply to all surfaces",
			"Include sidebars, server rail, headers, and composer.",
			"Channel list",
			"Show hidden channels",
			"Show channels you cannot currently access.",
			"Colour preset",
			"Share game activity",
			"Detect running games and ask Discord to share them as activity.",
			"Enable on Discord",
			"Check again",
			"Looking for a running game",
			"Activity sharing is off",
			"Synthetic activity, never shared or saved.",
			"Local storage",
			"Clear cache",
			"Removes cached messages and media. Drafts and your login stay.",
			"Messages and drafts are cached on this device inside bounded, account-isolated files. Cache data is not encrypted by Nivra; saved login tokens use the OS credential store.",
			"Your privacy",
			"Watch",
			"Open on YouTube",
			"Open on Vimeo",
			"Play",
			"Pause",
			"Resume",
			"Replay",
			"Retry",
			"Cancel",
			"Fullscreen",
			"Exit fullscreen (Esc)",
			"Seek video",
			"Download video",
			"Save this video to your computer",
			"Video attachment unavailable",
			"This file is not a video",
			"Unsupported embed video provider or URL",
			"Video preview limit: 100 MiB",
			"Video server does not support buffering; download to play externally",
			"Video download failed or changed; reload the conversation",
			"Video link expired; reload the conversation",
			"Video worker stopped; restart Nivra",
			"Could not start video worker",
			"Video audio output stopped",
			"Unsupported video audio timing",
			"Video buffering stalled; retry or download to play externally",
			"This video format or codec is not supported on this system.",
			"The video could not be decoded safely.",
			"Inline playback supports videos up to 1080p.",
			"Videos longer than two hours are not supported.",
			"This video cannot seek to that position.",
			"This video format or codec is not supported by Windows.",
			"This video format or codec is not supported by macOS.",
			"This video format or codec is not supported by GStreamer.",
			"Nivra does not collect telemetry or upload diagnostics. Discord retains service-side data according to its own policies.",
			"Offline preview · changes stay in this session and are never sent.",
			"Closing the window keeps Nivra in the menu bar. Quit from its menu to exit.",
			"Closing keeps Nivra running. Use the tray to show, minimize or quit.",
			"Closing the window keeps Nivra in the notification area. Quit from its menu to exit.",
			"Nivra keeps running in the system tray",
			"Takes effect the next time Nivra starts.",
			"Use a different account",
			"Waiting for Discord…",
			"Use another account",
			"Continue with Discord",
			"Welcome back",
			"Welcome to Nivra",
			"Continue with a saved account, or sign in with another one.",
			"Sign in with Discord.",
			"Early preview",
			"Checking your saved login",
			"Connecting to Discord",
			"Sign in to Discord",
			"discord.com · temporary login window · passwords and 2FA never leave the page",
			"Sign in with a session token",
			"For owners who already hold a valid Discord session token, for example from another signed-in Nivra install. Passwords and 2FA are never used here; this bypasses Discord's hosted login page entirely.",
			"Session token",
			"Connect with this token",
			"About Nivra",
			"Forget saved login",
			"Messaging, reactions, search and read markers have offline tests. Real Discord interoperability is still unverified; attachment uploads and advanced search remain incomplete.",
			"Messages and drafts are cached locally. Login tokens use the operating system credential store.",
			"Unofficial clients may put your Discord account at risk.",
			"Explore the offline preview",
			"Sample conversations. No Discord connection.",
			"or",
			"Sign in again",
			"Sign in before calling",
			"Sign in before changing your profile picture",
			"Sign in through Discord; saved-login lookup stopped",
			"Waiting for Discord login",
			"Platform login webview unavailable; see platform-support.md",
			"Platform login webview unavailable",
			"Interface language",
			"Language",
			"Loading discord.com…",
			"Saved accounts",
			"This is my account",
			"Check this to continue.",
			"Independent and open source. Not affiliated with Discord.",
			"Message",
			"Unread messages",
			"Mark as read",
			"Jump to unread",
			"Copy",
			"Copy message",
			"Copy download link",
			"Reply",
			"Forward",
			"Forward message",
			"Create Thread…",
			"View reactions",
			"Mark read through here",
			"Mark Unread",
			"Unpin message",
			"Pin message",
			"Edit message",
			"Enter to save · Shift+Enter for a new line · Esc to cancel",
			"Remove from delete selection",
			"Select for batch delete",
			"Select",
			"selected",
			"Shift+click selects a range · drag paints · Esc exits",
			"Esc stops the rest",
			"Stop",
			"Delete",
			"Download",
			"Save .txt",
			"Save .md",
			"Export chat",
			"Exporting chat",
			"Export cancelled",
			"Select all visible",
			"Select messages to enable actions",
			"None of the selected messages can be deleted",
			"Maximum 5 messages per delete",
			"No attachments in the selection",
			"Maximum 15 attachments per download",
			"You can delete up to 5 at a time",
			"Only your messages can be deleted here",
			"You can download up to 15 attachments at a time",
			"Download attachments",
			"Copy text",
			"Downloads",
			"Choose a folder…",
			"Queued",
			"Downloading",
			"Done",
			"Failed",
			"Cancelled",
			"Retry",
			"Open folder",
			"Selection copied",
			"Selection saved",
			"Download complete",
			"Download cancelled",
			"You can select up to 5 messages at a time.",
			"Delete message…",
			"Delete message immediately",
			"Message history is unavailable with current permission information.",
			"Mute",
			"Unmute",
			"Deafen",
			"Undeafen",
			"Disconnect",
			"Dismiss call",
			"Reconnect to call",
			"Recent call",
			"You were in this call recently",
			"Dismiss",
			"Reconnecting",
			"attempt",
			"Reconnect now",
			"Connected",
			"Reconnecting…",
			"No connection",
			"Ping",
			"Connected for",
			"Connection recovery",
			"Rejoin calls after brief disconnects",
			"Automatically returns to the same call when Discord reconnects within 15 seconds.",
			"This can put you back on voice without an extra tap after short outages.",
			"Only enable this if you are comfortable rejoining voice automatically on this device.",
			"Continue",
			"Enable auto-rejoin",
			"Share your screen",
			"Stop sharing",
			"Turn on camera",
			"Turn off camera",
			"Turn on microphone",
			"Turn off microphone",
			"Turn on incoming audio",
			"Turn off incoming audio",
			"Speaking is unavailable in this channel.",
			"Voice settings",
			"Microphone and speaker settings",
			"Noise suppression",
			"Share a screen or window",
			"Stop sharing your screen",
			"Stop sharing your camera",
			"Share your selected camera with this call",
			"Turn off noise suppression",
			"Turn on noise suppression",
			"Voice processing",
			"Voice processing & input mode",
			"Removes background noise from your microphone before anyone else hears it.",
			"Mode",
			"Off",
			"Light",
			"Standard",
			"Maximum",
			"Bot",
			"Screens",
			"Apps",
			"Options",
			"None open",
			"open",
			"Resolution",
			"Frame rate",
			"Choose a screen or window",
			"Looking for screens and windows…",
			"Offline preview · no screen is captured",
			"In a call",
			"In a call · microphone muted",
			"In a call · deafened",
			"unread mentions",
			"User volume",
			"Reset volume",
			"Silent",
			"Normal",
			"Louder than normal",
			"5% quieter",
			"5% louder",
			"Bots start at 50% to protect your hearing. You can still raise it here.",
			"Start bots at 50% volume",
			"Protects your hearing from bots that join very loud. Right-click a bot in the call to change its volume.",
			"Recommended",
			"All voice settings",
			"No filter. For studio microphones or when playing music.",
			"Steady hum like fans or air conditioning. Lightest on your PC.",
			"Keyboard, clicks and everyday home noise. Works well for most people.",
			"Very noisy home, or friends complain about your background noise. Uses more of your PC.",
			"PC usage: none",
			"PC usage: very low",
			"PC usage: low",
			"PC usage: medium",
			"Friends complaining about noise? Choose Maximum. If your voice cuts out or your PC slows down, go back to Standard.",
			"Your PC couldn't keep up with Maximum, so Nivra switched to Standard to keep your voice smooth.",
			"The defaults work for most people. Change these only if something sounds wrong.",
			"Click to turn on or off · right-click to choose the level",
			"Noise suppression is unavailable in this build or preview.",
			"Echo cancellation",
			"Recommended when speakers can be picked up by your microphone.",
			"Automatic microphone volume",
			"Keeps speech at a more consistent loudness without changing your output volume.",
			"Push to talk",
			"When enabled, your microphone transmits only while the configured shortcut is held.",
			"Mute and deafen always take priority.",
			"Hold your configured shortcut when you want to speak.",
			"Deafen turns off incoming audio and mutes your microphone with it.",
			"Advanced input settings",
			"Voice activity threshold",
			"Only transmit sound above the threshold.",
			"Open voice activity; mute and push to talk still apply.",
			"Input level",
			"Light suppression strength",
			"Higher levels remove more noise but can affect natural voice detail.",
			"Low",
			"Moderate",
			"High",
			"Very high",
			"Recommended defaults",
			"Raw microphone",
			"Devices & levels",
			"Input device",
			"Output device",
			"Microphone gain",
			"Speaker volume",
			"100% is the original level. Higher levels may distort.",
			"Rescan devices",
			"Reset levels",
			"System default follows your operating-system choice. Select a device only when you want Nivra to stay pinned to it.",
			"System default (recommended)",
			"Device unavailable — choose another",
			"Looking for audio devices...",
			"Looking for audio devices…",
			"Could not start audio device discovery",
			"Audio devices loaded · headphones avoid microphone echo",
			"Audio device discovery stopped",
			"One selected audio device is unavailable. Choose System default or rescan devices.",
			"Microphone unavailable · choose another input. You are still connected.",
			"Microphone unavailable · still connected. Choose another input in Audio settings.",
			"Camera",
			"Voice privacy code",
			"Call without end-to-end encryption",
			"Compare with the other participants. This code changes with the encrypted call group.",
			"Audio preferences are saved on this device. Your microphone starts only when you join a call or start testing.",
			"Install a voice-enabled build to use these controls.",
			"Friends",
			"Add Friend",
			"You can add friends with their Discord username.",
			"Username",
			"Enter a username",
			"Sending…",
			"Send Friend Request",
			"Offline demo · actions are simulated.",
			"Reconnect before sending a friend request.",
			"All",
			"Pending",
			"Blocked & Ignored",
			"All friends",
			"Blocked & ignored",
			"Blocked and ignored users are not available yet.",
			"Friends are not available yet.",
			"No blocked or ignored users match your search.",
			"No friends match your search.",
			"No blocked or ignored users.",
			"No friends yet.",
			"No friends are currently online.",
			"Some friends’ online status and activity couldn’t be loaded. The Online list may be incomplete.",
			"Dismiss friend status warning",
			"Direct Messages",
			"Mark As Read",
			"Server Settings",
			"Create invite",
			"Leave server",
			"Leave server?",
			"Are you sure you want to leave",
			"You will not be able to rejoin this server unless you are re-invited.",
			"Leaving…",
			"Leave Server",
			"Close",
			"Showing the beginning of a large file.",
			"Show more",
			"Offline preview · no server changes",
			"Remove From Favorites",
			"Add To Favorites",
			"Favorites are saved on this device.",
			"Invite to Channel",
			"Copy Link",
			"Unmute Channel",
			"Mute Channel",
			"For 15 Minutes",
			"For 1 Hour",
			"For 3 Hours",
			"For 8 Hours",
			"For 24 Hours",
			"Until I Turn It Back On",
			"Copy Channel ID",
			"Hide Muted Channels",
			"Restart to update",
			"Updating…",
			"Update available",
			"Navigation",
			"Move around Nivra without reaching for the mouse.",
			"Messages",
			"Composer shortcuts are only active while you are writing.",
			"Text Formatting",
			"Apply or remove formatting in the composer.",
			"Global availability",
			"Enable global shortcuts",
			"Mute, deafen and push-to-talk stay off until you turn this on. They then work even when Nivra is in the background.",
			"Show Keyboard Shortcuts List",
			"Switch Conversation",
			"Close Settings or Dialog",
			"Send Message",
			"Insert New Line",
			"Edit Last Editable Message",
			"Bold",
			"Italic",
			"Underline",
			"Strikethrough",
			"Inline Code",
			"Code Block",
			"Spoiler",
			"Push to Talk",
			"Toggle Mute",
			"Toggle Deafen",
			"Voice",
			"Control your microphone and incoming audio during a connected call.",
			"Already bound to",
			"Brazil",
			"United States",
			"Canada",
			"United Kingdom",
			"Germany",
			"Netherlands",
			"France",
			"Spain",
			"Poland",
			"Finland",
			"Sweden",
			"Singapore",
			"Japan",
			"Hong Kong",
			"Australia",
			"India",
			"South Africa",
			"Chile",
			"Argentina",
			"South Korea",
			"Europe",
			"Russia",
			"People in this call will see what you pick.",
			"Looking for your screens…",
			"Entire screen",
			"Share audio",
			"Also send sound from other apps. Your microphone stays as it is.",
			"Share an app",
			"No apps are open to share.",
			"App",
			"Refresh",
			"Quality",
			"Show cursor",
			"Include the pointer in the shared video.",
			"Share Screen",
			"Cancel",
			"Profile",
			"Mention",
			"Add Note",
			"Edit Friend Nickname",
			"Add Friend Nickname",
			"Private nicknames are available for confirmed friends.",
			"Pin DM",
			"Unpin DM",
			"Pinned direct messages are saved on this device.",
			"Mute Conversation",
			"Unmute Conversation",
			"Mute this direct message's notifications until you unmute it.",
			"Close DM",
			"Remove this conversation from your DM list. Messages are kept.",
			"No open direct message with this user.",
			"Block",
			"Unblock",
			"Change Nickname",
			"Nickname",
			"Roles",
			"Kick",
			"Save",
			"This removes the member from this server. They can rejoin with a new invite.",
			"Before you use Nivra",
			"Nivra is an unofficial app for your own Discord account: it is not Discord, it is not endorsed by Discord, and Discord's rules still apply to your account. It also includes other people's work (libraries, fonts and icons) under their own licenses, and accepting here does not waive those licenses or shift their copyright. Continuing confirms that you understand both points.",
			"Your app preferences could not be read, so Nivra cannot tell whether you accepted this before.",
			"View full licenses",
			"Hide full licenses",
			"Hide offline members",
			"Show offline members",
			"Display",
			"GLOBAL",
			"Send as text file?",
			"Your message is too long for chat, so it will be sent as a file.",
			"File name",
			"Send",
			"That file name will not work.",
			"Licenses",
			"Legal",
			"Licenses for the libraries, fonts, icons and sounds included in Nivra.",
			"Filter licenses",
			"No licenses match this filter.",
			"All licenses",
			crate::licenses::MPL_SOURCE_PREFIX,
			"Notification sounds",
			"Fonts",
			"Emoji",
			"Icons",
			"Core libraries",
			"Sign-in",
			"Audio playback",
			"Other dependencies",
			"Spam Filters",
			"Direct Messages",
			"Friend Requests",
			"Connected Games",
			"Direct Message (DM) Permissions",
			"Friend Request Permissions",
			"Messaging in Connected Games",
			"Automatically filter suspected spam messages",
			"Discord can filter out some messages that contain spam. These messages go to your Spam inbox.",
			"Filter all spam",
			"Filter messages from non-friends",
			"Recommended",
			"Don't filter spam",
			"Your account uses a custom spam filter setting. Select an option to replace it.",
			"All servers",
			"Server",
			"Some servers have different preferences. Choose a server to review its settings.",
			"Changes apply to all current servers and set the default for newly joined servers.",
			"Changes apply to this server only.",
			"Allow DMs from other server members",
			"Filter messages from server members I may not know",
			"Move messages from people you may not know into Message Requests.",
			"There are too many servers to update together. Choose an individual server.",
			"Saving…",
			"Loading your preferences…",
			"Try again",
			"Allow friend requests from",
			"Control who can send you friend requests and how they appear.",
			"Everyone",
			"Friends of friends",
			"Server members",
			"Only from servers where you also allow Direct Messages.",
			"Show personalized messages",
			"Show personalized messages on incoming friend requests. If you accept, the message will still appear in your DMs.",
			"Settings for games that use Discord to power their social experiences.",
			"Allow friends from games to send direct messages and invites",
			"Let friends from connected games send DMs and invite you to play, even when the game isn't open.",
			"Show Direct Messages in games",
			"Read and respond to DMs directly from in-game chats.",
			"Show all DMs",
			"Show only DMs from people who also play the game",
			"Don't show DMs",
			"Your account uses a custom in-game DM setting. Select an option to replace it.",
			"Search themes",
			"Search extensions",
			"Checking for packages and updates",
			"Working on your last action",
			"Clear search",
			"Create theme",
			"More",
			"Import theme…",
			"Refresh catalog",
			"Look for new packages and updates. Nothing installs on its own.",
			"Import package…",
			"Open a package file from this computer.",
			"No matches",
			"No themes yet",
			"No extensions yet",
			"Refresh the catalog or import a creator's package to get started.",
			"Try a different name or creator.",
			"Preview",
			"View preview",
			"Loading preview...",
			"Preview not loaded",
			"Preview unavailable",
			"Previewing theme",
			"Theme preview",
			"Changes are not saved yet",
			"Back to themes",
			"Back to theme editor",
			"Customize",
			"Use theme",
			"Apply this installed theme to the app.",
			"Edit theme",
			"Open tool",
			"Disable",
			"Update",
			"Active",
			"Enabled",
			"Remove",
			"by",
			"Plugin",
			"Cleanup pending",
			"Retry cleanup",
			"Add a new tool to your conversations.",
			"Install theme",
			"Review & enable",
			"Review the new release before it replaces this version.",
			"Remove this theme and delete its local data.",
			"Finish removing this extension and its local data.",
			"Removes this extension and deletes its local data.",
			"Selecting artwork sends it as an image attachment.",
			"Example deleted-message appearance",
			"Creator preview",
			"Close preview",
			"Reviewed",
			"Unreviewed",
			"View source",
			"Unreviewed package — its source has not been reviewed for the catalog.",
			"No access to conversations or composer text.",
			"Allow this extension to",
			"Enable this theme",
			"Enable this extension",
			"Everything it may touch is listed below.",
			"Allow every listed permission to continue.",
			"Disabling removes the extension and its local data. Re-enabling starts fresh.",
			"Save Changes",
			"Back",
			"Working…",
			"Theme details",
			"How your theme appears in the gallery.",
			"Theme name",
			"My theme",
			"Theme name is required.",
			"Created by",
			"Your name",
			"Creator name is required.",
			"Card cover",
			"Choose the image shown on your theme card in Themes.",
			"App background",
			"Use one image behind your conversations and sidebars.",
			"This older theme uses its original image placement.",
			"Use image across the app",
			"Preview in app",
			"Save and apply",
			"Basics",
			"Background",
			"Colors",
			"Advanced",
			"Discard unsaved theme?",
			"Your changes have not been saved.",
			"Discard changes",
			"Keep editing",
			"Custom cover",
			"Automatic preview",
			"Replace cover",
			"Choose cover",
			"Background image",
			"No image selected",
			"Replace image",
			"Choose image",
			"Editing",
			"Dark",
			"Image opacity",
			"Image fit",
			"Fill area",
			"Fit entire image",
			"Section opacity",
			"Select an area, then choose how much of the image shows through.",
			"Window gradient",
			"More colors",
			"Surface opacity",
			"0% shows the image. 100% is a solid section color.",
			"Selected section",
			"Top bars",
			"Server list",
			"People & channels",
			"Message list",
			"Member list",
			"Message input area",
			"Window title and conversation header",
			"The left server rail",
			"Direct messages and channel navigation",
			"The conversation timeline",
			"The member and search pane on the right",
			"The area around the message box",
			"Window background",
			"Sidebar",
			"Message area",
			"Cards & message input",
			"Hover",
			"Selection",
			"Borders",
			"Headings",
			"Body text",
			"Secondary text",
			"Text on accent",
			"Success",
			"Warning",
			"Error & danger",
			"Mention background",
			"Mention text",
			"Buttons, selection and highlights",
			"Messages and regular labels",
			"Timestamps and supporting text",
			"Channel, conversation and member lists",
			"Background behind your messages",
			"Use the default color for this appearance",
			"Use #RRGGBB or #RRGGBBAA.",
			"Horizontal",
			"Vertical",
			"Use the built-in value",
			"Text, spacing & corners",
			"These settings apply to dark and light appearances.",
			"Buttons",
			"Small text",
			"Code",
			"Control height",
			"Item spacing",
			"Button padding",
			"Control corners",
			"Window corners",
			"Menu corners",
			"Sharing & export",
			"The license and version are required. A source URL is optional for local themes.",
			"License",
			"Version",
			"Source URL",
			"Optional",
			"Use a valid HTTPS source URL or leave this blank.",
			"License and version are required.",
			"Only share images you own or have permission to use. Keep required attribution.",
			"Export theme",
			"Add a theme name and creator name before saving.",
			"Add a license and version before saving.",
			"Check the license, version, and optional source URL.",
			"Correct the highlighted color value.",
			"Keep image and section opacity between 0% and 100%.",
			"Correct the highlighted gradient value.",
			"Check the remaining theme settings before saving.",
			"Reload server settings",
			"Loading server settings…",
			"Reconnect to load server settings.",
			"Load server settings",
			"Discard unsaved changes?",
			"Your changes to this server will be lost.",
			"Wait for the current save to finish before closing.",
			"Delete server",
			"This action cannot be undone.",
			"Enter server name",
			"Offline preview · no server changes",
			"Deleting…",
			"Delete Server",
			"Server Profile",
			"Customize how your server appears in invite links and, if enabled, in Server Discovery and Announcement Channel messages.",
			"Name",
			"Icon",
			"We recommend an image of at least 512×512.",
			"Preparing icon…",
			"Change Server Icon",
			"Remove Icon",
			"Banner",
			"Traits",
			"Add up to 5 traits to show off your server's interests and personality.",
			"Trait name",
			"Remove trait",
			"Description",
			"How did your server get started? Why should people join?",
			"Tell the world a bit about this server.",
			"Saving changes…",
			"Careful — you have unsaved changes!",
			"Use a server name of 2–100 characters, a description of up to 300 characters, and valid traits without control characters.",
			"Could not save these changes. Check the selected channels, reconnect, or reload the server settings and try again.",
			"Reload the server settings before saving again. Your edits will be kept.",
			"Reconnect to save changes.",
			"MODERATION",
			"APPS",
			"EXPRESSION",
			"PEOPLE",
			"Engagement",
			"Manage settings that help keep your server active.",
			"System Messages",
			"Configure system event messages sent to your server.",
			"Send a random welcome message when someone joins this server.",
			"Prompt members to reply to welcome messages with a sticker.",
			"Send a message when someone boosts this server.",
			"Send helpful tips for server setup.",
			"System Messages Channel",
			"This is the channel we send system event messages to.",
			"Activity Feed Settings",
			"Shows a feed of activity from games and connected apps in this server.",
			"Display Activity Feed in this server",
			"Server default",
			"Default Notification Settings",
			"This will determine whether members who have not explicitly set their notification settings receive a notification for every message sent in this server or not.",
			"All Messages",
			"Only @mentions",
			"We highly recommend setting this to only @mentions for a Community Server.",
			"Inactive Channel",
			"Inactive Timeout",
			"Automatically move members to this channel and mute them when they have been idle for longer than the inactive timeout. This does not affect browsers.",
			"Unavailable channel",
			"No Inactive Channel",
			"No System Messages Channel",
			"None",
			"No accessible channels available.",
			"Stickers",
			"Members",
			"Invites",
			"Integrations",
			"Audit Log",
			"Enable explicit emoji and sticker image attachment selection",
			"Customize app colors, typography and control styling",
			"Read live message events and text in the active conversation",
			"Read the message I choose for an action",
			"Read my draft and propose text changes",
			"Store up to 1 MiB of local data for this account",
			"Read my account and current conversation details",
			"Read my loaded profile, including biography and pronouns",
			"Read my loaded server names and identifiers",
			"Read current channel metadata, recipients and permissions",
			"Receive changes to separately granted account and conversation data",
			"Read loaded embed text, stickers and message reference metadata",
			"Read loaded forum and thread summaries",
			"Read current typing users and loaded pins; observe reactions",
			"Read loaded channel topics, categories, thread details and permissions",
			"Read loaded server members, roles and server profiles",
			"Read the list of loaded, readable conversations",
			"Read loaded message replies, mentions, attachment metadata and reactions",
			"Read my loaded friends, requests, blocked and ignored users",
			"Read loaded messages in the active conversation",
			"Read loaded members of the active conversation",
			"Read loaded user presence status",
			"Read current call state and participant identifiers",
			"Read unread and mention counts in the active conversation",
			"Enable",
			"PNG or JPEG, up to 2 MiB. This image does not change the chat background.",
			"App preferences were not saved. If your acceptance of the terms was not recorded, Nivra will ask again next launch.",
			"Your session expired; sign in again to continue.",
			"Dismiss update",
			"Download update",
			"Check for updates",
			"Update checks are disabled in debug builds.",
			"Finish the current update before checking again.",
			"I understand — continue",
			"+",
			"< Integrations",
			"Add custom emoji that anyone can use in this server. Animated GIF emoji may be used by members with Discord Nitro.",
			"Add custom stickers for members to use in this server. Artwork is cropped and resized to 320 × 320 pixels before upload.",
			"All Actions",
			"All Users",
			"Bots and Apps",
			"Copy invite link",
			"Create Invite Link",
			"Create Role",
			"Create an invite link to welcome people to this server.",
			"Custom role color",
			"Default Permissions\n@everyone · applies to all server members",
			"Default role color",
			"Delete Emoji",
			"Delete Role",
			"Delete Sticker",
			"Description (optional)",
			"Discard Changes",
			"Drag and drop up to 10 images onto this page, or choose files. Review their names before uploading.",
			"Edit",
			"Emoji name",
			"Emoji name: 2–32 letters, numbers, or underscores",
			"Filter by Action",
			"Filter by User",
			"First page",
			"For example: 🐀",
			"Image",
			"Inactive for",
			"Keep Editing",
			"Kick Member",
			"Leave blank to use their username.",
			"Load More",
			"Loading audit log…",
			"MEMBERS",
			"Manage >",
			"Member details",
			"Members use the color of their highest role on the roles list.",
			"NONE",
			"Next page",
			"No active invite links",
			"No additional details were provided for this event.",
			"No audit log entries match these filters.",
			"No custom stickers yet.",
			"No integrations in this server.",
			"No members match this search.",
			"Permissions",
			"Posts from these followed channels are delivered to your server.",
			"Preparing emoji images...",
			"Preparing sticker artwork…",
			"Prune",
			"Prune Members",
			"Recent Members",
			"Related emoji",
			"Reload",
			"Reload Invites",
			"Reload Roles",
			"Reload integrations",
			"Reload integrations before making more changes. Your draft will be kept.",
			"Remove Integration",
			"Review sticker",
			"Review uploads",
			"Revoke Invite",
			"Revoke invite",
			"Role Style",
			"Role color",
			"Role icon",
			"Sample message",
			"Search Roles",
			"Search by username or ID",
			"Search members",
			"Search permissions",
			"Second gradient color",
			"Send updates from your apps and services to a channel in this server.",
			"Server Members",
			"Showing the first 50 integrations returned by Discord.",
			"Static PNG, JPEG and WebP artwork is supported up to 8 MB. The prepared PNG must fit within Discord's 512 KB limit.",
			"The audit log reached its local entry or memory limit. Adjust the filters to find other events.",
			"This integration is no longer available.",
			"This is how members with this role appear.",
			"Unknown",
			"Upload",
			"Upload Emoji",
			"Upload Sticker",
			"Uploaded By",
			"Use roles to group your server members and assign permissions.",
			"Use their username",
			"Your stickers",
			"←  BACK",
			"Tools",
			"Extension tool",
			"Review the result. App actions and draft changes need your approval.",
			"Proposed composer text",
			"Proposed app action",
			"Apply to Draft",
			"Voice Connected",
			"Voice preview",
			"Connecting…",
			"Call failed",
			"A selected message could not be deleted and is back in the conversation",
			"(edited)",
			". See all ",
			"Application interaction pending…",
			"Archived",
			"Archived posts need a connected session with history access.",
			"Choose a conversation to see its people.",
			"Clear this draft",
			"Copy edit text",
			"Dismiss message",
			"Display limited · Copy message for the full text",
			"Draft budget full. Clear an existing draft to continue.",
			"Enter a message...",
			"Hide spoilers",
			"Latest message unavailable",
			"Load archived posts",
			"Load more posts",
			"Loading archived posts…",
			"Loading posts…",
			"Message deleted",
			"No conversation selected",
			"No older archived posts reported.",
			"No people returned for this view.",
			"OFFLINE PREVIEW",
			"Older archived posts",
			"Only you can see this  •",
			"Open this channel’s threads",
			"Pick a channel or direct message from the list.",
			"Posting…",
			"Profile and status",
			"Remove Message",
			"Replying to ",
			"Retry",
			"Retry shortcuts",
			"Reveal spoiler media",
			"Search loaded conversations (Ctrl/Cmd+K)",
			"Search or create a post...",
			"Search this conversation",
			"Synthetic data · no network or local storage",
			"This is the beginning of the conversation.",
			"Thread started from this message",
			"Title",
			"Toggle Deleted Highlight",
			"View original",
			"View original message",
			"[Deleted message had no text]",
			"used",
			"· Save requested, check the connection before retrying",
			"↪ Forwarded",
			"Cancel download",
			"Choose files…",
			"Choose where to save this file · up to 100 MiB",
			"Clear selection",
			"Copy activity",
			"Copy download link",
			"Copy link",
			"Copy webhook ID",
			"Dismiss",
			"Edit profile",
			"Loading profile…",
			"Mute this direct message's notifications until you unmute it.",
			"No matching options loaded",
			"Offline preview · synthetic",
			"Open media",
			"Open original…",
			"Preview",
			"Refine your search to see more results",
			"Retry profile",
			"Reveal spoiler attachment",
			"Reveal spoiler component",
			"Reveal spoiler media",
			"Scroll to zoom · Drag to pan · Double-click to reset",
			"Search options",
			"Show remaining roles",
			"Submitting…",
			"This account was deleted. The conversation stays so you can read it.",
			"Type to search members; available roles and channels are listed",
			"View banner",
			"View profile picture",
		];
		for key in KEYS {
			assert!(portuguese_brazil(key).is_some(), "missing pt-BR: {key}");
			assert!(spanish(key).is_some(), "missing es: {key}");
		}
	}

	#[test]
	fn locales_fall_back_to_english_without_empty_controls() {
		assert_eq!(
			text(Language::PortugueseBrazil, "Voice & Video"),
			"Voz e vídeo"
		);
		assert_eq!(text(Language::Spanish, "Voice & Video"), "Voz y video");
		assert_eq!(
			text(Language::Spanish, "Untranslated sentinel"),
			"Untranslated sentinel"
		);
	}
}
