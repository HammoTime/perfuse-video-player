//! Exchange account credentials or an approved Quick Connect code for a user token.
use super::{Api, http, json, send};
use serde::Deserialize;
use std::{
    thread,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Authentication {
    access_token: String,
}

pub fn password(
    server: &str,
    device: &str,
    username: &str,
    password: &str,
) -> Result<String, String> {
    if username.trim().is_empty() {
        return Err("Enter your Jellyfin username.".into());
    }
    let api = Api::new(server, device, None)?;
    let response = send(
        api.post("/Users/AuthenticateByName")
            .json(&serde_json::json!({
                "Username": username.trim(), "Pw": password,
            })),
    )?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Jellyfin rejected the username or password.".into());
    }
    Ok(http::json::<Authentication>(response, "Jellyfin")?.access_token)
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Challenge {
    code: String,
    secret: String,
}

pub fn quick_connect(
    server: &str,
    device: &str,
    show_code: impl FnOnce(String),
    cancelled: impl Fn() -> bool,
) -> Result<String, String> {
    let api = Api::new(server, device, None)?;
    let response = send(api.post("/QuickConnect/Initiate"))?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(
            "Quick Connect is disabled on this server. Sign in with your Jellyfin account.".into(),
        );
    }
    let challenge: Challenge = http::json(response, "Jellyfin")?;
    if challenge.code.len() != 6
        || !challenge.code.bytes().all(|b| b.is_ascii_digit())
        || challenge.secret.is_empty()
        || challenge.secret.len() > 256
    {
        return Err("Jellyfin returned an invalid Quick Connect challenge.".into());
    }
    show_code(challenge.code);
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct State {
        authenticated: bool,
    }
    let deadline = Instant::now() + Duration::from_secs(600);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_secs(5));
        if cancelled() {
            break;
        }
        // Jellyfin requires the one-time challenge secret in this polling query.
        let response = send(
            api.get("/QuickConnect/Connect")
                .query(&[("secret", &challenge.secret)]),
        )?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            break;
        }
        if http::json::<State>(response, "Jellyfin")?.authenticated && !cancelled() {
            let auth: Authentication = json(
                api.post("/Users/AuthenticateWithQuickConnect")
                    .json(&serde_json::json!({ "Secret": challenge.secret })),
            )?;
            return Ok(auth.access_token);
        }
    }
    Err("Quick Connect ended. Start again to get a new code.".into())
}
