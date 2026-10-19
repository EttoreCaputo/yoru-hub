//! Services the app can offer with one click.
//!
//! Only official remote MCP servers whose URL was seen on the vendor's own
//! documentation or in the server's live OAuth metadata are listed, each with
//! where and when that was checked. "Dynamic" registration was confirmed by
//! a `registration_endpoint` in the authorization server's metadata
//! (RFC 8414) fetched on that day. Services that gate registration to
//! approved clients (Vercel, Figma, Canva's waitlist) are left out, as are
//! servers found only in metadata without a vendor page.
//!
//! The rest (Google, GitHub, Slack, Box, Asana, HubSpot) take neither dynamic
//! registration nor client ID metadata documents (checked 2026-10-19): they
//! sign in only with a client registered in their console. The app ships its
//! own, given at build time in `YORU_<SERVICE>_CLIENT_ID` and
//! `YORU_<SERVICE>_CLIENT_SECRET` (see `docs/oauth-clients.md`); with it the
//! service is one click like the others. A desktop app can't keep a secret,
//! so these are the providers' "installed app" clients, whose secret isn't
//! one, and PKCE protects the code. Without them in the build, the user's own
//! client stays possible ("Advanced" in the app).

use serde::{Serialize, Serializer};

use super::config::{ConnectorConfig, OAuthConfig, TransportConfig};

/// How a client gets an OAuth client id for the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Registration {
    /// Dynamic client registration: nothing to set up.
    Dynamic,
    /// Only a client registered in the service's console signs in: the
    /// app's own ([`Preset::app_client`]) or one the user creates.
    OwnClient,
    /// No sign-in needed.
    None,
}

/// The app's registered OAuth client for a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppClient {
    pub id: &'static str,
    pub secret: Option<&'static str>,
}

/// The client a build was given: an id, and a secret when the provider wants
/// one even from an installed app. Empty means none.
const fn app_client(id: Option<&'static str>, secret: Option<&'static str>) -> Option<AppClient> {
    let secret = match secret {
        Some(s) if !s.is_empty() => Some(s),
        _ => None,
    };
    match id {
        Some(id) if !id.is_empty() => Some(AppClient { id, secret }),
        _ => None,
    }
}

const GOOGLE_CLIENT: Option<AppClient> = app_client(option_env!("YORU_GOOGLE_CLIENT_ID"), option_env!("YORU_GOOGLE_CLIENT_SECRET"));
const GITHUB_CLIENT: Option<AppClient> = app_client(option_env!("YORU_GITHUB_CLIENT_ID"), option_env!("YORU_GITHUB_CLIENT_SECRET"));
const SLACK_CLIENT: Option<AppClient> = app_client(option_env!("YORU_SLACK_CLIENT_ID"), option_env!("YORU_SLACK_CLIENT_SECRET"));
const BOX_CLIENT: Option<AppClient> = app_client(option_env!("YORU_BOX_CLIENT_ID"), option_env!("YORU_BOX_CLIENT_SECRET"));
const ASANA_CLIENT: Option<AppClient> = app_client(option_env!("YORU_ASANA_CLIENT_ID"), option_env!("YORU_ASANA_CLIENT_SECRET"));
const HUBSPOT_CLIENT: Option<AppClient> = app_client(option_env!("YORU_HUBSPOT_CLIENT_ID"), option_env!("YORU_HUBSPOT_CLIENT_SECRET"));

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preset {
    /// The connector id it gets.
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    pub registration: Registration,
    /// The user's own client needs a secret too.
    pub needs_client_secret: bool,
    /// The app's own client, for an `own_client` service; the app only sees
    /// whether there is one.
    #[serde(serialize_with = "is_some")]
    pub app_client: Option<AppClient>,
    /// Where to read about the server (and create the client, for
    /// `own_client`).
    pub docs: &'static str,
    /// What the service gives Yoru, in one sentence.
    pub notes: &'static str,
    /// How to make one's own client, for `own_client`; empty otherwise.
    pub setup: &'static str,
    /// Scopes to ask for; empty lets the server's metadata decide.
    pub scopes: &'static [&'static str],
    pub auth_params: &'static [(&'static str, &'static str)],
    /// A fixed redirect port, for clients that must list their exact
    /// redirect URI (`http://127.0.0.1:<port>/callback`).
    pub redirect_port: Option<u16>,
}

fn is_some<S: Serializer>(client: &Option<AppClient>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_bool(client.is_some())
}

