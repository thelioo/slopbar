# Sharing in-use status through cloud storage

SlopBar can show which accounts are in use on the user's other computers. Each SlopBar keeps one
small presence record per machine. The record holds the machine id, the computer name, when it
was written, and hashed account ids with where each account is signed in. It never holds emails,
tokens or organization names. See `src-tauri/src/presence.rs`.

Records go through the `PresenceStore` trait (`list`, `write`, `clear`). The reader, the staleness
rules and the UI only see that trait.

| Backend | Status | Needs |
| --- | --- | --- |
| `SyncedFolder`: any folder a sync client mirrors | Shipped | Nothing. Settings > Data detects OneDrive, Dropbox, Google Drive for desktop, iCloud Drive and Box, or takes a custom folder. |
| `Rclone`: any remote the user set up with `rclone config` | Shipped | rclone installed. **No app registration.** |
| `CloudApi(Dropbox)` | Optional stub, reports `not_configured` | A Dropbox app registered by the owner |
| `CloudApi(GoogleDrive)` | Optional stub, reports `not_configured` | A Google Cloud OAuth client registered by the owner |
| `CloudApi(OneDrive)` | Optional stub, reports `not_configured` | A Microsoft Entra app registered by the owner |

Machines without a sync client should use rclone. The direct API backends below are
**optional**: build them only if a sign-in inside SlopBar itself becomes worth registering apps
for. Every API backend would store records in an **app-private folder only**, so SlopBar never
asks for access to the user's own files.

Google Docs is not a target. Presence records are JSON files, and Google Drive covers Docs users.

## rclone: any cloud, no app registration (`src-tauri/src/rclone.rs`)

The user connects their cloud once, outside SlopBar, with `rclone config`. That covers Dropbox,
Google Drive, OneDrive, Box, pCloud, MEGA, S3, WebDAV, SFTP and the other rclone backends.
rclone keeps its own config and tokens.

- **Choosing a remote.** Settings > Data lists the remotes from `rclone listremotes --long` as
  "Dropbox (via rclone)" and so on, next to the detected sync folders. That command prints only
  names and backend types. SlopBar never runs `rclone config dump` or `config show`, and never
  reads `rclone.conf`, so it never sees a token.
- **When rclone is missing.** Settings shows a one-line hint: "Install rclone and run
  `rclone config` to connect any cloud without a sync app."
- **Where rclone is found.** On `PATH`, or in `%LOCALAPPDATA%\Microsoft\WinGet\Links`.
- **Calls.** SlopBar runs only these, and only on `<remote>:SlopBar/presence`:
  - `lsjson --files-only --no-mimetype` to list records;
  - `cat <file>` to read each one;
  - `rcat <id>.tmp`, then `moveto <id>.tmp <id>.json`, to write this machine's record
    atomically.

  A clean quit rewrites this machine's record with no accounts.
