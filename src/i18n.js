// Translations follow the system language (WebView2 reports the Windows display language).
// Missing languages and keys fall back to English.
const MESSAGES = {
  en: {
    title: "Plan usage", loading: "Loading…", noAccounts: "No accounts",
    noCredentials: "No Claude Code or Codex credentials found on Windows or WSL.",
    searching: "Looking for accounts…", refresh: "Refresh",
    session: "Session (5h)", weekly: "Weekly", weekly_opus: "Weekly · Opus", weekly_sonnet: "Weekly · Sonnet", limit: "Limit",
    resetsIn: "Resets in {t}", resetsNow: "Resets now", credits: "Credits: {v}", unlimited: "Unlimited",
    token_expired: "Token expired — open {provider} to renew it",
    token_invalid: "Invalid token — open {provider} to sign in again",
    bad_response: "Unexpected response", http: "HTTP error {code}",
    quit: "Quit",
    show: "Show usage", settings: "Settings", sDisplay: "Display", sMode: "Mode", modeIsland: "Island", modeTray: "Tray icon", sMonitor: "Monitor", monitorPrimary: "Primary monitor", display: "Display {n}", primary: "primary", sExpand: "Open island on", expandHover: "Hover", expandClick: "Click", sData: "Data", sRefresh: "Refresh every", minutes: "{n} min", sProviders: "Providers", sGeneral: "General", sLanguage: "Language", langAuto: "System default", sStartup: "Launch at login",
    modeTaskbar: "Taskbar", dragHint: "Tip: drag the island to the top or a side of any monitor, or onto the taskbar.",
    sPosition: "Position", anchor_top: "Top (island)", anchor_left: "Left edge", anchor_right: "Right edge",
    unknownAccount: "Unknown account", inUse: "In use", use: "Use", switching: "Switching…", idle: "Not used recently — switch to it to see its usage", staleAgo: "updated {t} ago", switched: "{provider}: now using {to}", switchedAuto: "{provider} limit reached — switched to {to}", sAccounts: "Accounts", addAccount: "Add account", addHint: "Sign in in the terminal window that opened. The account shows up here when you're done.", alias: "Nickname", remove: "Remove", activeCantRemove: "Switch to another account before removing this one.", cliMissing: "Couldn't find the Claude Code or Codex CLI on Windows or WSL.", autoSwitch: "Switch automatically", switchAt: "Switch at",
    descDisplay: "Where the island lives and how it opens.", descAccounts: "Your Claude and Codex accounts, and when to switch between them.", descData: "How often usage is checked, and for which tools.", descGeneral: "Language and startup.",
    rateLimited: "The usage service is busy — trying again in a few minutes",
    sUpdates: "Updates", autoUpdate: "Update automatically", checkUpdates: "Check for updates", checking: "Checking…", upToDate: "You're up to date", updateAvailable: "Version {v} is available", installUpdate: "Install and restart", updating: "Updating to {v}…", versionN: "Version {v}",
   
    resetFull: "Full reset", resetSession: "Session reset", resetUsableNow: "usable now", resetExpires: "expires {d}", resetNext: "next in {t}", resetsTitle: "Limit resets",
    resetNextExpires: "next expires {d}",
    sCompact: "Collapsed style", compactNames: "Names", compactMinimal: "Minimal",
    updatedTo: "Updated to v{v}",
  },
  pt: {
    title: "Uso dos planos", loading: "Carregando…", noAccounts: "Nenhuma conta",
    noCredentials: "Nenhuma credencial do Claude Code ou Codex no Windows ou WSL.",
    searching: "Procurando contas…", refresh: "Atualizar",
    session: "Sessão (5h)", weekly: "Semana", weekly_opus: "Semana · Opus", weekly_sonnet: "Semana · Sonnet", limit: "Limite",
    resetsIn: "Reinicia em {t}", resetsNow: "Reinicia agora", credits: "Créditos: {v}", unlimited: "Ilimitado",
    token_expired: "Token expirado — abra o {provider} para renovar",
    token_invalid: "Token inválido — abra o {provider} para entrar de novo",
    bad_response: "Resposta inesperada", http: "Erro HTTP {code}",
    quit: "Sair",
    show: "Mostrar uso", settings: "Configurações", sDisplay: "Exibição", sMode: "Modo", modeIsland: "Ilha", modeTray: "Ícone na bandeja", sMonitor: "Monitor", monitorPrimary: "Monitor principal", display: "Tela {n}", primary: "principal", sExpand: "Abrir a ilha ao", expandHover: "Passar o mouse", expandClick: "Clicar", sData: "Dados", sRefresh: "Atualizar a cada", minutes: "{n} min", sProviders: "Provedores", sGeneral: "Geral", sLanguage: "Idioma", langAuto: "Padrão do sistema", sStartup: "Iniciar com o Windows",
    modeTaskbar: "Barra de tarefas", dragHint: "Dica: arraste a ilha para o topo ou uma lateral de qualquer monitor, ou para a barra de tarefas.",
    sPosition: "Posição", anchor_top: "Topo (ilha)", anchor_left: "Lateral esquerda", anchor_right: "Lateral direita",
    unknownAccount: "Conta desconhecida", inUse: "Em uso", use: "Usar", switching: "Trocando…", idle: "Sem uso recente — troque para ela para ver o uso", staleAgo: "atualizado há {t}", switched: "{provider}: usando {to}", switchedAuto: "Limite do {provider} atingido — trocado para {to}", sAccounts: "Contas", addAccount: "Adicionar conta", addHint: "Entre na janela de terminal que abriu. A conta aparece aqui quando você terminar.", alias: "Apelido", remove: "Remover", activeCantRemove: "Troque para outra conta antes de remover esta.", cliMissing: "Não encontrei o Claude Code nem o Codex no Windows ou no WSL.", autoSwitch: "Trocar automaticamente", switchAt: "Trocar em",
    descDisplay: "Onde a ilha fica e como ela abre.", descAccounts: "Suas contas do Claude e do Codex, e quando trocar entre elas.", descData: "Com que frequência o uso é consultado, e de quais ferramentas.", descGeneral: "Idioma e inicialização.",
    rateLimited: "O serviço de uso está ocupado — tentando de novo em alguns minutos",
    sUpdates: "Atualizações", autoUpdate: "Atualizar automaticamente", checkUpdates: "Procurar atualizações", checking: "Procurando…", upToDate: "Você está na versão mais recente", updateAvailable: "A versão {v} está disponível", installUpdate: "Instalar e reiniciar", updating: "Atualizando para {v}…", versionN: "Versão {v}",
   
    resetFull: "Reset completo", resetSession: "Reset da sessão", resetUsableNow: "utilizável agora", resetExpires: "expira em {d}", resetNext: "próximo em {t}", resetsTitle: "Resets de limite",
    resetNextExpires: "próximo expira em {d}",
    sCompact: "Estilo recolhido", compactNames: "Nomes", compactMinimal: "Minimalista",
    updatedTo: "Atualizado para a v{v}",
  },
  es: {
    title: "Uso de los planes", loading: "Cargando…", noAccounts: "Sin cuentas",
    noCredentials: "No se encontraron credenciales de Claude Code o Codex en Windows ni WSL.",
    searching: "Buscando cuentas…", refresh: "Actualizar",
    session: "Sesión (5 h)", weekly: "Semana", weekly_opus: "Semana · Opus", weekly_sonnet: "Semana · Sonnet", limit: "Límite",
    resetsIn: "Se reinicia en {t}", resetsNow: "Se reinicia ahora", credits: "Créditos: {v}", unlimited: "Ilimitado",
    token_expired: "Token caducado: abre {provider} para renovarlo",
    token_invalid: "Token no válido: abre {provider} para volver a iniciar sesión",
    bad_response: "Respuesta inesperada", http: "Error HTTP {code}",
    quit: "Salir",
    show: "Mostrar uso", settings: "Configuración", sDisplay: "Pantalla", sMode: "Modo", modeIsland: "Isla", modeTray: "Icono en la bandeja", sMonitor: "Monitor", monitorPrimary: "Monitor principal", display: "Pantalla {n}", primary: "principal", sExpand: "Abrir la isla al", expandHover: "Pasar el ratón", expandClick: "Hacer clic", sData: "Datos", sRefresh: "Actualizar cada", minutes: "{n} min", sProviders: "Proveedores", sGeneral: "General", sLanguage: "Idioma", langAuto: "Predeterminado del sistema", sStartup: "Iniciar con Windows",
    modeTaskbar: "Barra de tareas", dragHint: "Consejo: arrastra la isla arriba o a un lado de cualquier monitor, o a la barra de tareas.",
    sPosition: "Posición", anchor_top: "Arriba (isla)", anchor_left: "Borde izquierdo", anchor_right: "Borde derecho",
    unknownAccount: "Cuenta desconocida", inUse: "En uso", use: "Usar", switching: "Cambiando…", idle: "Sin uso reciente: cámbiate a ella para ver su uso", staleAgo: "actualizado hace {t}", switched: "{provider}: usando {to}", switchedAuto: "Límite de {provider} alcanzado: cambiado a {to}", sAccounts: "Cuentas", addAccount: "Añadir cuenta", addHint: "Inicia sesión en la ventana de terminal que se abrió. La cuenta aparecerá aquí al terminar.", alias: "Apodo", remove: "Quitar", activeCantRemove: "Cambia a otra cuenta antes de quitar esta.", cliMissing: "No se encontró la CLI de Claude Code ni de Codex en Windows ni en WSL.", autoSwitch: "Cambiar automáticamente", switchAt: "Cambiar al",
    descDisplay: "Dónde vive la isla y cómo se abre.", descAccounts: "Tus cuentas de Claude y Codex, y cuándo cambiar entre ellas.", descData: "Cada cuánto se consulta el uso y de qué herramientas.", descGeneral: "Idioma e inicio.",
    rateLimited: "El servicio de uso está ocupado: se reintentará en unos minutos",
    sUpdates: "Actualizaciones", autoUpdate: "Actualizar automáticamente", checkUpdates: "Buscar actualizaciones", checking: "Buscando…", upToDate: "Tienes la última versión", updateAvailable: "La versión {v} está disponible", installUpdate: "Instalar y reiniciar", updating: "Actualizando a {v}…", versionN: "Versión {v}",
   
    resetFull: "Reinicio completo", resetSession: "Reinicio de sesión", resetUsableNow: "utilizable ahora", resetExpires: "caduca el {d}", resetNext: "próximo en {t}", resetsTitle: "Reinicios de límite",
    resetNextExpires: "el próximo caduca el {d}",
    sCompact: "Estilo contraído", compactNames: "Nombres", compactMinimal: "Minimalista",
    updatedTo: "Actualizado a la v{v}",
  },
  fr: {
    title: "Utilisation des forfaits", loading: "Chargement…", noAccounts: "Aucun compte",
    noCredentials: "Aucun identifiant Claude Code ou Codex trouvé sur Windows ou WSL.",
    searching: "Recherche des comptes…", refresh: "Actualiser",
    session: "Session (5 h)", weekly: "Semaine", weekly_opus: "Semaine · Opus", weekly_sonnet: "Semaine · Sonnet", limit: "Limite",
    resetsIn: "Réinitialisation dans {t}", resetsNow: "Réinitialisation imminente", credits: "Crédits : {v}", unlimited: "Illimité",
    token_expired: "Jeton expiré — ouvrez {provider} pour le renouveler",
    token_invalid: "Jeton invalide — ouvrez {provider} pour vous reconnecter",
    bad_response: "Réponse inattendue", http: "Erreur HTTP {code}",
    quit: "Quitter",
    show: "Afficher l’utilisation", settings: "Paramètres", sDisplay: "Affichage", sMode: "Mode", modeIsland: "Île", modeTray: "Icône de la barre d’état", sMonitor: "Écran", monitorPrimary: "Écran principal", display: "Écran {n}", primary: "principal", sExpand: "Ouvrir l’île au", expandHover: "Survol", expandClick: "Clic", sData: "Données", sRefresh: "Actualiser toutes les", minutes: "{n} min", sProviders: "Fournisseurs", sGeneral: "Général", sLanguage: "Langue", langAuto: "Langue du système", sStartup: "Lancer au démarrage",
    modeTaskbar: "Barre des tâches", dragHint: "Astuce : faites glisser l’île en haut ou sur un côté de n’importe quel écran, ou sur la barre des tâches.",
    sPosition: "Position", anchor_top: "En haut (île)", anchor_left: "Bord gauche", anchor_right: "Bord droit",
    unknownAccount: "Compte inconnu", inUse: "Utilisé", use: "Utiliser", switching: "Changement…", idle: "Pas utilisé récemment — passez-y pour voir son utilisation", staleAgo: "mis à jour il y a {t}", switched: "{provider} : compte {to} utilisé", switchedAuto: "Limite {provider} atteinte — passage à {to}", sAccounts: "Comptes", addAccount: "Ajouter un compte", addHint: "Connectez-vous dans la fenêtre de terminal qui s’est ouverte. Le compte apparaîtra ici ensuite.", alias: "Surnom", remove: "Supprimer", activeCantRemove: "Passez à un autre compte avant de supprimer celui-ci.", cliMissing: "CLI Claude Code ou Codex introuvable sous Windows ou WSL.", autoSwitch: "Changer automatiquement", switchAt: "Changer à",
    descDisplay: "Où se trouve l’île et comment elle s’ouvre.", descAccounts: "Vos comptes Claude et Codex, et quand passer de l’un à l’autre.", descData: "À quelle fréquence l’utilisation est vérifiée, et pour quels outils.", descGeneral: "Langue et démarrage.",
    rateLimited: "Le service d’utilisation est occupé — nouvel essai dans quelques minutes",
    sUpdates: "Mises à jour", autoUpdate: "Mettre à jour automatiquement", checkUpdates: "Rechercher des mises à jour", checking: "Recherche…", upToDate: "Vous êtes à jour", updateAvailable: "La version {v} est disponible", installUpdate: "Installer et redémarrer", updating: "Mise à jour vers {v}…", versionN: "Version {v}",
   
    resetFull: "Réinitialisation complète", resetSession: "Réinitialisation de session", resetUsableNow: "utilisable maintenant", resetExpires: "expire le {d}", resetNext: "prochaine dans {t}", resetsTitle: "Réinitialisations de limite",
    resetNextExpires: "le prochain expire le {d}",
    sCompact: "Style réduit", compactNames: "Noms", compactMinimal: "Minimaliste",
    updatedTo: "Mis à jour vers la v{v}",
  },
  de: {
    title: "Plan-Nutzung", loading: "Wird geladen…", noAccounts: "Keine Konten",
    noCredentials: "Keine Claude-Code- oder Codex-Anmeldedaten unter Windows oder WSL gefunden.",
    searching: "Konten werden gesucht…", refresh: "Aktualisieren",
    session: "Sitzung (5 Std.)", weekly: "Woche", weekly_opus: "Woche · Opus", weekly_sonnet: "Woche · Sonnet", limit: "Limit",
    resetsIn: "Zurückgesetzt in {t}", resetsNow: "Wird jetzt zurückgesetzt", credits: "Guthaben: {v}", unlimited: "Unbegrenzt",
    token_expired: "Token abgelaufen – öffne {provider}, um es zu erneuern",
    token_invalid: "Token ungültig – öffne {provider}, um dich neu anzumelden",
    bad_response: "Unerwartete Antwort", http: "HTTP-Fehler {code}",
    quit: "Beenden",
    show: "Nutzung anzeigen", settings: "Einstellungen", sDisplay: "Anzeige", sMode: "Modus", modeIsland: "Insel", modeTray: "Tray-Symbol", sMonitor: "Bildschirm", monitorPrimary: "Hauptbildschirm", display: "Bildschirm {n}", primary: "Haupt", sExpand: "Insel öffnen bei", expandHover: "Überfahren", expandClick: "Klick", sData: "Daten", sRefresh: "Aktualisieren alle", minutes: "{n} Min.", sProviders: "Anbieter", sGeneral: "Allgemein", sLanguage: "Sprache", langAuto: "Systemstandard", sStartup: "Beim Anmelden starten",
    modeTaskbar: "Taskleiste", dragHint: "Tipp: Zieh die Insel an den oberen Rand oder eine Seite eines beliebigen Bildschirms oder auf die Taskleiste.",
    sPosition: "Position", anchor_top: "Oben (Insel)", anchor_left: "Linker Rand", anchor_right: "Rechter Rand",
    unknownAccount: "Unbekanntes Konto", inUse: "Aktiv", use: "Verwenden", switching: "Wechsle…", idle: "Länger nicht genutzt – wechsle dorthin, um die Nutzung zu sehen", staleAgo: "vor {t} aktualisiert", switched: "{provider}: nutzt jetzt {to}", switchedAuto: "{provider}-Limit erreicht – zu {to} gewechselt", sAccounts: "Konten", addAccount: "Konto hinzufügen", addHint: "Melde dich im geöffneten Terminalfenster an. Das Konto erscheint hier, sobald du fertig bist.", alias: "Spitzname", remove: "Entfernen", activeCantRemove: "Wechsle zuerst zu einem anderen Konto, bevor du dieses entfernst.", cliMissing: "Claude-Code- oder Codex-CLI unter Windows oder WSL nicht gefunden.", autoSwitch: "Automatisch wechseln", switchAt: "Wechseln bei",
    descDisplay: "Wo die Insel sitzt und wie sie sich öffnet.", descAccounts: "Deine Claude- und Codex-Konten und wann zwischen ihnen gewechselt wird.", descData: "Wie oft die Nutzung abgefragt wird und für welche Tools.", descGeneral: "Sprache und Autostart.",
    rateLimited: "Der Nutzungsdienst ist ausgelastet – neuer Versuch in ein paar Minuten",
    sUpdates: "Updates", autoUpdate: "Automatisch aktualisieren", checkUpdates: "Nach Updates suchen", checking: "Suche…", upToDate: "Du bist auf dem neuesten Stand", updateAvailable: "Version {v} ist verfügbar", installUpdate: "Installieren und neu starten", updating: "Aktualisiere auf {v}…", versionN: "Version {v}",
   
    resetFull: "Vollständiger Reset", resetSession: "Sitzungs-Reset", resetUsableNow: "jetzt nutzbar", resetExpires: "läuft ab am {d}", resetNext: "nächster in {t}", resetsTitle: "Limit-Resets",
    resetNextExpires: "nächster läuft ab am {d}",
    sCompact: "Eingeklappter Stil", compactNames: "Namen", compactMinimal: "Minimal",
    updatedTo: "Auf v{v} aktualisiert",
  },
  it: {
    title: "Utilizzo dei piani", loading: "Caricamento…", noAccounts: "Nessun account",
    noCredentials: "Nessuna credenziale di Claude Code o Codex trovata su Windows o WSL.",
    searching: "Ricerca account…", refresh: "Aggiorna",
    session: "Sessione (5 h)", weekly: "Settimana", weekly_opus: "Settimana · Opus", weekly_sonnet: "Settimana · Sonnet", limit: "Limite",
    resetsIn: "Si azzera tra {t}", resetsNow: "Si azzera ora", credits: "Crediti: {v}", unlimited: "Illimitati",
    token_expired: "Token scaduto: apri {provider} per rinnovarlo",
    token_invalid: "Token non valido: apri {provider} per accedere di nuovo",
    bad_response: "Risposta inattesa", http: "Errore HTTP {code}",
    quit: "Esci",
    show: "Mostra utilizzo", settings: "Impostazioni", sDisplay: "Visualizzazione", sMode: "Modalità", modeIsland: "Isola", modeTray: "Icona nell’area di notifica", sMonitor: "Monitor", monitorPrimary: "Monitor principale", display: "Schermo {n}", primary: "principale", sExpand: "Apri l’isola al", expandHover: "Passaggio del mouse", expandClick: "Clic", sData: "Dati", sRefresh: "Aggiorna ogni", minutes: "{n} min", sProviders: "Provider", sGeneral: "Generale", sLanguage: "Lingua", langAuto: "Predefinita di sistema", sStartup: "Avvia all’accesso",
    modeTaskbar: "Barra delle applicazioni", dragHint: "Suggerimento: trascina l’isola in alto o su un lato di qualsiasi monitor, oppure sulla barra delle applicazioni.",
    sPosition: "Posizione", anchor_top: "In alto (isola)", anchor_left: "Bordo sinistro", anchor_right: "Bordo destro",
    unknownAccount: "Account sconosciuto", inUse: "In uso", use: "Usa", switching: "Cambio…", idle: "Non usato di recente: passaci per vederne l’utilizzo", staleAgo: "aggiornato {t} fa", switched: "{provider}: ora usa {to}", switchedAuto: "Limite di {provider} raggiunto: passato a {to}", sAccounts: "Account", addAccount: "Aggiungi account", addHint: "Accedi nella finestra del terminale che si è aperta. L’account apparirà qui al termine.", alias: "Soprannome", remove: "Rimuovi", activeCantRemove: "Passa a un altro account prima di rimuovere questo.", cliMissing: "CLI di Claude Code o Codex non trovata su Windows o WSL.", autoSwitch: "Cambia automaticamente", switchAt: "Cambia al",
    descDisplay: "Dove si trova l’isola e come si apre.", descAccounts: "I tuoi account Claude e Codex, e quando passare dall’uno all’altro.", descData: "Ogni quanto viene controllato l’utilizzo e per quali strumenti.", descGeneral: "Lingua e avvio.",
    rateLimited: "Il servizio di utilizzo è occupato: nuovo tentativo tra qualche minuto",
    sUpdates: "Aggiornamenti", autoUpdate: "Aggiorna automaticamente", checkUpdates: "Cerca aggiornamenti", checking: "Ricerca…", upToDate: "Sei aggiornato", updateAvailable: "È disponibile la versione {v}", installUpdate: "Installa e riavvia", updating: "Aggiornamento a {v}…", versionN: "Versione {v}",
   
    resetFull: "Reset completo", resetSession: "Reset della sessione", resetUsableNow: "utilizzabile ora", resetExpires: "scade il {d}", resetNext: "prossimo tra {t}", resetsTitle: "Reset del limite",
    resetNextExpires: "il prossimo scade il {d}",
    sCompact: "Stile compatto", compactNames: "Nomi", compactMinimal: "Minimale",
    updatedTo: "Aggiornato alla v{v}",
  },
  ja: {
    title: "プランの使用状況", loading: "読み込み中…", noAccounts: "アカウントなし",
    noCredentials: "Windows または WSL に Claude Code / Codex の認証情報が見つかりません。",
    searching: "アカウントを検索中…", refresh: "更新",
    session: "セッション（5時間）", weekly: "週間", weekly_opus: "週間 · Opus", weekly_sonnet: "週間 · Sonnet", limit: "上限",
    resetsIn: "{t}後にリセット", resetsNow: "まもなくリセット", credits: "クレジット: {v}", unlimited: "無制限",
    token_expired: "トークンの期限切れ — {provider} を開いて更新してください",
    token_invalid: "トークンが無効です — {provider} を開いて再ログインしてください",
    bad_response: "予期しない応答", http: "HTTP エラー {code}",
    quit: "終了",
    show: "使用状況を表示", settings: "設定", sDisplay: "表示", sMode: "モード", modeIsland: "アイランド", modeTray: "トレイアイコン", sMonitor: "モニター", monitorPrimary: "メインモニター", display: "ディスプレイ {n}", primary: "メイン", sExpand: "アイランドを開く操作", expandHover: "ホバー", expandClick: "クリック", sData: "データ", sRefresh: "更新間隔", minutes: "{n} 分", sProviders: "プロバイダー", sGeneral: "一般", sLanguage: "言語", langAuto: "システムの既定", sStartup: "ログイン時に起動",
    modeTaskbar: "タスクバー", dragHint: "ヒント: アイランドは任意のモニターの上端や左右の端、またはタスクバーにドラッグできます。",
    sPosition: "位置", anchor_top: "上部（アイランド）", anchor_left: "左端", anchor_right: "右端",
    unknownAccount: "不明なアカウント", inUse: "使用中", use: "使用", switching: "切り替え中…", idle: "最近使われていません — 切り替えると使用状況が表示されます", staleAgo: "{t}前に更新", switched: "{provider}: {to} を使用中", switchedAuto: "{provider} の上限に到達 — {to} に切り替えました", sAccounts: "アカウント", addAccount: "アカウントを追加", addHint: "開いたターミナルでサインインしてください。完了するとここに表示されます。", alias: "ニックネーム", remove: "削除", activeCantRemove: "削除する前に別のアカウントに切り替えてください。", cliMissing: "Windows または WSL に Claude Code / Codex の CLI が見つかりません。", autoSwitch: "自動で切り替える", switchAt: "切り替えのしきい値",
    descDisplay: "アイランドの位置と開き方。", descAccounts: "Claude と Codex のアカウントと、切り替えのタイミング。", descData: "使用状況を確認する頻度と対象のツール。", descGeneral: "言語と起動。",
    rateLimited: "使用状況サービスが混み合っています。数分後に再試行します",
    sUpdates: "アップデート", autoUpdate: "自動的にアップデート", checkUpdates: "アップデートを確認", checking: "確認中…", upToDate: "最新バージョンです", updateAvailable: "バージョン {v} が利用可能です", installUpdate: "インストールして再起動", updating: "{v} にアップデート中…", versionN: "バージョン {v}",
   
    resetFull: "フルリセット", resetSession: "セッションリセット", resetUsableNow: "今すぐ使用可能", resetExpires: "{d} に期限切れ", resetNext: "次回まで {t}", resetsTitle: "上限リセット",
    resetNextExpires: "次は {d} に期限切れ",
    sCompact: "折りたたみ時の表示", compactNames: "名前", compactMinimal: "ミニマル",
    updatedTo: "v{v} にアップデートしました",
  },
  zh: {
    title: "套餐用量", loading: "加载中…", noAccounts: "没有账户",
    noCredentials: "在 Windows 或 WSL 中未找到 Claude Code 或 Codex 凭据。",
    searching: "正在查找账户…", refresh: "刷新",
    session: "会话（5 小时）", weekly: "每周", weekly_opus: "每周 · Opus", weekly_sonnet: "每周 · Sonnet", limit: "限额",
    resetsIn: "{t}后重置", resetsNow: "即将重置", credits: "额度：{v}", unlimited: "无限",
    token_expired: "令牌已过期 — 请打开 {provider} 续期",
    token_invalid: "令牌无效 — 请打开 {provider} 重新登录",
    bad_response: "意外的响应", http: "HTTP 错误 {code}",
    quit: "退出",
    show: "显示用量", settings: "设置", sDisplay: "显示", sMode: "模式", modeIsland: "灵动岛", modeTray: "托盘图标", sMonitor: "显示器", monitorPrimary: "主显示器", display: "显示器 {n}", primary: "主", sExpand: "打开灵动岛的方式", expandHover: "悬停", expandClick: "点击", sData: "数据", sRefresh: "刷新间隔", minutes: "{n} 分钟", sProviders: "服务商", sGeneral: "通用", sLanguage: "语言", langAuto: "跟随系统", sStartup: "登录时启动",
    modeTaskbar: "任务栏", dragHint: "提示：可以将灵动岛拖到任意显示器的顶部或两侧，或拖到任务栏。",
    sPosition: "位置", anchor_top: "顶部（灵动岛）", anchor_left: "左侧", anchor_right: "右侧",
    unknownAccount: "未知账户", inUse: "使用中", use: "使用", switching: "正在切换…", idle: "最近未使用 — 切换过去即可查看用量", staleAgo: "{t}前更新", switched: "{provider}：正在使用 {to}", switchedAuto: "{provider} 已达上限 — 已切换到 {to}", sAccounts: "账户", addAccount: "添加账户", addHint: "请在打开的终端窗口中登录，完成后账户会显示在这里。", alias: "昵称", remove: "移除", activeCantRemove: "请先切换到其他账户再移除此账户。", cliMissing: "在 Windows 或 WSL 中未找到 Claude Code 或 Codex CLI。", autoSwitch: "自动切换", switchAt: "切换阈值",
    descDisplay: "灵动岛的位置和打开方式。", descAccounts: "你的 Claude 和 Codex 账户，以及何时在它们之间切换。", descData: "多久检查一次用量，以及检查哪些工具。", descGeneral: "语言和启动。",
    rateLimited: "用量服务繁忙，几分钟后重试",
    sUpdates: "更新", autoUpdate: "自动更新", checkUpdates: "检查更新", checking: "正在检查…", upToDate: "已是最新版本", updateAvailable: "有新版本 {v} 可用", installUpdate: "安装并重启", updating: "正在更新到 {v}…", versionN: "版本 {v}",
   
    resetFull: "完全重置", resetSession: "会话重置", resetUsableNow: "现在可用", resetExpires: "{d} 到期", resetNext: "{t}后可用", resetsTitle: "额度重置",
    resetNextExpires: "最早 {d} 到期",
    sCompact: "收起样式", compactNames: "名称", compactMinimal: "极简",
    updatedTo: "已更新到 v{v}",
  },
  ko: {
    title: "요금제 사용량", loading: "불러오는 중…", noAccounts: "계정 없음",
    noCredentials: "Windows 또는 WSL에서 Claude Code 또는 Codex 자격 증명을 찾을 수 없습니다.",
    searching: "계정을 찾는 중…", refresh: "새로 고침",
    session: "세션 (5시간)", weekly: "주간", weekly_opus: "주간 · Opus", weekly_sonnet: "주간 · Sonnet", limit: "한도",
    resetsIn: "{t} 후 초기화", resetsNow: "곧 초기화", credits: "크레딧: {v}", unlimited: "무제한",
    token_expired: "토큰 만료 — {provider}을(를) 열어 갱신하세요",
    token_invalid: "토큰이 유효하지 않음 — {provider}을(를) 열어 다시 로그인하세요",
    bad_response: "예상치 못한 응답", http: "HTTP 오류 {code}",
    quit: "종료",
    show: "사용량 보기", settings: "설정", sDisplay: "표시", sMode: "모드", modeIsland: "아일랜드", modeTray: "트레이 아이콘", sMonitor: "모니터", monitorPrimary: "주 모니터", display: "디스플레이 {n}", primary: "주", sExpand: "아일랜드 열기", expandHover: "마우스 오버", expandClick: "클릭", sData: "데이터", sRefresh: "새로 고침 간격", minutes: "{n}분", sProviders: "제공자", sGeneral: "일반", sLanguage: "언어", langAuto: "시스템 기본값", sStartup: "로그인 시 실행",
    modeTaskbar: "작업 표시줄", dragHint: "팁: 아일랜드를 모니터의 위쪽이나 양옆 또는 작업 표시줄로 끌어 놓을 수 있습니다.",
    sPosition: "위치", anchor_top: "상단 (아일랜드)", anchor_left: "왼쪽 가장자리", anchor_right: "오른쪽 가장자리",
    unknownAccount: "알 수 없는 계정", inUse: "사용 중", use: "사용", switching: "전환 중…", idle: "최근 사용 안 함 — 전환하면 사용량을 볼 수 있습니다", staleAgo: "{t} 전 업데이트", switched: "{provider}: {to} 사용 중", switchedAuto: "{provider} 한도 도달 — {to}(으)로 전환했습니다", sAccounts: "계정", addAccount: "계정 추가", addHint: "열린 터미널 창에서 로그인하세요. 완료되면 여기에 계정이 표시됩니다.", alias: "별명", remove: "제거", activeCantRemove: "이 계정을 제거하기 전에 다른 계정으로 전환하세요.", cliMissing: "Windows 또는 WSL에서 Claude Code 또는 Codex CLI를 찾을 수 없습니다.", autoSwitch: "자동 전환", switchAt: "전환 기준",
    descDisplay: "아일랜드의 위치와 여는 방식.", descAccounts: "Claude와 Codex 계정, 그리고 계정을 전환할 시점.", descData: "사용량을 확인하는 주기와 대상 도구.", descGeneral: "언어 및 시작.",
    rateLimited: "사용량 서비스가 혼잡합니다. 몇 분 후 다시 시도합니다",
    sUpdates: "업데이트", autoUpdate: "자동 업데이트", checkUpdates: "업데이트 확인", checking: "확인 중…", upToDate: "최신 버전입니다", updateAvailable: "버전 {v}을(를) 사용할 수 있습니다", installUpdate: "설치 후 재시작", updating: "{v}(으)로 업데이트 중…", versionN: "버전 {v}",
   
    resetFull: "전체 초기화", resetSession: "세션 초기화", resetUsableNow: "지금 사용 가능", resetExpires: "{d} 만료", resetNext: "{t} 후 다음", resetsTitle: "한도 초기화",
    resetNextExpires: "다음 만료 {d}",
    sCompact: "접힌 상태 스타일", compactNames: "이름", compactMinimal: "미니멀",
    updatedTo: "v{v}(으)로 업데이트됨",
  },
  ru: {
    title: "Использование тарифов", loading: "Загрузка…", noAccounts: "Нет аккаунтов",
    noCredentials: "Учётные данные Claude Code или Codex не найдены ни в Windows, ни в WSL.",
    searching: "Поиск аккаунтов…", refresh: "Обновить",
    session: "Сессия (5 ч)", weekly: "Неделя", weekly_opus: "Неделя · Opus", weekly_sonnet: "Неделя · Sonnet", limit: "Лимит",
    resetsIn: "Сброс через {t}", resetsNow: "Сброс сейчас", credits: "Кредиты: {v}", unlimited: "Без ограничений",
    token_expired: "Токен истёк — откройте {provider}, чтобы обновить его",
    token_invalid: "Недействительный токен — откройте {provider}, чтобы войти снова",
    bad_response: "Неожиданный ответ", http: "Ошибка HTTP {code}",
    quit: "Выход",
    show: "Показать использование", settings: "Настройки", sDisplay: "Отображение", sMode: "Режим", modeIsland: "Остров", modeTray: "Значок в трее", sMonitor: "Монитор", monitorPrimary: "Основной монитор", display: "Дисплей {n}", primary: "основной", sExpand: "Открывать остров по", expandHover: "Наведению", expandClick: "Клику", sData: "Данные", sRefresh: "Обновлять каждые", minutes: "{n} мин", sProviders: "Провайдеры", sGeneral: "Общие", sLanguage: "Язык", langAuto: "Как в системе", sStartup: "Запускать при входе",
    modeTaskbar: "Панель задач", dragHint: "Совет: перетащите остров к верхнему краю или к боковой стороне любого монитора, или на панель задач.",
    sPosition: "Положение", anchor_top: "Сверху (остров)", anchor_left: "Левый край", anchor_right: "Правый край",
    unknownAccount: "Неизвестный аккаунт", inUse: "Используется", use: "Использовать", switching: "Переключение…", idle: "Давно не использовался — переключитесь, чтобы увидеть расход", staleAgo: "обновлено {t} назад", switched: "{provider}: теперь используется {to}", switchedAuto: "Лимит {provider} исчерпан — переключено на {to}", sAccounts: "Аккаунты", addAccount: "Добавить аккаунт", addHint: "Войдите в открывшемся окне терминала. Аккаунт появится здесь, когда вы закончите.", alias: "Псевдоним", remove: "Удалить", activeCantRemove: "Сначала переключитесь на другой аккаунт, затем удалите этот.", cliMissing: "CLI Claude Code или Codex не найден ни в Windows, ни в WSL.", autoSwitch: "Переключать автоматически", switchAt: "Переключать при",
    descDisplay: "Где находится остров и как он открывается.", descAccounts: "Ваши аккаунты Claude и Codex и когда между ними переключаться.", descData: "Как часто проверяется расход и для каких инструментов.", descGeneral: "Язык и автозапуск.",
    rateLimited: "Сервис учёта занят — повторим через несколько минут",
    sUpdates: "Обновления", autoUpdate: "Обновлять автоматически", checkUpdates: "Проверить обновления", checking: "Проверка…", upToDate: "У вас последняя версия", updateAvailable: "Доступна версия {v}", installUpdate: "Установить и перезапустить", updating: "Обновление до {v}…", versionN: "Версия {v}",
   
    resetFull: "Полный сброс", resetSession: "Сброс сессии", resetUsableNow: "доступен сейчас", resetExpires: "истекает {d}", resetNext: "следующий через {t}", resetsTitle: "Сбросы лимита",
    resetNextExpires: "ближайший истекает {d}",
    sCompact: "Свёрнутый вид", compactNames: "Названия", compactMinimal: "Минимальный",
    updatedTo: "Обновлено до v{v}",
  },
};