impl Preset {
    /// Connect clicks through: the sign-in page opens with nothing to fill in.
    pub fn one_click(&self) -> bool {
        self.registration != Registration::OwnClient || self.app_client.is_some()
    }

    /// The connector for this service. `client_id` and `client_secret` are
    /// the user's own client (Advanced); without them, an `own_client`
    /// service uses the app's. Its secret goes to the keychain like a
    /// user's, so refreshing works the same way.
    pub fn config(&self, client_id: Option<String>, client_secret: Option<String>) -> ConnectorConfig {
        let (client_id, client_secret) = match (client_id, self.app_client) {
            (None, Some(app)) => (Some(app.id.to_string()), app.secret.map(str::to_string)),
            (id, _) => (id, client_secret),
        };
        let oauth = OAuthConfig {
            client_id,
            client_secret,
            scopes: self.scopes.iter().map(|s| s.to_string()).collect(),
            auth_params: self.auth_params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            redirect_port: self.redirect_port,
            ..Default::default()
        };
        let needed = oauth != OAuthConfig::default();
        ConnectorConfig {
            id: self.id.into(),
            name: self.name.into(),
            transport: TransportConfig::Http { url: self.url.into() },
            oauth: needed.then_some(oauth),
            enabled: true,
            tools: Default::default(),
        }
    }
}

const fn dynamic(id: &'static str, name: &'static str, url: &'static str, docs: &'static str, notes: &'static str) -> Preset {
    Preset {
        id,
        name,
        url,
        registration: Registration::Dynamic,
        needs_client_secret: false,
        app_client: None,
        docs,
        notes,
        setup: "",
        scopes: &[],
        auth_params: &[],
        redirect_port: None,
    }
}

/// The redirect port of the Google preset: register
/// `http://127.0.0.1:8765/callback` on the Google OAuth client.
pub const GOOGLE_REDIRECT_PORT: u16 = 8765;
/// Google returns a refresh token only for `access_type=offline`; its doc
/// says `offline_access` isn't enough.
const GOOGLE_PARAMS: &[(&str, &str)] = &[("access_type", "offline"), ("prompt", "consent")];
const GOOGLE_DOCS: &str = "https://developers.google.com/workspace/guides/configure-mcp-servers";
const GOOGLE_SETUP: &str = "In your own Google Cloud project, enable the API and its MCP service, and create a \
\"Web application\" OAuth client (marked for an AI agent) with redirect URI http://127.0.0.1:8765/callback. \
While the project is in testing, Google makes you sign in again every 7 days.";

const fn google(id: &'static str, name: &'static str, url: &'static str, notes: &'static str) -> Preset {
    Preset {
        id,
        name,
        url,
        registration: Registration::OwnClient,
        needs_client_secret: true,
        app_client: GOOGLE_CLIENT,
        docs: GOOGLE_DOCS,
        notes,
        setup: GOOGLE_SETUP,
        scopes: &[],
        auth_params: GOOGLE_PARAMS,
        redirect_port: Some(GOOGLE_REDIRECT_PORT),
    }
}

const fn own_client(
    id: &'static str,
    name: &'static str,
    url: &'static str,
    app_client: Option<AppClient>,
    docs: &'static str,
    notes: &'static str,
    setup: &'static str,
) -> Preset {
    Preset {
        id,
        name,
        url,
        registration: Registration::OwnClient,
        needs_client_secret: true,
        app_client,
        docs,
        notes,
        setup,
        scopes: &[],
        auth_params: &[],
        redirect_port: None,
    }
}

