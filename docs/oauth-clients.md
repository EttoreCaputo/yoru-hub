# The app's own OAuth clients

Connect on the Connectors page opens the service's sign-in page and asks nothing else. Most services allow that by themselves: the app registers a client named "Yoru" on the fly (dynamic client registration, RFC 7591), with PKCE and a loopback redirect (`http://127.0.0.1:<port>/callback`, RFC 8252).

Five don't: GitHub, Slack, Box, Asana and HubSpot. Their authorization servers publish neither a `registration_endpoint` nor `client_id_metadata_document_supported` (checked 2026-10-19), so only a client registered in their console can sign in. The app ships one for each, compiled in at build time:

| Service | Build variables | Register it at | Redirect URI |
|---------|-----------------|----------------|--------------|
| GitHub | `YORU_GITHUB_CLIENT_ID`, `YORU_GITHUB_CLIENT_SECRET` | GitHub, Settings, Developer settings, OAuth Apps | `http://127.0.0.1/callback` (any port is accepted) |
| Slack | `YORU_SLACK_CLIENT_ID`, `YORU_SLACK_CLIENT_SECRET` | api.slack.com/apps, distributed so other workspaces can install it | as Slack's MCP docs require |
| Box | `YORU_BOX_CLIENT_ID`, `YORU_BOX_CLIENT_SECRET` | Box developer console | `http://127.0.0.1/callback` |
| Asana | `YORU_ASANA_CLIENT_ID`, `YORU_ASANA_CLIENT_SECRET` | Asana developer console | `http://127.0.0.1/callback` |
| HubSpot | `YORU_HUBSPOT_CLIENT_ID`, `YORU_HUBSPOT_CLIENT_SECRET` | HubSpot developer platform, a user-level app | `http://127.0.0.1/callback` |

`core/src/connectors/catalog.rs` reads them with `option_env!`, so set them in the environment of `cargo build` or `npm run tauri build`; cargo rebuilds when they change. The release workflow takes them from repository secrets of the same names. An empty or missing variable means no client: that service then shows under "With your own OAuth client" and signs in with a client the user creates, under Advanced.

A desktop app can't keep a secret: anyone can read these out of the binary. That is what the providers expect of an installed app (as with the OAuth apps behind most CLIs): the secret identifies the app, it doesn't authenticate it, and PKCE is what keeps a stolen code useless. Don't reuse a client that guards anything else.

## Google: always the user's own client

Gmail and Google Calendar don't go through Google's MCP servers or a client of the app: a client of ours would need Google's verification (Gmail's scopes are restricted), which we can't get. Yoru Hub serves their tools itself (`core/src/connectors/google/`) on the REST APIs, `gmail.googleapis.com` and `www.googleapis.com/calendar/v3`, which take any Google account, personal ones too, and are free within their quotas. Each user signs in with a client they make in their own Google Cloud project:

1. At console.cloud.google.com, create a project and enable the **Gmail API** and the **Google Calendar API**.
2. **OAuth consent screen** (Google Auth Platform): type External, add yourself as a test user. Then **Publish app**: while it stays in "testing", Google's refresh tokens last 7 days, so Yoru would ask for a sign-in every week. Published but not verified, Google shows "Google hasn't verified this app" at sign-in (Advanced, Go to …): that's expected for an app used only by its own developer, and fine under 100 users.
3. **Credentials**, Create credentials, **OAuth client ID**, type **Desktop app**. A Desktop client takes any loopback port, so there is no redirect URI to register.
4. In Yoru Hub, Connectors, Gmail (or Google Calendar), Advanced: give the client id and secret, or paste the JSON the console downloads. One client serves both; each connector signs in once.

The scopes asked for: Gmail `gmail.modify` and `gmail.send` (read, label, archive, trash, drafts, send); Calendar `calendar.events` and `calendar.readonly`. The tokens are in the keychain like every connector's; rmcp's OAuth isn't used for them because it sends an RFC 8707 `resource` Google's token endpoint doesn't expect.

Connectors made before 2026-10-22 pointed at Google's MCP servers (`gmailmcp.googleapis.com`, `calendarmcp.googleapis.com`); `Connectors::open` moves them to the REST APIs, keeping the client id and a fixed redirect port (a "Web application" client still works), and drops their tokens and tool policies, so they ask for a sign-in once. Drive is gone for now.

GitHub also offers the device flow (`device_authorization_endpoint`), which needs no secret but still a registered client id; the loopback flow is used for every service instead, so there is one way to sign in.