- **Each call.** rclone runs with no window, `--ask-password=false` (it never waits on an
  encrypted config's prompt), `--contimeout 10s --timeout 15s`, and a hard 20-second kill. Calls
  run off the async runtime. rclone's stderr is discarded and its output is never logged.
- **The setting.** `settings.json` stores only the remote's name, `presence_remote`. A chosen
  remote takes precedence over `presence_folder`. A missing rclone means no sharing; it never
  falls back to a folder.
- **Tests.** Unit tests use a fake rclone, an injected `Runner`. Run the live check, which uses
  only `SlopBar/presence` and removes its synthetic record afterwards, with:
  `SLOPBAR_RCLONE_LIVE=<remote> cargo test --lib rclone_live -- --ignored`

## Rejected: reusing browser sign-ins

Reading the user's browser cookies to reach their cloud storage, or borrowing another app's OAuth
client id, is ruled out:

- **It is credential extraction.** Decrypting another program's session cookies is what
  infostealer malware does. SlopBar would hold full-account sessions it has no business holding,
  far beyond one app folder.
- **Chrome app-bound encryption.** Since Chrome 127, Chromium browsers on Windows bind cookie
  encryption to the browser itself. Reading the cookies means defeating that protection.
- **Provider terms.** Dropbox, Google and Microsoft require API access through the app's own
  registered client. Session-cookie scraping and impersonating another client break their terms
  and get accounts flagged.

rclone avoids all three. The user signs in through rclone's own OAuth flow, rclone uses its own
registered clients, and SlopBar never touches the credentials.

## Rules every API backend follows

- **Tokens.** OAuth tokens go in DPAPI-protected storage through `private_store::save_secret`
  and `load_secret`, like the monitoring credentials, for example
  `<app dir>/presence/<provider>.bin`. They never go in `settings.json`, logs or diagnostics
  exports. `settings.json` records only which backend is chosen.
- **Sign-in flow.** Use the authorization-code flow with PKCE (S256) and a loopback redirect on
  `127.0.0.1`. The sign-in runs in the user's browser. SlopBar never sees a password.
- **Layout.** Use one file per machine, `presence/<machine id>.json`, overwritten in place. Read
  every file in that folder. `presence::peers` already handles conflict copies, clock skew and
  staleness.
- **Signing out.** Revoke the refresh token where the provider supports it, delete the DPAPI
  blob, and `clear()` this machine's record first.
- **Tests.** Use synthetic identities only, and never contact the real provider in tests.

## Wiring a backend in, once its app is registered

1. Put the client id in `CloudApi::client_id`, and the client "secret" for Google's Desktop
   client type. Neither is confidential for installed apps.
2. Implement `list` and `write` against the provider's API, as described below.
3. Add `presence_cloud: Option<CloudProvider>` to `Settings`. Have `presence::store` return
   `CloudApi` when it is set, next to the existing rclone and synced-folder choices.
4. In Settings > Data, offer "Connect Dropbox…", "Connect Google Drive…" and "Connect OneDrive…"
   next to the detected folders. Run the OAuth flow, store the tokens as above, then refresh.

## Dropbox (App folder, OAuth PKCE)

1. Sign in to <https://www.dropbox.com/developers/apps> with the owner's account and choose
   **Create app**.
2. Choose **Scoped access**, then **App folder** (not "Full Dropbox"). Name the app, for example
   `SlopBar`. The folder becomes `/Apps/SlopBar` in each user's Dropbox.
3. On the **Permissions** tab, tick `files.metadata.read`, `files.content.read` and
   `files.content.write`, then **Submit**.
4. On the **Settings** tab:
   - Set **Allow public clients (Implicit Grant & PKCE)** to *Allow*. PKCE then needs no app
     secret.
   - Under **Redirect URIs**, add the exact loopback URI SlopBar will listen on, for example
     `http://127.0.0.1:53682/oauth/dropbox`. Dropbox matches redirect URIs exactly, so fix the
     port.
   - Copy the **App key**. It is the client id.
5. Request `token_access_type=offline` so Dropbox returns a refresh token.
6. API: `files/upload` (mode `overwrite`) to `/presence/<machine id>.json`, then
   `files/list_folder` and `files/download` to read. Paths are relative to the app folder.
7. A new app is in *development* status and can be linked by a limited number of users (500 at
   the time of writing). Apply for **production** on the app's page before wide release.

## Google Drive (`drive.appdata`, Desktop OAuth client)

1. At <https://console.cloud.google.com>, create a project, for example `SlopBar`.
2. Go to **APIs & Services > Library**, find **Google Drive API** and **Enable** it.
3. Open **Google Auth Platform**, also shown as **OAuth consent screen**:
   - **Audience:** set User type to *External*.
   - **Branding:** set the app name, support email and developer contact.
   - **Data access:** add the scope `https://www.googleapis.com/auth/drive.appdata`. Google
     lists it as non-sensitive, so it needs no scope verification. Confirm the classification the
     console shows when adding it.
4. **Publishing status matters.** While the app is in **Testing**:
   - only listed test users (up to 100) can sign in;
   - **refresh tokens expire after 7 days**, so every machine would have to sign in again
     weekly.

   Choose **Publish app** to move it to *In production* before anyone relies on it.
5. Go to **Clients**, also shown as **Credentials**, choose **Create client**, and pick
   application type **Desktop app**. Copy the client id and client secret. For Desktop clients
   the secret is not confidential, and Google accepts any loopback redirect
   (`http://127.0.0.1:<port>`) without registering it.
6. API: create files with `parents: ["appDataFolder"]` (`files.create`), list them with
   `files.list?spaces=appDataFolder`, and update them with `files.update`.
7. The appDataFolder is hidden and private to this OAuth client's project. Every machine must
   use the same client id and the same Google account to see the same records.

## OneDrive (Microsoft Graph `Files.ReadWrite.AppFolder`, public client)

1. At <https://entra.microsoft.com>, go to **Identity > Applications > App registrations** and
   choose **New registration**.
2. Name it, for example `SlopBar`. Under **Supported account types**, choose *Accounts in any
   organizational directory and personal Microsoft accounts*.
3. Under **Redirect URI**, choose platform *Public client/native (mobile & desktop)* and enter
   `http://localhost`. Loopback ports are matched loosely for this platform.
4. Go to **Authentication > Advanced settings** and set **Allow public client flows** to *Yes*.
   The app uses no client secret.
5. Go to **API permissions > Add a permission > Microsoft Graph > Delegated permissions** and add
   `Files.ReadWrite.AppFolder` and `offline_access`. `User.Read` is there by default.
   - Personal accounts consent for themselves.
   - Work or school tenants may require admin consent, depending on tenant policy.
6. Copy the **Application (client) ID**. It is the client id. Use the authority
   `https://login.microsoftonline.com/common` with PKCE.
7. API: `PUT /me/drive/special/approot:/presence/<machine id>.json:/content` to write, and
   `GET /me/drive/special/approot:/presence:/children` to list. The folder appears as
   `Apps/SlopBar` in the user's OneDrive. Check `approot` support against work and school
   accounts before relying on it there.
