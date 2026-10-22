// Translations follow the system language (WebView2 reports the Windows display language).
// Missing languages and keys fall back to English.
const MESSAGES = {
  en: {
    sUsage: "Usage",
    descUsage: "Tokens by account, repository and model, read from Claude Code and the Codex CLI on this PC.",
    usagePeriod: "Period",
    usageDays: "Last {n} days",
    usageHow: "Read from the CLIs' own files here: model names, token counts, times and folders, never what was written. Repository names stay on this PC. Codex sessions name their account; Claude use is credited to the account SlopBar saw signed in at the time, so it is inferred, and time SlopBar was not running stays unattributed. Use in WSL, on other machines or on the web is not included.",
    usageTotal: "{v} tokens",
    usageUnattributedTotal: "{v} unattributed",
    usageSplit: "Input {i} · Output {o} · Cached {c}",
    usageModelMix: "Model mix",
    usageTopRepos: "Top repositories",
    usageTopModels: "Top models",
    usageHeatmap: "Busiest hours (local time)",
    usageHeatmapAll: "All accounts",
    usageNone: "No use on this PC in this period",
    usageLoading: "Reading the CLIs' files…",
    usageUnattributed: "Unattributed",
    usageUnattributedHint: "Used while SlopBar was not watching which account was signed in",
    usageInferred: "Inferred from the account signed in",
    usageMixed: "Partly inferred from the account signed in",
    usageUnknown: "Account not in SlopBar",
    sExportCsv: "Export",
    exportCsv: "Export CSV files",
    exportCsvHint: "Weekly usage history, this breakdown by day, and the subscription report, for a spreadsheet. The files hold account emails and repository names.",
    exportCsvButton: "Export",
    exportCsvSaving: "Exporting…",
    exportCsvSaved: "Saved {n} files to {path}",
    exportCsvFailed: "Could not write there. Use a full folder path you can write to.",
    sDiagnostics: "Diagnostics",
    crashRecords: "Record crashes",
    autoMonitoring: "Connect monitoring automatically",
    autoMonitoringHint: "When an account without monitoring has been in use for 3 minutes, SlopBar opens its monitoring sign-in so it stays current after you switch away. One approval in your browser; your active account stays the same.",
    monitorAutoTitle: "Finish connecting monitoring for {name}",
    monitorAutoBody: "Approve the sign-in in your browser so SlopBar can keep reading this account after you switch away. Your active account stays the same.",
    crashRecordsHint: "Off by default. Keeps where in SlopBar's code a crash happened (file and line, never its message) in the local history above, so exports include it. Nothing is sent anywhere.",
    sHealth: "Health",
    descHealth: "SlopBar checks its own sign-ins, readings, sharing and services. Each problem comes with the fix.",
    healthHow: "Checked from what SlopBar already knows; nothing extra is sent to any provider.",
    healthFixRefresh: "Refresh", healthFixRestart: "Restart SlopBar", healthFixOpen: "Open",
    healthIssues: "Health: {n} to fix",
    healthRestarting: "Restarting SlopBar…",
    exportDiagnostics: "Export diagnostics",
    diagnosticsHint: "Local history: up to 7 days or 10,000 events. Reports include anonymous account numbers and usage, but no credentials or account identities. Nothing is uploaded automatically.",
    diagnosticsExporting: "Exporting diagnostics…",
    diagnosticsSaved: "Diagnostics saved: {path}",
    diagnosticsFailed: "Could not export diagnostics. Check disk space and file access.",
    monitorDisconnect: "Disconnect monitoring", monitorDisconnected: "Monitoring disconnected and session revoked.",
    monitorRevokeFailed: "Monitoring disconnected locally. Server revocation was not confirmed; revoke this session in your provider account settings.",
    monitor_uncertain: "Renewal could not be confirmed after two attempts. Reconnect monitoring to continue.",
    monitorConnect: "Connect monitoring", monitorReconnect: "Reconnect monitoring",
    monitorConnected: "Independent monitoring connected", monitorPending: "Finish the monitoring sign-in in the opened terminal. Your active account stays the same.",
    monitor_setup: "Connect monitoring once to keep this account's usage available while idle.",
    monitor_reconnect: "Monitoring sign-in needs reconnecting. No account use is required.",
    monitor_storage: "Could not save or read the monitoring login. Check available disk space and file access.",
    monitor_network: "Monitoring service unavailable — retrying shortly.",
    monitor_wrong_account: "That sign-in belongs to a different account or organization. Sign in to the account selected here.",
    monitor_shared_login: "That login is shared with a CLI. Use a fresh monitoring sign-in.",
    monitor_login_failed: "Monitoring sign-in did not complete. Try connecting again.",
    monitor_busy: "Another monitoring action is in progress for this account.",
    lastObserved: "Last confirmed {t} ago", lastReading: "Last reading", usageUnknown: "Usage unavailable",
    manualUpdates: "Manual updates only for this build.",
    used: "used", modelUsageUnavailable: "Model-specific usage unavailable from this source",
    resetUnknown: "Reset · type unknown", resetWeekly: "Weekly reset", resetNotUsableNow: "not usable now",

    title: "Plan usage", loading: "Loading…", noAccounts: "No accounts",
    noCredentials: "No Claude Code or Codex credentials found on Windows or WSL.",
    searching: "Looking for accounts…", refresh: "Refresh",
    session: "Session (5h)", weekly: "Weekly", weekly_opus: "Weekly · Opus", weekly_sonnet: "Weekly · Sonnet", limit: "Limit",
    resetsIn: "Resets in {t}", resetsNow: "Resets now", credits: "Credits: {v}", unlimited: "Unlimited",
    windowNotStarted: "Not started · begins on first use",
    resetTimeUnavailable: "Reset time unavailable", lastResetAt: "Last known reset: {d}", resetAwaitingRefresh: "Reset time passed · awaiting refresh ({d})",
    token_expired: "Token expired — open {provider} to renew it",
    token_invalid: "Invalid token — open {provider} to sign in again",
    bad_response: "Unexpected response", http: "HTTP error {code}",
    quit: "Quit", close: "Close",
    settings: "Settings", sData: "Data", sRefresh: "Refresh every", minutes: "{n} min", sProviders: "Providers", sGeneral: "General", sLanguage: "Language", langAuto: "System default", sStartup: "Launch at login",
    unknownAccount: "Unknown account", inUse: "In use", inUseOn: "In use on {list}", seenOn: "{machine}: seen {t} ago", usedOutside: "Used outside this PC", usedOutsideHint: "Its usage is rising while it isn't signed in here or on any computer sharing its status: another computer, the web or the desktop app.", sPresence: "Other computers", sharePresence: "Share in-use status via folder", sharePresenceHint: "Shows which accounts are in use on your other computers. Each SlopBar writes a small file to a folder you sync (OneDrive, Dropbox, Google Drive, iCloud Drive, Box) with its computer name and anonymous account codes only, never emails or sign-ins.", presenceOff: "Off", presenceOther: "Custom folder…", presenceViaRclone: "{provider} (via rclone)", presenceRcloneHint: "Install rclone and run “rclone config” to connect any cloud without a sync app.", presenceFolderPlaceholder: "Folder path, e.g. D:\\Sync", presenceFolderInvalid: "That folder doesn't exist.", presenceSave: "Use folder", use: "Use", switching: "Switching…", idle: "Not used recently — switch to it to see its usage", staleAgo: "updated {t} ago", switched: "{provider}: now using {to}", switchedAuto: "{provider} limit reached — switched to {to}", sAccounts: "Accounts", addAccount: "Add account", addHint: "Sign in in the terminal window that opened. The account shows up here when you're done.", alias: "Nickname", remove: "Remove", activeCantRemove: "Switch to another account before removing this one.", cliMissing: "Couldn't find the Claude Code or Codex CLI on Windows or WSL.", autoSwitch: "Switch automatically", switchAt: "Switch at",
    descAccounts: "Your Claude and Codex accounts, and when to switch between them.", descData: "How often usage is checked, and for which tools.", descGeneral: "Language and startup.",
    rateLimited: "The usage service is busy — trying again in a few minutes",
    sUpdates: "Updates", autoUpdate: "Update automatically", checkUpdates: "Check for updates", checking: "Checking…", upToDate: "You're up to date", updateAvailable: "Version {v} is available", installUpdate: "Install and restart", updating: "Updating to {v}…", versionN: "Version {v}",
    resetFull: "Full reset", resetSession: "Session reset", resetUsableNow: "usable now", resetExpires: "expires {d}", resetNext: "next in {t}", resetsTitle: "Limit resets",
    resetNextExpires: "next expires {d}",
    updatedTo: "Updated to v{v}",
    sNotifications: "Notifications", descNotifications: "What SlopBar tells you in Windows notifications.", notifyUsage: "Usage alerts", notifyUsageHint: "When a limit reaches 75% and 90%, and when it runs out.", notifyResets: "Limits and resets", notifyResetsHint: "When a limit you ran out of is back, and when an account receives a limit reset.", notifySwitches: "Automatic switches", notifySwitchesHint: "When SlopBar switches accounts for you.", nUsageTitle: "{provider} at {p}%", nLimitTitle: "{provider} limit reached", nLimitHasReset: "You have a limit reset you can use now.", nBackTitle: "{provider} is available again", nBackBody: "{window} limit reset · {account}", nGrantTitle: "New {provider} limit reset", notifyRenewals: "Plan renewals", notifyRenewalsHint: "When a plan that looks unneeded renews within 3 days, and when any paid plan renews within a day.", nRenewTitle: "{provider} plan renews in {t}", nRenewWasteHint: "Not needed? Cancel it on the provider's billing page.", perMonth: "{v}/month", nRunsOutTitle: "{provider} runs out ~{at}", nRunsOutBody: "Weekly limit · {account} · resets in {t}", runsOut: "Runs out ~{at}",
    nProCutTitle: "Codex Pro 200 allowance drops after {d}", nProCutBody: "At recent use these would hit the weekly limit: {accounts}", subProCut: "Pro 200’s Codex allowance drops after {d} (to about half, estimated). At its recent peak of {p}% this account would reach {q}% of the new weekly limit.",
    notifyDigest: "Monday digest", notifyDigestHint: "Monday mornings: what your plans cost, what you could save, renewals this week and accounts that ran out.",
    nDigestTitle: "Your plans this week", nDigestSpend: "{v}/month on plans", nDigestUnpriced: "{n} without a price", nDigestSave: "could save {v}",
    nDigestRenewing: "Renewing: {list}", nDigestRanOut: "Ran out last week: {list}", nDigestPool: "{provider}: usage fits in {n} × {unit}; you hold {held}",
    sCalendar: "Renewal calendar", calendarKeep: "Keep a renewal calendar", calendarKeepHint: "An .ics file with each paid plan's next renewal, and a reminder 3 days ahead for plans that look unneeded. Subscribe to it from Outlook or Google Calendar; SlopBar updates it after each refresh.",
    calendarExport: "Export now", calendarSaved: "Saved {n} renewals to {path}", calendarFailed: "Couldn't write the calendar file.", calendarPathInvalid: "Choose an .ics file in a folder that exists.",
    notifyUnexpected: "Unexpected use", notifyUnexpectedHint: "When an account is used while none of your computers sharing in-use status has it signed in: the web or mobile app, or someone else. Needs sharing on every computer.",
    nUnexpectedTitle: "{provider} used outside your computers", nUnexpectedBody: "{account} is being used, but none of your computers has it signed in: the web or mobile app, or someone else. If it isn't you, review its sessions and change the password.", nUnexpectedReview: "Review in SlopBar", nUnexpectedSecurity: "Open security page",
    unexpectedSeen: "Unexpected use seen {n} times this week", unexpectedSeenOnce: "Unexpected use seen once this week", unexpectedHint: "Used outside your computers — web/mobile app or someone else. Last seen {t} ago. If you also use it on the web or a phone, tick “Also used on web/phone”.",
    webUse: "Also used on web/phone", webUseHint: "Its use away from your computers is expected, so SlopBar doesn't report it as unexpected.",
    sSubscriptions: "Subscriptions", descSubscriptions: "Which paid plans run out every week and which sit idle, and when each one renews.",
    subDatesHow: "Codex renewal dates come from ChatGPT's own account record. Claude only reports when a plan started, so its renewal is counted a month at a time from that day. If a date is wrong, pick the right one under Next billing date; Reset goes back to the provider's or the counted date.",
    subHowJudged: "Verdicts use completed weekly limits SlopBar watched for at least a day across three days. A week it did not watch counts as unknown, never as unused. It takes two such weeks to judge a plan, and four for a confident one. Connect monitoring on idle accounts so their weeks count.",
    subNoCancel: "SlopBar cannot cancel or change a plan. Manage plan opens the provider's billing page, where you cancel it yourself.",
    subVerdict_insufficient_data: "Not enough history", subVerdict_exhausted_weekly: "Runs out weekly", subVerdict_healthy: "Well used",
    subVerdict_underused: "Mostly idle", subVerdict_unused: "Unused",
    subInsufficient: "Watching: {n} of {needed} weeks needed so far (tracked {days} days).",
    subRecent: "Last {n} weeks: reached the limit in {hit}, used {avg} at its weekly peak on average.",
    subModelCap: "{window} keeps reaching its limit.", subLowConfidence: "Few readings: treat this as a first impression.",
    subThisWeek: "This week: {p} used, resets in {t}.", subThisWeekFast: "This week: {p} used, resets in {t} — on pace to run out early.",
    subNoWeekly: "No weekly limit reported yet.",
    subRenews: "Renews {d} · in {n} days ({source})", subRenewsToday: "Renews today ({source})", subRenewsTomorrow: "Renews tomorrow, {d} ({source})", subRenewUnknown: "Renewal date not reported yet",
    subSourceEntered: "corrected by you", subSourceProvider: "from the provider", subSourceEstimated: "monthly from the plan's start",
    subProviderStale: "The sign-in last reported this plan paid until {d}; the next renewal is counted monthly from there.",
    subCostList: "{v}/mo", subCostEntered: "{v}/mo", subCostUnknown: "Cost unknown",
    subOrgManaged: "Paid by an organization: its admin manages this plan.", subFree: "Free plan: nothing to renew.", subPlanUnknown: "Plan not reported by the provider.",
    subStoreBilled: "Bought in an app store: cancel it in the App Store or Google Play, not on the website.",
    subAdviceUnused: "Renewing looks unneeded. To avoid the next charge, cancel before {d}; the plan keeps working until then.",
    subAdviceUnusedNoDate: "Renewing looks unneeded. Cancel before the next billing date to avoid the charge.",
    subAdviceUnderused: "A smaller plan would likely cover this usage.", subAdviceUnderusedMaybe: "Usage is low so far; check again after a few more weeks.",
    subAdviceExhausted: "This plan is fully used; it is not the one to cancel.",
    subLapsing: "Letting this plan lapse: automatic switching skips it after {d}.", chipLapsing: "Lapsing", chipCancelled: "Cancelled",
    subEnds: "Cancelled · ends {d} (from the provider)", subAdviceEnding: "Already cancelled: it will not charge again and keeps working until {d}.",
    subDelinquent: "The provider reports a failed payment for this plan.",
    subManage: "Manage plan", subNextBilling: "Next billing date", subResetDate: "Reset", subCorrectHint: "Wrong? Pick the right date; later renewals count on from it.", subMonthlyCost: "Monthly cost", subLettingLapse: "I'm cancelling this plan",
    subPaidCount: "{n} paid", subMonthlyTotal: "{v}/mo", subUnpriced: "{n} unpriced", subExhaustedCount: "{n} run out weekly",
    subUnusedCount: "{n} unused", subSavings: "could save {v}/mo",
    subPool: "Your usage fits in {n} × {unit} (+{spare} spare); you hold {held}", subPoolBasis: "busiest of {n} weeks + 20%",
    subPoolEstimate: "estimate", subPoolLocal: "includes weeks read from this PC's Codex history", subPoolInsufficient: "Pool size: not enough history yet ({n} of {needed} weeks)",
    subPoolUnweighted: "{n} plans of unknown size left out",
    subBlendedPercent: "{v} per 1% of a {unit} week used", subPerPercent: "{v} per 1% of the weekly limit used",
    subPerThread: "{v} per thread (this PC only)",
    subCredits: "Credits: {v}", subCreditsUnlimited: "Unlimited credits", subCreditsSpent: "spent {n} since {d}",
    subClaudeFree: "After a Claude plan ends the account falls back to Claude Free, which does not include Claude Code. Remove the account from SlopBar once its paid period is over.",
    subCodexFree: "After a ChatGPT plan ends the account falls back to ChatGPT Free, which still includes Codex with lower limits.",

    subLocalLast: "Last used here: {d}", subLocalToday: "Last used here: today", subLocalNever: "Not used on this PC since {d}",
    subLocalIdle: "{n} days idle", subLocalThreads: "{n} threads in 28 days", subLocalTokens: "{v} tokens",
    subIdleNow: "Weekly limit untouched: no use anywhere in the past 7 days.",
    subLocalBasis: "Based on local Codex history on this PC and the untouched weekly limit, not on weeks of SlopBar readings.",
    subLocalWeeks: "Includes weeks read from this PC's Codex history.",
    subLocalCaveat: "Use in WSL, on other machines or in the cloud is not seen locally.",
    subCodexLocal: "Codex rows also show this PC's own Codex history, which names the account behind each thread since {d}. Use in WSL, on other machines or in the cloud is not seen locally.",
  },
  pt: {
    sDiagnostics: "Diagnóstico",
    exportDiagnostics: "Exportar diagnóstico",
    diagnosticsHint: "Histórico local: até 7 dias ou 10.000 eventos. Relatórios incluem números anônimos de contas e uso, sem credenciais ou identidades. Nada é enviado automaticamente.",
    diagnosticsExporting: "Exportando diagnóstico…",
    diagnosticsSaved: "Diagnóstico salvo: {path}",
    diagnosticsFailed: "Não foi possível exportar. Verifique o espaço em disco e o acesso aos arquivos.",
    monitorDisconnect: "Desconectar monitoramento",
    monitorDisconnected: "Monitoramento desconectado e sessão revogada.",
    monitorRevokeFailed: "Monitoramento desconectado neste computador. A revogação no servidor não foi confirmada; revogue esta sessão nas configurações da sua conta.",
    monitor_uncertain: "Não foi possível confirmar a renovação após duas tentativas. Reconecte o monitoramento para continuar.",
    monitorConnect: "Conectar monitoramento",
    monitorReconnect: "Reconectar monitoramento",
    monitorConnected: "Monitoramento independente conectado",
    monitorPending: "Conclua o login de monitoramento no terminal aberto. Sua conta ativa permanece a mesma.",
    monitor_setup: "Conecte o monitoramento uma vez para acompanhar o uso desta conta mesmo quando estiver inativa.",
    monitor_reconnect: "Reconecte o login de monitoramento. Não é necessário usar a conta.",
    monitor_storage: "Não foi possível salvar ou ler o login de monitoramento. Verifique o espaço em disco e o acesso aos arquivos.",
    monitor_network: "Serviço de monitoramento indisponível — nova tentativa em breve.",
    monitor_wrong_account: "Esse login pertence a outra conta ou organização. Entre na conta selecionada aqui.",
    monitor_shared_login: "Esse login é compartilhado com uma CLI. Faça um novo login para monitoramento.",
    monitor_login_failed: "O login de monitoramento não foi concluído. Tente conectar novamente.",
    monitor_busy: "Outra ação de monitoramento está em andamento para esta conta.",
    lastObserved: "Última confirmação há {t}",
    lastReading: "Última leitura",
    usageUnknown: "Uso indisponível",
    manualUpdates: "Esta versão só recebe atualizações manuais.",
    title: "Uso dos planos", loading: "Carregando…", noAccounts: "Nenhuma conta",
    noCredentials: "Nenhuma credencial do Claude Code ou Codex no Windows ou WSL.",
    searching: "Procurando contas…", refresh: "Atualizar",
    session: "Sessão (5h)", weekly: "Semana", weekly_opus: "Semana · Opus", weekly_sonnet: "Semana · Sonnet", limit: "Limite",
    resetsIn: "Reinicia em {t}", resetsNow: "Reinicia agora", credits: "Créditos: {v}", unlimited: "Ilimitado",
    windowNotStarted: "Não iniciado · começa no primeiro uso",
    resetTimeUnavailable: "Horário de reinício indisponível", lastResetAt: "Último reinício conhecido: {d}", resetAwaitingRefresh: "Horário de reinício passou · aguardando atualização ({d})",
    token_expired: "Token expirado — abra o {provider} para renovar",
    token_invalid: "Token inválido — abra o {provider} para entrar de novo",
    bad_response: "Resposta inesperada", http: "Erro HTTP {code}",
    quit: "Sair", close: "Fechar",
    settings: "Configurações", sData: "Dados", sRefresh: "Atualizar a cada", minutes: "{n} min", sProviders: "Provedores", sGeneral: "Geral", sLanguage: "Idioma", langAuto: "Padrão do sistema", sStartup: "Iniciar com o Windows",
    unknownAccount: "Conta desconhecida", inUse: "Em uso", use: "Usar", switching: "Trocando…", idle: "Sem uso recente — troque para ela para ver o uso", staleAgo: "atualizado há {t}", switched: "{provider}: usando {to}", switchedAuto: "Limite do {provider} atingido — trocado para {to}", sAccounts: "Contas", addAccount: "Adicionar conta", addHint: "Entre na janela de terminal que abriu. A conta aparece aqui quando você terminar.", alias: "Apelido", remove: "Remover", activeCantRemove: "Troque para outra conta antes de remover esta.", cliMissing: "Não encontrei o Claude Code nem o Codex no Windows ou no WSL.", autoSwitch: "Trocar automaticamente", switchAt: "Trocar em",
    descAccounts: "Suas contas do Claude e do Codex, e quando trocar entre elas.", descData: "Com que frequência o uso é consultado, e de quais ferramentas.", descGeneral: "Idioma e inicialização.",
    rateLimited: "O serviço de uso está ocupado — tentando de novo em alguns minutos",
    sUpdates: "Atualizações", autoUpdate: "Atualizar automaticamente", checkUpdates: "Procurar atualizações", checking: "Procurando…", upToDate: "Você está na versão mais recente", updateAvailable: "A versão {v} está disponível", installUpdate: "Instalar e reiniciar", updating: "Atualizando para {v}…", versionN: "Versão {v}",
    resetFull: "Reset completo", resetSession: "Reset da sessão", resetUsableNow: "utilizável agora", resetExpires: "expira em {d}", resetNext: "próximo em {t}", resetsTitle: "Resets de limite",
    resetNextExpires: "próximo expira em {d}",
    updatedTo: "Atualizado para a v{v}",
    sNotifications: "Notificações", descNotifications: "O que o SlopBar avisa nas notificações do Windows.", notifyUsage: "Alertas de uso", notifyUsageHint: "Quando um limite chega a 75% e 90%, e quando acaba.", notifyResets: "Limites e resets", notifyResetsHint: "Quando um limite esgotado volta, e quando uma conta recebe um reset de limite.", notifySwitches: "Trocas automáticas", notifySwitchesHint: "Quando o SlopBar troca de conta por você.", nUsageTitle: "{provider} em {p}%", nLimitTitle: "Limite do {provider} atingido", nLimitHasReset: "Você tem um reset de limite para usar agora.", nBackTitle: "{provider} disponível de novo", nBackBody: "Limite {window} reiniciado · {account}", nGrantTitle: "Novo reset de limite do {provider}", notifyRenewals: "Renovações de plano", notifyRenewalsHint: "Quando um plano que parece desnecessário renova em 3 dias, e quando qualquer plano pago renova em um dia.", nRenewTitle: "Plano {provider} renova em {t}", nRenewWasteHint: "Não precisa dele? Cancele na página de cobrança do provedor.", perMonth: "{v}/mês", nRunsOutTitle: "{provider} acaba ~{at}", nRunsOutBody: "Limite semanal · {account} · reinicia em {t}", runsOut: "Acaba ~{at}",
    nProCutTitle: "Cota do Codex no Pro 200 cai depois de {d}", nProCutBody: "Com o uso recente, estas atingiriam o limite semanal: {accounts}", subProCut: "A cota do Codex no Pro 200 cai depois de {d} (para cerca da metade, estimado). No pico recente de {p}%, esta conta chegaria a {q}% do novo limite semanal.",
  },
  es: {
    sDiagnostics: "Diagnóstico",
    exportDiagnostics: "Exportar diagnóstico",
    diagnosticsHint: "Historial local: hasta 7 días o 10.000 eventos. Incluye cuentas anónimas y uso, sin credenciales ni identidades. No se envía nada automáticamente.",
    diagnosticsExporting: "Exportando diagnóstico…",
    diagnosticsSaved: "Diagnóstico guardado: {path}",
    diagnosticsFailed: "No se pudo exportar. Comprueba el espacio y el acceso a los archivos.",
    monitorDisconnect: "Desconectar supervisión",
    monitorDisconnected: "Supervisión desconectada y sesión revocada.",
    monitorRevokeFailed: "Supervisión desconectada en este equipo. No se confirmó la revocación en el servidor; revoca esta sesión en los ajustes de tu cuenta.",
    monitor_uncertain: "No se pudo confirmar la renovación tras dos intentos. Reconecta la supervisión para continuar.",
    monitorConnect: "Conectar supervisión",
    monitorReconnect: "Reconectar supervisión",
    monitorConnected: "Supervisión independiente conectada",
    monitorPending: "Completa el inicio de sesión para supervisión en el terminal abierto. Tu cuenta activa seguirá siendo la misma.",
    monitor_setup: "Conecta la supervisión una vez para consultar el uso de esta cuenta aunque esté inactiva.",
    monitor_reconnect: "Es necesario reconectar la sesión de supervisión. No hace falta usar la cuenta.",
    monitor_storage: "No se pudo guardar o leer la sesión de supervisión. Comprueba el espacio en disco y el acceso a los archivos.",
    monitor_network: "Servicio de supervisión no disponible — se reintentará pronto.",
    monitor_wrong_account: "Esta sesión pertenece a otra cuenta u organización. Inicia sesión en la cuenta seleccionada aquí.",
    monitor_shared_login: "Esta sesión se comparte con una CLI. Inicia una sesión nueva para la supervisión.",
    monitor_login_failed: "No se completó el inicio de sesión para supervisión. Intenta conectar de nuevo.",
    monitor_busy: "Hay otra acción de supervisión en curso para esta cuenta.",
    lastObserved: "Última confirmación hace {t}",
    lastReading: "Última lectura",
    usageUnknown: "Uso no disponible",
    manualUpdates: "Esta versión solo admite actualizaciones manuales.",
    title: "Uso de los planes", loading: "Cargando…", noAccounts: "Sin cuentas",
    noCredentials: "No se encontraron credenciales de Claude Code o Codex en Windows ni WSL.",
    searching: "Buscando cuentas…", refresh: "Actualizar",
    session: "Sesión (5 h)", weekly: "Semana", weekly_opus: "Semana · Opus", weekly_sonnet: "Semana · Sonnet", limit: "Límite",
    resetsIn: "Se reinicia en {t}", resetsNow: "Se reinicia ahora", credits: "Créditos: {v}", unlimited: "Ilimitado",
    windowNotStarted: "Sin iniciar · empieza con el primer uso",
    resetTimeUnavailable: "Hora de reinicio no disponible", lastResetAt: "Último reinicio conocido: {d}", resetAwaitingRefresh: "La hora de reinicio pasó · esperando actualización ({d})",
    token_expired: "Token caducado: abre {provider} para renovarlo",
    token_invalid: "Token no válido: abre {provider} para volver a iniciar sesión",
    bad_response: "Respuesta inesperada", http: "Error HTTP {code}",
    quit: "Salir", close: "Cerrar",
    settings: "Configuración", sData: "Datos", sRefresh: "Actualizar cada", minutes: "{n} min", sProviders: "Proveedores", sGeneral: "General", sLanguage: "Idioma", langAuto: "Predeterminado del sistema", sStartup: "Iniciar con Windows",
    unknownAccount: "Cuenta desconocida", inUse: "En uso", use: "Usar", switching: "Cambiando…", idle: "Sin uso reciente: cámbiate a ella para ver su uso", staleAgo: "actualizado hace {t}", switched: "{provider}: usando {to}", switchedAuto: "Límite de {provider} alcanzado: cambiado a {to}", sAccounts: "Cuentas", addAccount: "Añadir cuenta", addHint: "Inicia sesión en la ventana de terminal que se abrió. La cuenta aparecerá aquí al terminar.", alias: "Apodo", remove: "Quitar", activeCantRemove: "Cambia a otra cuenta antes de quitar esta.", cliMissing: "No se encontró la CLI de Claude Code ni de Codex en Windows ni en WSL.", autoSwitch: "Cambiar automáticamente", switchAt: "Cambiar al",
    descAccounts: "Tus cuentas de Claude y Codex, y cuándo cambiar entre ellas.", descData: "Cada cuánto se consulta el uso y de qué herramientas.", descGeneral: "Idioma e inicio.",
    rateLimited: "El servicio de uso está ocupado: se reintentará en unos minutos",
    sUpdates: "Actualizaciones", autoUpdate: "Actualizar automáticamente", checkUpdates: "Buscar actualizaciones", checking: "Buscando…", upToDate: "Tienes la última versión", updateAvailable: "La versión {v} está disponible", installUpdate: "Instalar y reiniciar", updating: "Actualizando a {v}…", versionN: "Versión {v}",
    resetFull: "Reinicio completo", resetSession: "Reinicio de sesión", resetUsableNow: "utilizable ahora", resetExpires: "caduca el {d}", resetNext: "próximo en {t}", resetsTitle: "Reinicios de límite",
    resetNextExpires: "el próximo caduca el {d}",
    updatedTo: "Actualizado a la v{v}",
    sNotifications: "Notificaciones", descNotifications: "Lo que SlopBar te avisa en las notificaciones de Windows.", notifyUsage: "Alertas de uso", notifyUsageHint: "Cuando un límite llega al 75% y al 90%, y cuando se agota.", notifyResets: "Límites y reinicios", notifyResetsHint: "Cuando vuelve un límite agotado y cuando una cuenta recibe un reinicio de límite.", notifySwitches: "Cambios automáticos", notifySwitchesHint: "Cuando SlopBar cambia de cuenta por ti.", nUsageTitle: "{provider} al {p}%", nLimitTitle: "Límite de {provider} alcanzado", nLimitHasReset: "Tienes un reinicio de límite que puedes usar ahora.", nBackTitle: "{provider} vuelve a estar disponible", nBackBody: "Límite {window} reiniciado · {account}", nGrantTitle: "Nuevo reinicio de límite de {provider}", notifyRenewals: "Renovaciones de plan", notifyRenewalsHint: "Cuando un plan que parece innecesario se renueva en 3 días, y cuando cualquier plan de pago se renueva en un día.", nRenewTitle: "El plan de {provider} se renueva en {t}", nRenewWasteHint: "¿No lo necesitas? Cancélalo en la página de facturación del proveedor.", perMonth: "{v}/mes", nRunsOutTitle: "{provider} se agota ~{at}", nRunsOutBody: "Límite semanal · {account} · se reinicia en {t}", runsOut: "Se agota ~{at}",
    nProCutTitle: "La cuota de Codex en Pro 200 baja después del {d}", nProCutBody: "Con el uso reciente, estas alcanzarían el límite semanal: {accounts}", subProCut: "La cuota de Codex en Pro 200 baja después del {d} (a la mitad aproximadamente, estimado). Con su pico reciente del {p}%, esta cuenta llegaría al {q}% del nuevo límite semanal.",
  },
  fr: {
    sDiagnostics: "Diagnostic",
    exportDiagnostics: "Exporter le diagnostic",
    diagnosticsHint: "Historique local : 7 jours ou 10 000 événements maximum. Comptes anonymes et utilisation, sans identifiants de connexion ni identité. Aucun envoi automatique.",
    diagnosticsExporting: "Exportation du diagnostic…",
    diagnosticsSaved: "Diagnostic enregistré : {path}",
    diagnosticsFailed: "Exportation impossible. Vérifiez l’espace disque et l’accès aux fichiers.",
    monitorDisconnect: "Déconnecter le suivi",
    monitorDisconnected: "Suivi déconnecté et session révoquée.",
    monitorRevokeFailed: "Suivi déconnecté sur cet ordinateur. La révocation sur le serveur n’a pas été confirmée ; révoquez cette session dans les paramètres de votre compte.",
    monitor_uncertain: "Le renouvellement n’a pas été confirmé après deux tentatives. Reconnectez le suivi pour continuer.",
    monitorConnect: "Connecter le suivi",
    monitorReconnect: "Reconnecter le suivi",
    monitorConnected: "Suivi indépendant connecté",
    monitorPending: "Terminez la connexion pour le suivi dans le terminal ouvert. Votre compte actif reste le même.",
    monitor_setup: "Connectez le suivi une fois pour consulter l’utilisation de ce compte même lorsqu’il est inactif.",
    monitor_reconnect: "La session de suivi doit être reconnectée. Vous n’avez pas besoin d’utiliser le compte.",
    monitor_storage: "Impossible d’enregistrer ou de lire la session de suivi. Vérifiez l’espace disque et l’accès aux fichiers.",
    monitor_network: "Service de suivi indisponible — nouvelle tentative bientôt.",
    monitor_wrong_account: "Cette connexion appartient à un autre compte ou une autre organisation. Connectez-vous au compte sélectionné ici.",
    monitor_shared_login: "Cette session est partagée avec une CLI. Créez une nouvelle connexion pour le suivi.",
    monitor_login_failed: "La connexion pour le suivi n’a pas abouti. Réessayez.",
    monitor_busy: "Une autre action de suivi est en cours pour ce compte.",
    lastObserved: "Dernière confirmation il y a {t}",
    lastReading: "Dernière mesure",
    usageUnknown: "Utilisation indisponible",
    manualUpdates: "Mises à jour manuelles uniquement pour cette version.",
    title: "Utilisation des forfaits", loading: "Chargement…", noAccounts: "Aucun compte",
    noCredentials: "Aucun identifiant Claude Code ou Codex trouvé sur Windows ou WSL.",
    searching: "Recherche des comptes…", refresh: "Actualiser",
    session: "Session (5 h)", weekly: "Semaine", weekly_opus: "Semaine · Opus", weekly_sonnet: "Semaine · Sonnet", limit: "Limite",
    resetsIn: "Réinitialisation dans {t}", resetsNow: "Réinitialisation imminente", credits: "Crédits : {v}", unlimited: "Illimité",
    windowNotStarted: "Pas commencé · démarre à la première utilisation",
    resetTimeUnavailable: "Heure de réinitialisation indisponible", lastResetAt: "Dernière réinitialisation connue : {d}", resetAwaitingRefresh: "Heure de réinitialisation passée · en attente d’actualisation ({d})",
    token_expired: "Jeton expiré — ouvrez {provider} pour le renouveler",
    token_invalid: "Jeton invalide — ouvrez {provider} pour vous reconnecter",
    bad_response: "Réponse inattendue", http: "Erreur HTTP {code}",
    quit: "Quitter", close: "Fermer",
    settings: "Paramètres", sData: "Données", sRefresh: "Actualiser toutes les", minutes: "{n} min", sProviders: "Fournisseurs", sGeneral: "Général", sLanguage: "Langue", langAuto: "Langue du système", sStartup: "Lancer au démarrage",
    unknownAccount: "Compte inconnu", inUse: "Utilisé", use: "Utiliser", switching: "Changement…", idle: "Pas utilisé récemment — passez-y pour voir son utilisation", staleAgo: "mis à jour il y a {t}", switched: "{provider} : compte {to} utilisé", switchedAuto: "Limite {provider} atteinte — passage à {to}", sAccounts: "Comptes", addAccount: "Ajouter un compte", addHint: "Connectez-vous dans la fenêtre de terminal qui s’est ouverte. Le compte apparaîtra ici ensuite.", alias: "Surnom", remove: "Supprimer", activeCantRemove: "Passez à un autre compte avant de supprimer celui-ci.", cliMissing: "CLI Claude Code ou Codex introuvable sous Windows ou WSL.", autoSwitch: "Changer automatiquement", switchAt: "Changer à",
    descAccounts: "Vos comptes Claude et Codex, et quand passer de l’un à l’autre.", descData: "À quelle fréquence l’utilisation est vérifiée, et pour quels outils.", descGeneral: "Langue et démarrage.",
    rateLimited: "Le service d’utilisation est occupé — nouvel essai dans quelques minutes",
    sUpdates: "Mises à jour", autoUpdate: "Mettre à jour automatiquement", checkUpdates: "Rechercher des mises à jour", checking: "Recherche…", upToDate: "Vous êtes à jour", updateAvailable: "La version {v} est disponible", installUpdate: "Installer et redémarrer", updating: "Mise à jour vers {v}…", versionN: "Version {v}",
    resetFull: "Réinitialisation complète", resetSession: "Réinitialisation de session", resetUsableNow: "utilisable maintenant", resetExpires: "expire le {d}", resetNext: "prochaine dans {t}", resetsTitle: "Réinitialisations de limite",
    resetNextExpires: "le prochain expire le {d}",
    updatedTo: "Mis à jour vers la v{v}",
    sNotifications: "Notifications", descNotifications: "Ce que SlopBar vous signale dans les notifications Windows.", notifyUsage: "Alertes d’utilisation", notifyUsageHint: "Quand une limite atteint 75 % et 90 %, et quand elle est épuisée.", notifyResets: "Limites et réinitialisations", notifyResetsHint: "Quand une limite épuisée revient, et quand un compte reçoit une réinitialisation.", notifySwitches: "Changements automatiques", notifySwitchesHint: "Quand SlopBar change de compte pour vous.", nUsageTitle: "{provider} à {p} %", nLimitTitle: "Limite {provider} atteinte", nLimitHasReset: "Vous avez une réinitialisation utilisable maintenant.", nBackTitle: "{provider} est de nouveau disponible", nBackBody: "Limite {window} réinitialisée · {account}", nGrantTitle: "Nouvelle réinitialisation {provider}", notifyRenewals: "Renouvellements d’abonnement", notifyRenewalsHint: "Quand un abonnement qui semble inutile se renouvelle sous 3 jours, et quand tout abonnement payant se renouvelle sous un jour.", nRenewTitle: "L’abonnement {provider} se renouvelle dans {t}", nRenewWasteHint: "Inutile ? Résiliez-le sur la page de facturation du fournisseur.", perMonth: "{v}/mois", nRunsOutTitle: "{provider} épuisé vers {at}", nRunsOutBody: "Limite hebdomadaire · {account} · réinitialisée dans {t}", runsOut: "Épuisé vers {at}",
    nProCutTitle: "Le quota Codex de Pro 200 baisse après le {d}", nProCutBody: "À l’usage récent, ces comptes atteindraient la limite hebdomadaire : {accounts}", subProCut: "Le quota Codex de Pro 200 baisse après le {d} (environ de moitié, estimation). À son pic récent de {p} %, ce compte atteindrait {q} % de la nouvelle limite hebdomadaire.",
  },
  de: {
    sDiagnostics: "Diagnose",
    exportDiagnostics: "Diagnose exportieren",
    diagnosticsHint: "Lokaler Verlauf: bis zu 7 Tage oder 10.000 Ereignisse. Anonyme Kontonummern und Nutzung, ohne Zugangsdaten oder Kontodaten. Kein automatischer Upload.",
    diagnosticsExporting: "Diagnose wird exportiert…",
    diagnosticsSaved: "Diagnose gespeichert: {path}",
    diagnosticsFailed: "Export fehlgeschlagen. Prüfen Sie Speicherplatz und Dateizugriff.",
    monitorDisconnect: "Überwachung trennen",
    monitorDisconnected: "Überwachung getrennt und Sitzung widerrufen.",
    monitorRevokeFailed: "Überwachung auf diesem Computer getrennt. Der Widerruf auf dem Server wurde nicht bestätigt; widerrufe diese Sitzung in deinen Kontoeinstellungen.",
    monitor_uncertain: "Die Erneuerung konnte nach zwei Versuchen nicht bestätigt werden. Verbinde die Überwachung erneut.",
    monitorConnect: "Überwachung verbinden",
    monitorReconnect: "Überwachung erneut verbinden",
    monitorConnected: "Unabhängige Überwachung verbunden",
    monitorPending: "Schließe die Anmeldung für die Überwachung im geöffneten Terminal ab. Dein aktives Konto bleibt gleich.",
    monitor_setup: "Verbinde die Überwachung einmal, um die Nutzung dieses Kontos auch bei Inaktivität abzurufen.",
    monitor_reconnect: "Die Anmeldung für die Überwachung muss erneuert werden. Du musst das Konto dazu nicht benutzen.",
    monitor_storage: "Die Anmeldung für die Überwachung konnte nicht gespeichert oder gelesen werden. Prüfe Speicherplatz und Dateizugriff.",
    monitor_network: "Überwachungsdienst nicht verfügbar — erneuter Versuch in Kürze.",
    monitor_wrong_account: "Diese Anmeldung gehört zu einem anderen Konto oder einer anderen Organisation. Melde dich beim hier ausgewählten Konto an.",
    monitor_shared_login: "Diese Sitzung wird mit einer CLI geteilt. Melde dich für die Überwachung neu an.",
    monitor_login_failed: "Die Anmeldung für die Überwachung wurde nicht abgeschlossen. Versuche es erneut.",
    monitor_busy: "Für dieses Konto läuft bereits eine andere Überwachungsaktion.",
    lastObserved: "Zuletzt vor {t} bestätigt",
    lastReading: "Letzte Messung",
    usageUnknown: "Nutzung nicht verfügbar",
    manualUpdates: "Dieser Build erhält nur manuelle Updates.",
    title: "Plan-Nutzung", loading: "Wird geladen…", noAccounts: "Keine Konten",
    noCredentials: "Keine Claude-Code- oder Codex-Anmeldedaten unter Windows oder WSL gefunden.",
    searching: "Konten werden gesucht…", refresh: "Aktualisieren",
    session: "Sitzung (5 Std.)", weekly: "Woche", weekly_opus: "Woche · Opus", weekly_sonnet: "Woche · Sonnet", limit: "Limit",
    resetsIn: "Zurückgesetzt in {t}", resetsNow: "Wird jetzt zurückgesetzt", credits: "Guthaben: {v}", unlimited: "Unbegrenzt",
    windowNotStarted: "Nicht gestartet · beginnt bei der ersten Nutzung",
    resetTimeUnavailable: "Rücksetzzeit nicht verfügbar", lastResetAt: "Letzte bekannte Rücksetzzeit: {d}", resetAwaitingRefresh: "Rücksetzzeit verstrichen · Aktualisierung ausstehend ({d})",
    token_expired: "Token abgelaufen – öffne {provider}, um es zu erneuern",
    token_invalid: "Token ungültig – öffne {provider}, um dich neu anzumelden",
    bad_response: "Unerwartete Antwort", http: "HTTP-Fehler {code}",
    quit: "Beenden", close: "Schließen",
    settings: "Einstellungen", sData: "Daten", sRefresh: "Aktualisieren alle", minutes: "{n} Min.", sProviders: "Anbieter", sGeneral: "Allgemein", sLanguage: "Sprache", langAuto: "Systemstandard", sStartup: "Beim Anmelden starten",
    unknownAccount: "Unbekanntes Konto", inUse: "Aktiv", use: "Verwenden", switching: "Wechsle…", idle: "Länger nicht genutzt – wechsle dorthin, um die Nutzung zu sehen", staleAgo: "vor {t} aktualisiert", switched: "{provider}: nutzt jetzt {to}", switchedAuto: "{provider}-Limit erreicht – zu {to} gewechselt", sAccounts: "Konten", addAccount: "Konto hinzufügen", addHint: "Melde dich im geöffneten Terminalfenster an. Das Konto erscheint hier, sobald du fertig bist.", alias: "Spitzname", remove: "Entfernen", activeCantRemove: "Wechsle zuerst zu einem anderen Konto, bevor du dieses entfernst.", cliMissing: "Claude-Code- oder Codex-CLI unter Windows oder WSL nicht gefunden.", autoSwitch: "Automatisch wechseln", switchAt: "Wechseln bei",
    descAccounts: "Deine Claude- und Codex-Konten und wann zwischen ihnen gewechselt wird.", descData: "Wie oft die Nutzung abgefragt wird und für welche Tools.", descGeneral: "Sprache und Autostart.",
    rateLimited: "Der Nutzungsdienst ist ausgelastet – neuer Versuch in ein paar Minuten",
    sUpdates: "Updates", autoUpdate: "Automatisch aktualisieren", checkUpdates: "Nach Updates suchen", checking: "Suche…", upToDate: "Du bist auf dem neuesten Stand", updateAvailable: "Version {v} ist verfügbar", installUpdate: "Installieren und neu starten", updating: "Aktualisiere auf {v}…", versionN: "Version {v}",
    resetFull: "Vollständiger Reset", resetSession: "Sitzungs-Reset", resetUsableNow: "jetzt nutzbar", resetExpires: "läuft ab am {d}", resetNext: "nächster in {t}", resetsTitle: "Limit-Resets",
    resetNextExpires: "nächster läuft ab am {d}",
    updatedTo: "Auf v{v} aktualisiert",
    sNotifications: "Benachrichtigungen", descNotifications: "Worüber SlopBar dich in Windows-Benachrichtigungen informiert.", notifyUsage: "Nutzungswarnungen", notifyUsageHint: "Wenn ein Limit 75 % und 90 % erreicht und wenn es aufgebraucht ist.", notifyResets: "Limits und Resets", notifyResetsHint: "Wenn ein aufgebrauchtes Limit zurück ist und wenn ein Konto einen Limit-Reset erhält.", notifySwitches: "Automatische Wechsel", notifySwitchesHint: "Wenn SlopBar für dich das Konto wechselt.", nUsageTitle: "{provider} bei {p} %", nLimitTitle: "{provider}-Limit erreicht", nLimitHasReset: "Du hast einen Limit-Reset, den du jetzt nutzen kannst.", nBackTitle: "{provider} ist wieder verfügbar", nBackBody: "Limit {window} zurückgesetzt · {account}", nGrantTitle: "Neuer {provider}-Limit-Reset", notifyRenewals: "Abo-Verlängerungen", notifyRenewalsHint: "Wenn sich ein scheinbar unnötiges Abo in 3 Tagen verlängert und wenn sich ein bezahltes Abo innerhalb eines Tages verlängert.", nRenewTitle: "{provider}-Abo verlängert sich in {t}", nRenewWasteHint: "Nicht nötig? Kündige es auf der Abrechnungsseite des Anbieters.", perMonth: "{v}/Monat", nRunsOutTitle: "{provider} aufgebraucht ~{at}", nRunsOutBody: "Wochenlimit · {account} · Reset in {t}", runsOut: "Aufgebraucht ~{at}",
    nProCutTitle: "Codex-Kontingent von Pro 200 sinkt nach dem {d}", nProCutBody: "Bei der jüngsten Nutzung würden diese das Wochenlimit erreichen: {accounts}", subProCut: "Das Codex-Kontingent von Pro 200 sinkt nach dem {d} (geschätzt auf etwa die Hälfte). Mit seiner jüngsten Spitze von {p} % käme dieses Konto auf {q} % des neuen Wochenlimits.",
  },
  it: {
    sDiagnostics: "Diagnostica",
    exportDiagnostics: "Esporta diagnostica",
    diagnosticsHint: "Cronologia locale: fino a 7 giorni o 10.000 eventi. Conti anonimi e utilizzo, senza credenziali o identità. Nessun caricamento automatico.",
    diagnosticsExporting: "Esportazione diagnostica…",
    diagnosticsSaved: "Diagnostica salvata: {path}",
    diagnosticsFailed: "Impossibile esportare. Controlla spazio su disco e accesso ai file.",
    monitorDisconnect: "Disconnetti monitoraggio",
    monitorDisconnected: "Monitoraggio disconnesso e sessione revocata.",
    monitorRevokeFailed: "Monitoraggio disconnesso su questo computer. La revoca sul server non è stata confermata; revoca questa sessione nelle impostazioni del tuo account.",
    monitor_uncertain: "Impossibile confermare il rinnovo dopo due tentativi. Riconnetti il monitoraggio per continuare.",
    monitorConnect: "Connetti monitoraggio",
    monitorReconnect: "Riconnetti monitoraggio",
    monitorConnected: "Monitoraggio indipendente connesso",
    monitorPending: "Completa l’accesso per il monitoraggio nel terminale aperto. Il tuo account attivo resta lo stesso.",
    monitor_setup: "Connetti il monitoraggio una volta per consultare l’utilizzo di questo account anche quando è inattivo.",
    monitor_reconnect: "È necessario riconnettere la sessione di monitoraggio. Non occorre usare l’account.",
    monitor_storage: "Impossibile salvare o leggere la sessione di monitoraggio. Controlla lo spazio su disco e l’accesso ai file.",
    monitor_network: "Servizio di monitoraggio non disponibile — nuovo tentativo a breve.",
    monitor_wrong_account: "Questo accesso appartiene a un altro account o a un’altra organizzazione. Accedi all’account selezionato qui.",
    monitor_shared_login: "Questa sessione è condivisa con una CLI. Effettua un nuovo accesso per il monitoraggio.",
    monitor_login_failed: "L’accesso per il monitoraggio non è stato completato. Riprova a connetterti.",
    monitor_busy: "Un’altra azione di monitoraggio è in corso per questo account.",
    lastObserved: "Ultima conferma {t} fa",
    lastReading: "Ultima lettura",
    usageUnknown: "Utilizzo non disponibile",
    manualUpdates: "Solo aggiornamenti manuali per questa versione.",
    title: "Utilizzo dei piani", loading: "Caricamento…", noAccounts: "Nessun account",
    noCredentials: "Nessuna credenziale di Claude Code o Codex trovata su Windows o WSL.",
    searching: "Ricerca account…", refresh: "Aggiorna",
    session: "Sessione (5 h)", weekly: "Settimana", weekly_opus: "Settimana · Opus", weekly_sonnet: "Settimana · Sonnet", limit: "Limite",
    resetsIn: "Si azzera tra {t}", resetsNow: "Si azzera ora", credits: "Crediti: {v}", unlimited: "Illimitati",
    windowNotStarted: "Non iniziato · inizia al primo utilizzo",
    resetTimeUnavailable: "Orario di azzeramento non disponibile", lastResetAt: "Ultimo azzeramento noto: {d}", resetAwaitingRefresh: "Orario di azzeramento trascorso · in attesa di aggiornamento ({d})",
    token_expired: "Token scaduto: apri {provider} per rinnovarlo",
    token_invalid: "Token non valido: apri {provider} per accedere di nuovo",
    bad_response: "Risposta inattesa", http: "Errore HTTP {code}",
    quit: "Esci", close: "Chiudi",
    settings: "Impostazioni", sData: "Dati", sRefresh: "Aggiorna ogni", minutes: "{n} min", sProviders: "Provider", sGeneral: "Generale", sLanguage: "Lingua", langAuto: "Predefinita di sistema", sStartup: "Avvia all’accesso",
    unknownAccount: "Account sconosciuto", inUse: "In uso", use: "Usa", switching: "Cambio…", idle: "Non usato di recente: passaci per vederne l’utilizzo", staleAgo: "aggiornato {t} fa", switched: "{provider}: ora usa {to}", switchedAuto: "Limite di {provider} raggiunto: passato a {to}", sAccounts: "Account", addAccount: "Aggiungi account", addHint: "Accedi nella finestra del terminale che si è aperta. L’account apparirà qui al termine.", alias: "Soprannome", remove: "Rimuovi", activeCantRemove: "Passa a un altro account prima di rimuovere questo.", cliMissing: "CLI di Claude Code o Codex non trovata su Windows o WSL.", autoSwitch: "Cambia automaticamente", switchAt: "Cambia al",
    descAccounts: "I tuoi account Claude e Codex, e quando passare dall’uno all’altro.", descData: "Ogni quanto viene controllato l’utilizzo e per quali strumenti.", descGeneral: "Lingua e avvio.",
    rateLimited: "Il servizio di utilizzo è occupato: nuovo tentativo tra qualche minuto",
    sUpdates: "Aggiornamenti", autoUpdate: "Aggiorna automaticamente", checkUpdates: "Cerca aggiornamenti", checking: "Ricerca…", upToDate: "Sei aggiornato", updateAvailable: "È disponibile la versione {v}", installUpdate: "Installa e riavvia", updating: "Aggiornamento a {v}…", versionN: "Versione {v}",
    resetFull: "Reset completo", resetSession: "Reset della sessione", resetUsableNow: "utilizzabile ora", resetExpires: "scade il {d}", resetNext: "prossimo tra {t}", resetsTitle: "Reset del limite",
    resetNextExpires: "il prossimo scade il {d}",
    updatedTo: "Aggiornato alla v{v}",
    sNotifications: "Notifiche", descNotifications: "Cosa ti segnala SlopBar nelle notifiche di Windows.", notifyUsage: "Avvisi di utilizzo", notifyUsageHint: "Quando un limite arriva al 75% e al 90%, e quando si esaurisce.", notifyResets: "Limiti e reset", notifyResetsHint: "Quando un limite esaurito torna disponibile e quando un account riceve un reset.", notifySwitches: "Cambi automatici", notifySwitchesHint: "Quando SlopBar cambia account per te.", nUsageTitle: "{provider} al {p}%", nLimitTitle: "Limite di {provider} raggiunto", nLimitHasReset: "Hai un reset del limite utilizzabile ora.", nBackTitle: "{provider} di nuovo disponibile", nBackBody: "Limite {window} azzerato · {account}", nGrantTitle: "Nuovo reset del limite di {provider}", notifyRenewals: "Rinnovi del piano", notifyRenewalsHint: "Quando un piano che sembra inutile si rinnova entro 3 giorni, e quando un piano a pagamento si rinnova entro un giorno.", nRenewTitle: "Il piano {provider} si rinnova tra {t}", nRenewWasteHint: "Non ti serve? Annullalo nella pagina di fatturazione del fornitore.", perMonth: "{v}/mese", nRunsOutTitle: "{provider} esaurito ~{at}", nRunsOutBody: "Limite settimanale · {account} · si azzera tra {t}", runsOut: "Esaurito ~{at}",
    nProCutTitle: "La quota Codex di Pro 200 scende dopo il {d}", nProCutBody: "Con l’uso recente questi raggiungerebbero il limite settimanale: {accounts}", subProCut: "La quota Codex di Pro 200 scende dopo il {d} (a circa la metà, stima). Con il picco recente del {p}%, questo account arriverebbe al {q}% del nuovo limite settimanale.",
  },
  ja: {
    sDiagnostics: "診断",
    exportDiagnostics: "診断をエクスポート",
    diagnosticsHint: "ローカル履歴は最大7日間または10,000件です。匿名のアカウント番号と使用量を含みますが、認証情報やアカウントの識別情報は含みません。自動送信はしません。",
    diagnosticsExporting: "診断をエクスポート中…",
    diagnosticsSaved: "診断を保存しました: {path}",
    diagnosticsFailed: "エクスポートできませんでした。空き容量とファイルへのアクセスを確認してください。",
    monitorDisconnect: "監視を切断",
    monitorDisconnected: "監視を切断し、セッションを失効させました。",
    monitorRevokeFailed: "このコンピューターの監視を切断しました。サーバー側の失効は確認できませんでした。プロバイダーのアカウント設定でこのセッションを失効させてください。",
    monitor_uncertain: "2回の試行で更新を確認できませんでした。監視を再接続してください。",
    monitorConnect: "監視を接続",
    monitorReconnect: "監視を再接続",
    monitorConnected: "独立した監視を接続済み",
    monitorPending: "開いたターミナルで監視用のサインインを完了してください。使用中のアカウントは変わりません。",
    monitor_setup: "一度監視を接続すると、このアカウントを使用していない間も使用状況を確認できます。",
    monitor_reconnect: "監視用のサインインを再接続してください。アカウントを使用する必要はありません。",
    monitor_storage: "監視用のサインインを保存または読み取れませんでした。空き容量とファイルへのアクセスを確認してください。",
    monitor_network: "監視サービスを利用できません。まもなく再試行します。",
    monitor_wrong_account: "別のアカウントまたは組織でサインインしています。ここで選択したアカウントにサインインしてください。",
    monitor_shared_login: "このサインインはCLIと共有されています。監視用に新しくサインインしてください。",
    monitor_login_failed: "監視用のサインインが完了しませんでした。もう一度接続してください。",
    monitor_busy: "このアカウントで別の監視操作が実行中です。",
    lastObserved: "最終確認: {t}前",
    lastReading: "前回の測定値",
    usageUnknown: "使用状況を取得できません",
    manualUpdates: "このビルドは手動更新のみです。",
    title: "プランの使用状況", loading: "読み込み中…", noAccounts: "アカウントなし",
    noCredentials: "Windows または WSL に Claude Code / Codex の認証情報が見つかりません。",
    searching: "アカウントを検索中…", refresh: "更新",
    session: "セッション（5時間）", weekly: "週間", weekly_opus: "週間 · Opus", weekly_sonnet: "週間 · Sonnet", limit: "上限",
    resetsIn: "{t}後にリセット", resetsNow: "まもなくリセット", credits: "クレジット: {v}", unlimited: "無制限",
    windowNotStarted: "未開始 · 初回使用時に開始",
    resetTimeUnavailable: "リセット時刻は不明です", lastResetAt: "最後に確認したリセット時刻: {d}", resetAwaitingRefresh: "リセット予定時刻を過ぎています · 更新待ち ({d})",
    token_expired: "トークンの期限切れ — {provider} を開いて更新してください",
    token_invalid: "トークンが無効です — {provider} を開いて再ログインしてください",
    bad_response: "予期しない応答", http: "HTTP エラー {code}",
    quit: "終了", close: "閉じる",
    settings: "設定", sData: "データ", sRefresh: "更新間隔", minutes: "{n} 分", sProviders: "プロバイダー", sGeneral: "一般", sLanguage: "言語", langAuto: "システムの既定", sStartup: "ログイン時に起動",
    unknownAccount: "不明なアカウント", inUse: "使用中", use: "使用", switching: "切り替え中…", idle: "最近使われていません — 切り替えると使用状況が表示されます", staleAgo: "{t}前に更新", switched: "{provider}: {to} を使用中", switchedAuto: "{provider} の上限に到達 — {to} に切り替えました", sAccounts: "アカウント", addAccount: "アカウントを追加", addHint: "開いたターミナルでサインインしてください。完了するとここに表示されます。", alias: "ニックネーム", remove: "削除", activeCantRemove: "削除する前に別のアカウントに切り替えてください。", cliMissing: "Windows または WSL に Claude Code / Codex の CLI が見つかりません。", autoSwitch: "自動で切り替える", switchAt: "切り替えのしきい値",
    descAccounts: "Claude と Codex のアカウントと、切り替えのタイミング。", descData: "使用状況を確認する頻度と対象のツール。", descGeneral: "言語と起動。",
    rateLimited: "使用状況サービスが混み合っています。数分後に再試行します",
    sUpdates: "アップデート", autoUpdate: "自動的にアップデート", checkUpdates: "アップデートを確認", checking: "確認中…", upToDate: "最新バージョンです", updateAvailable: "バージョン {v} が利用可能です", installUpdate: "インストールして再起動", updating: "{v} にアップデート中…", versionN: "バージョン {v}",
    resetFull: "フルリセット", resetSession: "セッションリセット", resetUsableNow: "今すぐ使用可能", resetExpires: "{d} に期限切れ", resetNext: "次回まで {t}", resetsTitle: "上限リセット",
    resetNextExpires: "次は {d} に期限切れ",
    updatedTo: "v{v} にアップデートしました",
    sNotifications: "通知", descNotifications: "SlopBar が Windows の通知でお知らせする内容。", notifyUsage: "使用量のアラート", notifyUsageHint: "上限の 75% と 90% に達したとき、上限に達したとき。", notifyResets: "上限とリセット", notifyResetsHint: "上限が回復したとき、アカウントが上限リセットを受け取ったとき。", notifySwitches: "自動切り替え", notifySwitchesHint: "SlopBar がアカウントを切り替えたとき。", nUsageTitle: "{provider} が {p}% に到達", nLimitTitle: "{provider} の上限に達しました", nLimitHasReset: "今すぐ使える上限リセットがあります。", nBackTitle: "{provider} が再び使えます", nBackBody: "{window} の上限がリセットされました · {account}", nGrantTitle: "{provider} の新しい上限リセット", notifyRenewals: "プランの更新", notifyRenewalsHint: "不要に見えるプランが 3 日以内に更新されるとき、有料プランが 1 日以内に更新されるとき。", nRenewTitle: "{provider} のプランが {t} 後に更新", nRenewWasteHint: "不要ならプロバイダーの請求ページで解約してください。", perMonth: "{v}/月", nRunsOutTitle: "{provider} は ~{at} に上限到達", nRunsOutBody: "週間上限 · {account} · {t} 後にリセット", runsOut: "~{at} に上限到達",
    nProCutTitle: "Codex Pro 200 の利用枠は {d} 以降に減ります", nProCutBody: "最近の使用量では次のアカウントが週間上限に達します: {accounts}", subProCut: "Pro 200 の Codex 利用枠は {d} 以降に減ります（推定で約半分）。最近のピーク {p}% では、このアカウントは新しい週間上限の {q}% に達します。",
  },
  zh: {
    sDiagnostics: "诊断",
    exportDiagnostics: "导出诊断",
    diagnosticsHint: "本地历史最多保留7天或10,000条事件。报告包含匿名账户编号和使用量，不含凭据或账户身份信息，不会自动上传。",
    diagnosticsExporting: "正在导出诊断…",
    diagnosticsSaved: "诊断已保存：{path}",
    diagnosticsFailed: "无法导出。请检查磁盘空间和文件访问权限。",
    monitorDisconnect: "断开监控",
    monitorDisconnected: "已断开监控并撤销会话。",
    monitorRevokeFailed: "已在此电脑上断开监控。未能确认服务器端撤销；请在服务商的账户设置中撤销此会话。",
    monitor_uncertain: "两次尝试后仍无法确认续期。请重新连接监控。",
    monitorConnect: "连接监控",
    monitorReconnect: "重新连接监控",
    monitorConnected: "已连接独立监控",
    monitorPending: "请在打开的终端中完成监控登录。当前使用的账户不会改变。",
    monitor_setup: "连接一次监控，即可在此账户闲置时继续查看用量。",
    monitor_reconnect: "监控登录需要重新连接，无需使用此账户。",
    monitor_storage: "无法保存或读取监控登录信息。请检查磁盘空间和文件访问权限。",
    monitor_network: "监控服务暂不可用，将很快重试。",
    monitor_wrong_account: "此次登录属于其他账户或组织。请登录此处选择的账户。",
    monitor_shared_login: "此次登录与 CLI 共享。请为监控重新登录。",
    monitor_login_failed: "监控登录未完成。请尝试重新连接。",
    monitor_busy: "此账户正在进行另一项监控操作。",
    lastObserved: "上次确认于 {t}前",
    lastReading: "上次读数",
    usageUnknown: "用量不可用",
    manualUpdates: "此版本仅支持手动更新。",
    title: "套餐用量", loading: "加载中…", noAccounts: "没有账户",
    noCredentials: "在 Windows 或 WSL 中未找到 Claude Code 或 Codex 凭据。",
    searching: "正在查找账户…", refresh: "刷新",
    session: "会话（5 小时）", weekly: "每周", weekly_opus: "每周 · Opus", weekly_sonnet: "每周 · Sonnet", limit: "限额",
    resetsIn: "{t}后重置", resetsNow: "即将重置", credits: "额度：{v}", unlimited: "无限",
    windowNotStarted: "未开始 · 首次使用时开始",
    resetTimeUnavailable: "重置时间不可用", lastResetAt: "上次确认的重置时间：{d}", resetAwaitingRefresh: "已过重置时间 · 等待刷新（{d}）",
    token_expired: "令牌已过期 — 请打开 {provider} 续期",
    token_invalid: "令牌无效 — 请打开 {provider} 重新登录",
    bad_response: "意外的响应", http: "HTTP 错误 {code}",
    quit: "退出", close: "关闭",
    settings: "设置", sData: "数据", sRefresh: "刷新间隔", minutes: "{n} 分钟", sProviders: "服务商", sGeneral: "通用", sLanguage: "语言", langAuto: "跟随系统", sStartup: "登录时启动",
    unknownAccount: "未知账户", inUse: "使用中", use: "使用", switching: "正在切换…", idle: "最近未使用 — 切换过去即可查看用量", staleAgo: "{t}前更新", switched: "{provider}：正在使用 {to}", switchedAuto: "{provider} 已达上限 — 已切换到 {to}", sAccounts: "账户", addAccount: "添加账户", addHint: "请在打开的终端窗口中登录，完成后账户会显示在这里。", alias: "昵称", remove: "移除", activeCantRemove: "请先切换到其他账户再移除此账户。", cliMissing: "在 Windows 或 WSL 中未找到 Claude Code 或 Codex CLI。", autoSwitch: "自动切换", switchAt: "切换阈值",
    descAccounts: "你的 Claude 和 Codex 账户，以及何时在它们之间切换。", descData: "多久检查一次用量，以及检查哪些工具。", descGeneral: "语言和启动。",
    rateLimited: "用量服务繁忙，几分钟后重试",
    sUpdates: "更新", autoUpdate: "自动更新", checkUpdates: "检查更新", checking: "正在检查…", upToDate: "已是最新版本", updateAvailable: "有新版本 {v} 可用", installUpdate: "安装并重启", updating: "正在更新到 {v}…", versionN: "版本 {v}",
    resetFull: "完全重置", resetSession: "会话重置", resetUsableNow: "现在可用", resetExpires: "{d} 到期", resetNext: "{t}后可用", resetsTitle: "额度重置",
    resetNextExpires: "最早 {d} 到期",
    updatedTo: "已更新到 v{v}",
    sNotifications: "通知", descNotifications: "SlopBar 通过 Windows 通知告诉你的内容。", notifyUsage: "用量提醒", notifyUsageHint: "限额达到 75% 和 90% 时，以及用尽时。", notifyResets: "限额和重置", notifyResetsHint: "用尽的限额恢复时，以及账户收到限额重置时。", notifySwitches: "自动切换", notifySwitchesHint: "SlopBar 为你切换账户时。", nUsageTitle: "{provider} 已用 {p}%", nLimitTitle: "{provider} 已达到限额", nLimitHasReset: "你有一次可立即使用的限额重置。", nBackTitle: "{provider} 已恢复可用", nBackBody: "{window} 限额已重置 · {account}", nGrantTitle: "新的 {provider} 限额重置", notifyRenewals: "套餐续费", notifyRenewalsHint: "看似不需要的套餐将在 3 天内续费时，以及任何付费套餐将在一天内续费时。", nRenewTitle: "{provider} 套餐将在 {t} 后续费", nRenewWasteHint: "不需要？请在服务商的账单页面取消。", perMonth: "{v}/月", nRunsOutTitle: "{provider} 约 {at} 用尽", nRunsOutBody: "每周限额 · {account} · {t} 后重置", runsOut: "约 {at} 用尽",
    nProCutTitle: "Codex Pro 200 额度将在 {d} 之后降低", nProCutBody: "按近期用量，这些账户会达到每周限额：{accounts}", subProCut: "Pro 200 的 Codex 额度将在 {d} 之后降低（估计约为一半）。按近期峰值 {p}%，此账户会用到新每周限额的 {q}%。",
  },
  ko: {
    sDiagnostics: "진단",
    exportDiagnostics: "진단 내보내기",
    diagnosticsHint: "로컬 기록은 최대 7일 또는 10,000개 이벤트입니다. 익명 계정 번호와 사용량이 포함되며 자격 증명이나 계정 식별 정보는 제외됩니다. 자동 업로드하지 않습니다.",
    diagnosticsExporting: "진단 내보내는 중…",
    diagnosticsSaved: "진단 저장됨: {path}",
    diagnosticsFailed: "내보내지 못했습니다. 디스크 공간과 파일 접근 권한을 확인하세요.",
    monitorDisconnect: "모니터링 연결 해제",
    monitorDisconnected: "모니터링 연결을 해제하고 세션을 취소했습니다.",
    monitorRevokeFailed: "이 컴퓨터의 모니터링 연결을 해제했습니다. 서버의 세션 취소는 확인되지 않았습니다. 제공업체의 계정 설정에서 이 세션을 취소하세요.",
    monitor_uncertain: "두 번 시도했지만 갱신을 확인하지 못했습니다. 모니터링을 다시 연결하세요.",
    monitorConnect: "모니터링 연결",
    monitorReconnect: "모니터링 다시 연결",
    monitorConnected: "독립 모니터링 연결됨",
    monitorPending: "열린 터미널에서 모니터링 로그인을 완료하세요. 사용 중인 계정은 바뀌지 않습니다.",
    monitor_setup: "모니터링을 한 번 연결하면 이 계정을 사용하지 않을 때도 사용량을 확인할 수 있습니다.",
    monitor_reconnect: "모니터링 로그인을 다시 연결해야 합니다. 계정을 사용할 필요는 없습니다.",
    monitor_storage: "모니터링 로그인을 저장하거나 읽지 못했습니다. 디스크 공간과 파일 접근 권한을 확인하세요.",
    monitor_network: "모니터링 서비스를 사용할 수 없습니다. 곧 다시 시도합니다.",
    monitor_wrong_account: "다른 계정 또는 조직으로 로그인했습니다. 여기에서 선택한 계정으로 로그인하세요.",
    monitor_shared_login: "이 로그인은 CLI와 공유됩니다. 모니터링을 위해 새로 로그인하세요.",
    monitor_login_failed: "모니터링 로그인이 완료되지 않았습니다. 다시 연결하세요.",
    monitor_busy: "이 계정에서 다른 모니터링 작업이 진행 중입니다.",
    lastObserved: "마지막 확인: {t} 전",
    lastReading: "마지막 측정값",
    usageUnknown: "사용량을 확인할 수 없음",
    manualUpdates: "이 빌드는 수동 업데이트만 지원합니다.",
    title: "요금제 사용량", loading: "불러오는 중…", noAccounts: "계정 없음",
    noCredentials: "Windows 또는 WSL에서 Claude Code 또는 Codex 자격 증명을 찾을 수 없습니다.",
    searching: "계정을 찾는 중…", refresh: "새로 고침",
    session: "세션 (5시간)", weekly: "주간", weekly_opus: "주간 · Opus", weekly_sonnet: "주간 · Sonnet", limit: "한도",
    resetsIn: "{t} 후 초기화", resetsNow: "곧 초기화", credits: "크레딧: {v}", unlimited: "무제한",
    windowNotStarted: "시작 전 · 처음 사용할 때 시작",
    resetTimeUnavailable: "초기화 시간 정보 없음", lastResetAt: "마지막으로 확인된 초기화: {d}", resetAwaitingRefresh: "초기화 시간 지남 · 새로 고침 대기 중 ({d})",
    token_expired: "토큰 만료 — {provider}을(를) 열어 갱신하세요",
    token_invalid: "토큰이 유효하지 않음 — {provider}을(를) 열어 다시 로그인하세요",
    bad_response: "예상치 못한 응답", http: "HTTP 오류 {code}",
    quit: "종료", close: "닫기",
    settings: "설정", sData: "데이터", sRefresh: "새로 고침 간격", minutes: "{n}분", sProviders: "제공자", sGeneral: "일반", sLanguage: "언어", langAuto: "시스템 기본값", sStartup: "로그인 시 실행",
    unknownAccount: "알 수 없는 계정", inUse: "사용 중", use: "사용", switching: "전환 중…", idle: "최근 사용 안 함 — 전환하면 사용량을 볼 수 있습니다", staleAgo: "{t} 전 업데이트", switched: "{provider}: {to} 사용 중", switchedAuto: "{provider} 한도 도달 — {to}(으)로 전환했습니다", sAccounts: "계정", addAccount: "계정 추가", addHint: "열린 터미널 창에서 로그인하세요. 완료되면 여기에 계정이 표시됩니다.", alias: "별명", remove: "제거", activeCantRemove: "이 계정을 제거하기 전에 다른 계정으로 전환하세요.", cliMissing: "Windows 또는 WSL에서 Claude Code 또는 Codex CLI를 찾을 수 없습니다.", autoSwitch: "자동 전환", switchAt: "전환 기준",
    descAccounts: "Claude와 Codex 계정, 그리고 계정을 전환할 시점.", descData: "사용량을 확인하는 주기와 대상 도구.", descGeneral: "언어 및 시작.",
    rateLimited: "사용량 서비스가 혼잡합니다. 몇 분 후 다시 시도합니다",
    sUpdates: "업데이트", autoUpdate: "자동 업데이트", checkUpdates: "업데이트 확인", checking: "확인 중…", upToDate: "최신 버전입니다", updateAvailable: "버전 {v}을(를) 사용할 수 있습니다", installUpdate: "설치 후 재시작", updating: "{v}(으)로 업데이트 중…", versionN: "버전 {v}",
    resetFull: "전체 초기화", resetSession: "세션 초기화", resetUsableNow: "지금 사용 가능", resetExpires: "{d} 만료", resetNext: "{t} 후 다음", resetsTitle: "한도 초기화",
    resetNextExpires: "다음 만료 {d}",
    updatedTo: "v{v}(으)로 업데이트됨",
    sNotifications: "알림", descNotifications: "SlopBar가 Windows 알림으로 알려 주는 내용.", notifyUsage: "사용량 알림", notifyUsageHint: "한도가 75%와 90%에 도달했을 때, 그리고 소진되었을 때.", notifyResets: "한도와 초기화", notifyResetsHint: "소진된 한도가 돌아왔을 때, 그리고 계정이 한도 초기화를 받았을 때.", notifySwitches: "자동 전환", notifySwitchesHint: "SlopBar가 계정을 전환했을 때.", nUsageTitle: "{provider} {p}% 사용", nLimitTitle: "{provider} 한도 도달", nLimitHasReset: "지금 사용할 수 있는 한도 초기화가 있습니다.", nBackTitle: "{provider}를 다시 사용할 수 있습니다", nBackBody: "{window} 한도 초기화됨 · {account}", nGrantTitle: "새 {provider} 한도 초기화", notifyRenewals: "요금제 갱신", notifyRenewalsHint: "불필요해 보이는 요금제가 3일 안에 갱신될 때, 그리고 유료 요금제가 하루 안에 갱신될 때.", nRenewTitle: "{provider} 요금제가 {t} 후 갱신", nRenewWasteHint: "필요 없나요? 제공업체의 결제 페이지에서 취소하세요.", perMonth: "{v}/월", nRunsOutTitle: "{provider} ~{at} 소진 예상", nRunsOutBody: "주간 한도 · {account} · {t} 후 초기화", runsOut: "~{at} 소진 예상",
    nProCutTitle: "Codex Pro 200 사용량이 {d} 이후 줄어듭니다", nProCutBody: "최근 사용량이면 다음 계정이 주간 한도에 도달합니다: {accounts}", subProCut: "Pro 200의 Codex 사용량이 {d} 이후 줄어듭니다(추정 약 절반). 최근 최고치 {p}%라면 이 계정은 새 주간 한도의 {q}%에 도달합니다.",
  },
  ru: {
    sDiagnostics: "Диагностика",
    exportDiagnostics: "Экспорт диагностики",
    diagnosticsHint: "Локальная история: до 7 дней или 10 000 событий. Анонимные номера аккаунтов и использование, без учётных данных и идентификации аккаунтов. Автоматической отправки нет.",
    diagnosticsExporting: "Экспорт диагностики…",
    diagnosticsSaved: "Диагностика сохранена: {path}",
    diagnosticsFailed: "Не удалось экспортировать. Проверьте место на диске и доступ к файлам.",
    monitorDisconnect: "Отключить мониторинг",
    monitorDisconnected: "Мониторинг отключён, сеанс отозван.",
    monitorRevokeFailed: "Мониторинг отключён на этом компьютере. Отзыв на сервере не подтверждён; отзовите этот сеанс в настройках аккаунта провайдера.",
    monitor_uncertain: "После двух попыток обновление не подтверждено. Подключите мониторинг заново.",
    monitorConnect: "Подключить мониторинг",
    monitorReconnect: "Подключить мониторинг заново",
    monitorConnected: "Независимый мониторинг подключён",
    monitorPending: "Завершите вход для мониторинга в открытом терминале. Активный аккаунт не изменится.",
    monitor_setup: "Подключите мониторинг один раз, чтобы видеть использование этого аккаунта даже во время простоя.",
    monitor_reconnect: "Нужно заново подключить сеанс мониторинга. Пользоваться аккаунтом для этого не требуется.",
    monitor_storage: "Не удалось сохранить или прочитать сеанс мониторинга. Проверьте место на диске и доступ к файлам.",
    monitor_network: "Сервис мониторинга недоступен — скоро повторим попытку.",
    monitor_wrong_account: "Этот вход относится к другому аккаунту или организации. Войдите в выбранный здесь аккаунт.",
    monitor_shared_login: "Этот сеанс используется совместно с CLI. Выполните новый вход для мониторинга.",
    monitor_login_failed: "Вход для мониторинга не завершён. Попробуйте подключиться ещё раз.",
    monitor_busy: "Для этого аккаунта уже выполняется другое действие мониторинга.",
    lastObserved: "Последнее подтверждение {t} назад",
    lastReading: "Последнее измерение",
    usageUnknown: "Использование недоступно",
    manualUpdates: "В этой сборке доступны только ручные обновления.",
    title: "Использование тарифов", loading: "Загрузка…", noAccounts: "Нет аккаунтов",
    noCredentials: "Учётные данные Claude Code или Codex не найдены ни в Windows, ни в WSL.",
    searching: "Поиск аккаунтов…", refresh: "Обновить",
    session: "Сессия (5 ч)", weekly: "Неделя", weekly_opus: "Неделя · Opus", weekly_sonnet: "Неделя · Sonnet", limit: "Лимит",
    resetsIn: "Сброс через {t}", resetsNow: "Сброс сейчас", credits: "Кредиты: {v}", unlimited: "Без ограничений",
    windowNotStarted: "Не начато · начнётся при первом использовании",
    resetTimeUnavailable: "Время сброса недоступно", lastResetAt: "Последнее известное время сброса: {d}", resetAwaitingRefresh: "Время сброса прошло · ожидание обновления ({d})",
    token_expired: "Токен истёк — откройте {provider}, чтобы обновить его",
    token_invalid: "Недействительный токен — откройте {provider}, чтобы войти снова",
    bad_response: "Неожиданный ответ", http: "Ошибка HTTP {code}",
    quit: "Выход", close: "Закрыть",
    settings: "Настройки", sData: "Данные", sRefresh: "Обновлять каждые", minutes: "{n} мин", sProviders: "Провайдеры", sGeneral: "Общие", sLanguage: "Язык", langAuto: "Как в системе", sStartup: "Запускать при входе",
    unknownAccount: "Неизвестный аккаунт", inUse: "Используется", use: "Использовать", switching: "Переключение…", idle: "Давно не использовался — переключитесь, чтобы увидеть расход", staleAgo: "обновлено {t} назад", switched: "{provider}: теперь используется {to}", switchedAuto: "Лимит {provider} исчерпан — переключено на {to}", sAccounts: "Аккаунты", addAccount: "Добавить аккаунт", addHint: "Войдите в открывшемся окне терминала. Аккаунт появится здесь, когда вы закончите.", alias: "Псевдоним", remove: "Удалить", activeCantRemove: "Сначала переключитесь на другой аккаунт, затем удалите этот.", cliMissing: "CLI Claude Code или Codex не найден ни в Windows, ни в WSL.", autoSwitch: "Переключать автоматически", switchAt: "Переключать при",
    descAccounts: "Ваши аккаунты Claude и Codex и когда между ними переключаться.", descData: "Как часто проверяется расход и для каких инструментов.", descGeneral: "Язык и автозапуск.",
    rateLimited: "Сервис учёта занят — повторим через несколько минут",
    sUpdates: "Обновления", autoUpdate: "Обновлять автоматически", checkUpdates: "Проверить обновления", checking: "Проверка…", upToDate: "У вас последняя версия", updateAvailable: "Доступна версия {v}", installUpdate: "Установить и перезапустить", updating: "Обновление до {v}…", versionN: "Версия {v}",
    resetFull: "Полный сброс", resetSession: "Сброс сессии", resetUsableNow: "доступен сейчас", resetExpires: "истекает {d}", resetNext: "следующий через {t}", resetsTitle: "Сбросы лимита",
    resetNextExpires: "ближайший истекает {d}",
    updatedTo: "Обновлено до v{v}",
    sNotifications: "Уведомления", descNotifications: "О чём SlopBar сообщает в уведомлениях Windows.", notifyUsage: "Оповещения об использовании", notifyUsageHint: "Когда лимит достигает 75% и 90% и когда он исчерпан.", notifyResets: "Лимиты и сбросы", notifyResetsHint: "Когда исчерпанный лимит восстановился и когда аккаунт получил сброс лимита.", notifySwitches: "Автоматическое переключение", notifySwitchesHint: "Когда SlopBar переключает аккаунт за вас.", nUsageTitle: "{provider}: {p}%", nLimitTitle: "Лимит {provider} исчерпан", nLimitHasReset: "У вас есть сброс лимита, который можно использовать сейчас.", nBackTitle: "{provider} снова доступен", nBackBody: "Лимит «{window}» сброшен · {account}", nGrantTitle: "Новый сброс лимита {provider}", notifyRenewals: "Продление тарифов", notifyRenewalsHint: "Когда тариф, который кажется ненужным, продлевается в течение 3 дней, и когда любой платный тариф продлевается в течение суток.", nRenewTitle: "Тариф {provider} продлится через {t}", nRenewWasteHint: "Не нужен? Отмените его на странице оплаты провайдера.", perMonth: "{v}/мес.", nRunsOutTitle: "{provider} закончится ~{at}", nRunsOutBody: "Недельный лимит · {account} · сброс через {t}", runsOut: "Закончится ~{at}",
    nProCutTitle: "Лимит Codex в Pro 200 снизится после {d}", nProCutBody: "При недавнем использовании эти аккаунты упрутся в недельный лимит: {accounts}", subProCut: "Лимит Codex в Pro 200 снизится после {d} (примерно вдвое, по оценкам). При недавнем пике {p}% этот аккаунт израсходует {q}% нового недельного лимита.",
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

/** A machine and where on it, e.g. "LAPTOP-2" or "LAPTOP-2 (WSL: Ubuntu)". */
const machineText = (r) => {
  const where = r.where ?? [];
  return where.length && !(where.length === 1 && where[0] === "Windows") ? `${r.machine} (${where.join(", ")})` : r.machine;
};

/**
 * Where an account is in use: `inUse` lists this machine's places and live other machines
 * ("Windows, LAPTOP-2 (WSL: Ubuntu)"), or says "In use on LAPTOP-2" when only other machines
 * have it; `seen` are machines that went quiet within the last day.
 */
export function placesText(a) {
  const remote = a.remote ?? [];
  const live = remote.filter((r) => r.live).map(machineText);
  const here = a.active ? a.sources : [];
  const inUse = here.length ? [...here, ...live].join(", ") : live.length ? t("inUseOn", { list: live.join(", ") }) : "";
  const seen = remote.filter((r) => !r.live).map((r) => t("seenOn", {
    machine: machineText(r), t: duration(Math.max(1, Math.round((Date.now() - new Date(r.seen_at)) / 60000))),
  }));
  return { inUse, elsewhere: live.length > 0, seen };
}

/** Short localized date, e.g. "Oct 22". */
export function shortDate(iso) {
  return new Date(iso).toLocaleDateString(LOCALE, { month: "short", day: "numeric" });
}

/** A weekday and hour, e.g. "Thu 3 PM". */
export function dayTime(iso) {
  return new Date(iso).toLocaleString(LOCALE, { weekday: "short", hour: "numeric" });
}

/** A calendar day ("2026-11-02", or midnight UTC standing for one), the same in every time zone. */
export function shortDay(iso) {
  return new Date(iso).toLocaleDateString(LOCALE, { month: "short", day: "numeric", timeZone: "UTC" });
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
  const scope = { session: "resetSession", weekly: "resetWeekly", full: "resetFull" }[r.kind];
  const name = scope ? (r.title || t(scope)) : t("resetUnknown");
  const parts = [r.count > 1 ? `${name} ×${r.count}` : name];
  if (r.usable) parts.push(t("resetUsableNow"));
  else if (r.count) parts.push(t("resetNotUsableNow"));
  if (r.expires_at) parts.push(t(r.entries > 1 ? "resetNextExpires" : "resetExpires", { d: shortDate(r.expires_at) }));
  if (r.next_available_at && !r.count) {
    const mins = Math.max(1, Math.round((new Date(r.next_available_at) - Date.now()) / 60000));
    parts.push(t("resetNext", { t: duration(mins) }));
  }
  return parts.join(" · ");
}

function resetDate(iso) {
  return new Date(iso).toLocaleString(LOCALE, {
    month: "short", day: "numeric", hour: "numeric", minute: "2-digit", timeZoneName: "short",
  });
}

export function resetsIn(iso) {
  const at = iso ? Date.parse(iso) : NaN;
  if (!Number.isFinite(at)) return t("resetTimeUnavailable");
  if (at <= Date.now()) return t("resetAwaitingRefresh", { d: resetDate(at) });
  const mins = Math.max(1, Math.ceil((at - Date.now()) / 60000));
  return t("resetsIn", { t: duration(mins) });
}

/**
 * Whether a window has not started yet. Codex reports an unused window as resetting one full
 * window length after the request, a time that moves with every poll rather than a real reset.
 */
export function notStarted(w, fetchedAt) {
  const at = w.resets_at ? Date.parse(w.resets_at) : NaN;
  const fetched = fetchedAt ? Date.parse(fetchedAt) : NaN;
  return w.used_percent === 0 && w.window_seconds > 0 && Number.isFinite(at) && Number.isFinite(fetched)
    && Math.abs(at - fetched - w.window_seconds * 1000) <= 120000;
}

/** Provider-supplied reset time, with separate unknown, not-started, stale and elapsed states. */
export function windowResetText(w, stale = false, fetchedAt = null) {
  const at = w.resets_at ? Date.parse(w.resets_at) : NaN;
  if (!Number.isFinite(at)) return t("resetTimeUnavailable");
  if (notStarted(w, fetchedAt)) return t("windowNotStarted");
  const date = resetDate(at);
  if (stale) return t("lastResetAt", { d: date });
  if (at <= Date.now()) return t("resetAwaitingRefresh", { d: date });
  return `${resetsIn(w.resets_at)} · ${date}`;
}

/** A session reset cannot restore included usage while the overall week is exhausted. */
export function visibleResetWindows(windows) {
  const weeklyExhausted = windows.some(w => w.kind === "weekly" && w.used_percent >= 100);
  return windows.filter(w => w.resets_at && Number.isFinite(Date.parse(w.resets_at))
    && !(weeklyExhausted && w.kind === "session"));
}

export function windowLabel(w) {
  if (w.kind !== "window") return t(w.kind);
  return w.window_seconds ? `${t("limit")} · ${duration(Math.round(w.window_seconds / 60))}` : t("limit");
}

const CLI_NAME = { Claude: "Claude Code", Codex: "Codex" };

export function errorText(provider, code) {
  const vars = { provider: CLI_NAME[provider] ?? provider };
  if (code.startsWith("monitor_")) return t(code);
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