let systemLocale = null;

/** The OS display language, from the backend; it wins over the webview's own guess. */
export function setSystemLocale(tag) {
  systemLocale = tag || null;
}

function pickLocale() {
  const candidates = [systemLocale, ...(navigator.languages?.length ? navigator.languages : [navigator.language])];
  for (const tag of candidates.filter(Boolean)) {
    const base = tag.toLowerCase().split("-")[0];
    if (MESSAGES[base]) return { tag, lang: base };
  }
  return { tag: "en", lang: "en" };
}

export const LANGUAGES = {
  en: "English", pt: "Português", es: "Español", fr: "Français", de: "Deutsch",
  it: "Italiano", ja: "日本語", zh: "中文", ko: "한국어", ru: "Русский",
};

export let LOCALE = "en";
let LANG = "en";
let DAY, HOUR, MIN;

/** "auto" follows the system; otherwise one of LANGUAGES. */
export function setLanguage(pref = "auto") {
  ({ tag: LOCALE, lang: LANG } = MESSAGES[pref] ? { tag: pref, lang: pref } : pickLocale());
  document.documentElement.lang = LOCALE;
  const unit = (u) => new Intl.NumberFormat(LOCALE, { style: "unit", unit: u, unitDisplay: "narrow" });
  [DAY, HOUR, MIN] = [unit("day"), unit("hour"), unit("minute")];
}
setLanguage();