static PRESETS: &[Preset] = &[
    // 2026-10-19: URL from the live 401 and metadata of mcp.notion.com (registration_endpoint /register);
    // https://developers.notion.com/docs/mcp
    dynamic("notion", "Notion", "https://mcp.notion.com/mcp", "https://developers.notion.com/docs/mcp", "Pages and databases of your workspace."),
    // 2026-10-19: https://linear.app/docs/mcp ("OAuth 2.1 with dynamic client registration"); metadata has /register.
    dynamic("linear", "Linear", "https://mcp.linear.app/mcp", "https://linear.app/docs/mcp", "Issues, projects and comments."),
    // 2026-10-19: https://support.atlassian.com/atlassian-rovo-mcp-server/docs/getting-started-with-the-atlassian-remote-mcp-server/
    // (v2 is current); metadata has a registration endpoint on auth.atlassian.com.
    dynamic(
        "atlassian",
        "Jira and Confluence",
        "https://mcp.atlassian.com/v2/mcp",
        "https://support.atlassian.com/atlassian-rovo-mcp-server/docs/getting-started-with-the-atlassian-remote-mcp-server/",
        "Atlassian's Rovo MCP server for Jira and Confluence.",
    ),
    // 2026-10-19: https://mcp.sentry.dev (docs.sentry.io/product/sentry-mcp redirects there); metadata has /oauth/register.
    dynamic("sentry", "Sentry", "https://mcp.sentry.dev/mcp", "https://mcp.sentry.dev", "Issues and errors of your Sentry organizations."),
    // 2026-10-19: https://docs.stripe.com/mcp; metadata on access.stripe.com has a registration endpoint.
    dynamic("stripe", "Stripe", "https://mcp.stripe.com", "https://docs.stripe.com/mcp", "Payments, customers and invoices. Its tools can act on real money."),
    // 2026-10-19: https://huggingface.co/docs/hub/en/hf-mcp-server; metadata has /oauth/register.
    dynamic("huggingface", "Hugging Face", "https://huggingface.co/mcp", "https://huggingface.co/docs/hub/en/hf-mcp-server", "Models, datasets and Spaces; works without signing in, with fewer tools."),
    // 2026-10-19: https://developers.cloudflare.com/agents/model-context-protocol/mcp-servers-for-cloudflare/; metadata has /register.
    dynamic(
        "cloudflare",
        "Cloudflare",
        "https://mcp.cloudflare.com/mcp",
        "https://developers.cloudflare.com/agents/model-context-protocol/mcp-servers-for-cloudflare/",
        "Your Cloudflare account through its API.",
    ),
    // 2026-10-19: same Cloudflare page; answered initialize without auth.
    Preset {
        registration: Registration::None,
        ..dynamic(
            "cloudflare-docs",
            "Cloudflare docs",
            "https://docs.mcp.cloudflare.com/mcp",
            "https://developers.cloudflare.com/agents/model-context-protocol/mcp-servers-for-cloudflare/",
            "Searches Cloudflare's documentation; no sign-in.",
        )
    },
    // 2026-10-19: Todoist's help page "Connect Todoist to an AI assistant" and https://developer.todoist.com;
    // metadata on todoist.com has /oauth/register.
    dynamic("todoist", "Todoist", "https://ai.todoist.net/mcp", "https://developer.todoist.com", "Tasks and projects."),
    // 2026-10-19: https://docs.zapier.com/mcp/get-started/connect; metadata has /api/v1/oauth/register.
    dynamic("zapier", "Zapier", "https://mcp.zapier.com/api/v1/connect", "https://docs.zapier.com/mcp/get-started/connect", "The actions you set up in Zapier, across thousands of apps."),
    // 2026-10-19: https://docs.github.com/en/copilot/how-tos/provide-context/use-mcp/set-up-the-github-mcp-server and
    // https://github.com/github/github-mcp-server/blob/main/docs/remote-server.md; metadata (github.com/login/oauth) has no registration.
    // The metadata has a device_authorization_endpoint, but the device flow needs a registered client too.
    own_client(
        "github",
        "GitHub",
        "https://api.githubcopilot.com/mcp/",
        GITHUB_CLIENT,
        "https://docs.github.com/en/copilot/how-tos/provide-context/use-mcp/set-up-the-github-mcp-server",
        "Repositories, issues and pull requests; an organization that restricts OAuth apps has to approve Yoru.",
        "Create a GitHub OAuth app (Settings, Developer settings) with callback URL http://127.0.0.1/callback.",
    ),
    // 2026-10-19: https://developers.google.com/workspace/guides/configure-mcp-servers; metadata (accounts.google.com) has no registration.
    google("gmail", "Gmail", "https://gmailmcp.googleapis.com/mcp/v1", "The mail of your Google account."),
    google("google-calendar", "Google Calendar", "https://calendarmcp.googleapis.com/mcp/v1", "Your calendars and their events."),
    google("google-drive", "Google Drive", "https://drivemcp.googleapis.com/mcp/v1", "The files in your Drive."),
    // 2026-10-19: https://docs.slack.dev/ai/slack-mcp-server/ (no DCR; a registered Slack app, confidential client).
    own_client(
        "slack",
        "Slack",
        "https://mcp.slack.com/mcp",
        SLACK_CLIENT,
        "https://docs.slack.dev/ai/slack-mcp-server/",
        "Channels, messages and people of your workspace.",
        "Create a Slack app of your workspace (internal or published) and give its client id and secret.",
    ),
    // 2026-10-19: https://developer.box.com/guides/box-mcp/remote/; metadata (api.box.com) has no registration.
    own_client(
        "box",
        "Box",
        "https://mcp.box.com",
        BOX_CLIENT,
        "https://developer.box.com/guides/box-mcp/remote/",
        "Files and folders in Box; an admin may have to allow it.",
        "An admin creates \"Box MCP server\" credentials in the Box Admin Console, with the redirect URI.",
    ),
    // 2026-10-19: https://developers.asana.com/docs/using-asanas-mcp-server; v2 metadata (app.asana.com) has no registration.
    own_client(
        "asana",
        "Asana",
        "https://mcp.asana.com/v2/mcp",
        ASANA_CLIENT,
        "https://developers.asana.com/docs/using-asanas-mcp-server",
        "Tasks, projects and goals.",
        "Create an Asana app in Asana's developer console and give its client id and secret.",
    ),
    // 2026-10-19: https://developers.hubspot.com/mcp; metadata has no registration.
    own_client(
        "hubspot",
        "HubSpot",
        "https://mcp.hubspot.com",
        HUBSPOT_CLIENT,
        "https://developers.hubspot.com/mcp",
        "Contacts, companies and deals of your CRM.",
        "Create a user-level app on HubSpot's developer platform and give its client id and secret.",
    ),
];

