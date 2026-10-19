# The app's own OAuth clients

Connect on the Connectors page opens the service's sign-in page and asks nothing else. Most services allow that by themselves: the app registers a client named "Yoru" on the fly (dynamic client registration, RFC 7591), with PKCE and a loopback redirect (`http://127.0.0.1:<port>/callback`, RFC 8252).

Six don't: Google (Gmail, Calendar, Drive), GitHub, Slack, Box, Asana and HubSpot. Their authorization servers publish neither a `registration_endpoint` nor `client_id_metadata_document_supported` (checked 2026-10-19), so only a client registered in their console can sign in. The app ships one for each, compiled in at build time:

| Service | Build variables | Register it at | Redirect URI |
|---------|-----------------|----------------|--------------|
| Google | `YORU_GOOGLE_CLIENT_ID`, `YORU_GOOGLE_CLIENT_SECRET` | Google Cloud console: enable the Gmail, Calendar and Drive APIs and their MCP services, OAuth consent screen, then an OAuth client | `http://127.0.0.1:8765/callback` |
| GitHub | `YORU_GITHUB_CLIENT_ID`, `YORU_GITHUB_CLIENT_SECRET` | GitHub, Settings, Developer settings, OAuth Apps | `http://127.0.0.1/callback` (any port is accepted) |
| Slack | `YORU_SLACK_CLIENT_ID`, `YORU_SLACK_CLIENT_SECRET` | api.slack.com/apps, distributed so other workspaces can install it | as Slack's MCP docs require |
| Box | `YORU_BOX_CLIENT_ID`, `YORU_BOX_CLIENT_SECRET` | Box developer console | `http://127.0.0.1/callback` |
| Asana | `YORU_ASANA_CLIENT_ID`, `YORU_ASANA_CLIENT_SECRET` | Asana developer console | `http://127.0.0.1/callback` |
| HubSpot | `YORU_HUBSPOT_CLIENT_ID`, `YORU_HUBSPOT_CLIENT_SECRET` | HubSpot developer platform, a user-level app | `http://127.0.0.1/callback` |

`core/src/connectors/catalog.rs` reads them with `option_env!`, so set them in the environment of `cargo build` or `npm run tauri build`; cargo rebuilds when they change. The release workflow takes them from repository secrets of the same names. An empty or missing variable means no client: that service then shows under "With your own OAuth client" and signs in with a client the user creates, under Advanced.

A desktop app can't keep a secret: anyone can read these out of the binary. That is what the providers expect of an installed app (as with the OAuth apps behind most CLIs): the secret identifies the app, it doesn't authenticate it, and PKCE is what keeps a stolen code useless. Don't reuse a client that guards anything else.

Google: while the consent screen is in "testing", only listed test users can sign in and refresh tokens last 7 days; Gmail's scopes are restricted, so publishing needs Google's verification.

GitHub also offers the device flow (`device_authorization_endpoint`), which needs no secret but still a registered client id; the loopback flow is used for every service instead, so there is one way to sign in.