export function t(key, vars = {}) {
  const msg = MESSAGES[LANG][key] ?? MESSAGES.en[key] ?? key;
  return msg.replace(/\{(\w+)\}/g, (_, k) => vars[k] ?? "");
}


/** Compact duration such as "3d 4h" or "4h 47min", formatted for the locale. */
export function duration(totalMinutes) {
  const d = Math.floor(totalMinutes / 1440);
  const h = Math.floor((totalMinutes % 1440) / 60);
  const m = totalMinutes % 60;
  if (d) return [DAY.format(d), h && HOUR.format(h)].filter(Boolean).join(" ");
  if (h) return [HOUR.format(h), m && MIN.format(m)].filter(Boolean).join(" ");
  return MIN.format(Math.max(1, m));
}

/** Short localized date, e.g. "Oct 22". */
export function shortDate(iso) {
  return new Date(iso).toLocaleDateString(LOCALE, { month: "short", day: "numeric" });
}

/**
 * Merges identical resets (same kind, title and usability) into one entry with a total count
 * and the soonest expiry, so an account holding several doesn't list them one by one.
 */
export function groupResets(resets) {
  const groups = new Map();
  for (const r of resets ?? []) {
    const key = `${r.kind}|${r.title ?? ""}|${r.usable}`;
    const g = groups.get(key);
    if (!g) {
      groups.set(key, { ...r, entries: 1 });
      continue;
    }
    g.count += r.count;
    g.entries += 1;
    if (r.expires_at && (!g.expires_at || r.expires_at < g.expires_at)) g.expires_at = r.expires_at;
    if (r.next_available_at && (!g.next_available_at || r.next_available_at < g.next_available_at)) g.next_available_at = r.next_available_at;
  }
  // Usable first, then the ones expiring soonest.
  return [...groups.values()].sort((a, b) => (b.usable - a.usable) || String(a.expires_at ?? "~").localeCompare(String(b.expires_at ?? "~")));
}