/// Every preset, in the order the app shows them.
pub fn presets() -> &'static [Preset] {
    PRESETS
}

pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::config::is_valid_id;
    use std::collections::HashSet;

    #[test]
    fn presets_make_valid_connectors() {
        for p in presets() {
            assert!(is_valid_id(p.id), "{}", p.id);
            assert!(p.url.starts_with("https://") && p.docs.starts_with("https://"), "{}", p.id);
            let client = (p.registration == Registration::OwnClient).then(|| "client".to_string());
            let c = p.config(client, None);
            c.validate().unwrap_or_else(|e| panic!("{}: {e}", p.id));
            assert_eq!(c.oauth.is_some(), p.registration == Registration::OwnClient, "{}", p.id);
            assert_eq!(p.setup.is_empty(), p.registration != Registration::OwnClient, "{}", p.id);
        }
    }

    #[test]
    fn the_app_client_is_used_unless_the_user_gives_one() {
        let p = Preset { app_client: Some(AppClient { id: "app-id", secret: Some("app-secret") }), ..preset("github").unwrap().clone() };
        assert!(p.one_click());
        let oauth = p.config(None, None).oauth.unwrap();
        assert_eq!((oauth.client_id.as_deref(), oauth.client_secret.as_deref()), (Some("app-id"), Some("app-secret")));
        let oauth = p.config(Some("mine".into()), Some("s".into())).oauth.unwrap();
        assert_eq!((oauth.client_id.as_deref(), oauth.client_secret.as_deref()), (Some("mine"), Some("s")));

        let bare = Preset { app_client: None, ..p };
        assert!(!bare.one_click());
        assert!(preset("notion").unwrap().one_click() && preset("cloudflare-docs").unwrap().one_click());
    }

    #[test]
    fn an_empty_build_variable_is_no_client() {
        assert_eq!(app_client(Some(""), Some("x")), None);
        assert_eq!(app_client(None, None), None);
        assert_eq!(app_client(Some("id"), Some("")), Some(AppClient { id: "id", secret: None }));
    }

    #[test]
    fn ids_are_unique() {
        let ids: HashSet<_> = presets().iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), presets().len());
    }

    #[test]
    fn google_asks_for_a_refresh_token_on_a_fixed_port() {
        let c = preset("gmail").unwrap().config(Some("id".into()), Some("secret".into()));
        let oauth = c.oauth.unwrap();
        assert_eq!(oauth.auth_params.get("access_type").map(String::as_str), Some("offline"));
        assert_eq!(oauth.redirect_port, Some(GOOGLE_REDIRECT_PORT));
        assert_eq!(oauth.client_secret.as_deref(), Some("secret"));
    }

    #[test]
    fn serializes() {
        let v = serde_json::to_value(preset("linear").unwrap()).unwrap();
        assert_eq!(v["registration"], "dynamic");
        assert_eq!(v["app_client"], false);
        // The app's secret never reaches the webview.
        let p = Preset { app_client: Some(AppClient { id: "id", secret: Some("s3cret") }), ..preset("github").unwrap().clone() };
        let v = serde_json::to_string(&p).unwrap();
        assert!(v.contains("\"app_client\":true") && !v.contains("s3cret"), "{v}");
    }
}