/** One line describing a usage-limit reset (or a group of identical ones). */
export function resetText(r) {
  const name = r.title || t(r.kind === "session" ? "resetSession" : "resetFull");
  const parts = [r.count > 1 ? `${name} ×${r.count}` : name];
  if (r.usable) parts.push(t("resetUsableNow"));
  if (r.expires_at) parts.push(t(r.entries > 1 ? "resetNextExpires" : "resetExpires", { d: shortDate(r.expires_at) }));
  if (r.next_available_at && !r.count) {
    const mins = Math.max(1, Math.round((new Date(r.next_available_at) - Date.now()) / 60000));
    parts.push(t("resetNext", { t: duration(mins) }));
  }
  return parts.join(" · ");
}

export function resetsIn(iso) {
  const mins = Math.round((new Date(iso) - Date.now()) / 60000);
  return mins <= 0 ? t("resetsNow") : t("resetsIn", { t: duration(mins) });
}

export function windowLabel(w) {
  if (w.kind !== "window") return t(w.kind);
  return w.window_seconds ? `${t("limit")} · ${duration(Math.round(w.window_seconds / 60))}` : t("limit");
}

const CLI_NAME = { Claude: "Claude Code", Codex: "Codex" };

export function errorText(provider, code) {
  const vars = { provider: CLI_NAME[provider] ?? provider };
  if (code === "token_expired" || code === "token_invalid" || code === "bad_response") return t(code, vars);
  if (code === "idle") return t("idle");
  if (code === "rate_limited") return t("rateLimited");
  if (code === "active") return t("activeCantRemove");
  if (code === "CLI not found") return t("cliMissing");
  const http = /^http_(\d+)$/.exec(code);
  return http ? t("http", { code: http[1] }) : code;
}

export const creditsText = (v) => t("credits", { v: v === "unlimited" ? t("unlimited") : v });

export function time(iso) {
  return new Date(iso).toLocaleTimeString(LOCALE, { hour: "2-digit", minute: "2-digit" });
}

/** Fills static markup: elements with data-i18n get text, data-i18n-title get a tooltip. */
export function translateDom(root = document) {
  root.querySelectorAll("[data-i18n]").forEach((el) => (el.textContent = t(el.dataset.i18n)));
  root.querySelectorAll("[data-i18n-title]").forEach((el) => (el.title = t(el.dataset.i18nTitle)));
}
