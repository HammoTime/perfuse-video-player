use reqwest::{
    Url,
    blocking::{Client, Response},
    header::HeaderMap,
};
use serde::de::DeserializeOwned;
use std::{io::Read, time::Duration};

pub fn base_url(server: &str) -> Result<Url, String> {
    let base = Url::parse(server.trim()).map_err(|_| "Enter a valid server URL.".to_owned())?;
    if !matches!(base.scheme(), "http" | "https")
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
        || base.path() != "/"
    {
        return Err(
            "Enter an HTTP or HTTPS server address without a path, query or credentials.".into(),
        );
    }
    Ok(base)
}

pub fn client(headers: HeaderMap) -> Result<Client, String> {
    Client::builder()
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "Could not initialize networking.".into())
}

pub fn check_status(status: reqwest::StatusCode, server: &str) -> Result<(), String> {
    if status.is_success() {
        Ok(())
    } else if matches!(status.as_u16(), 401 | 403) {
        Err(format!(
            "{server} denied access. Check your sign-in and library permissions."
        ))
    } else if status.is_redirection() {
        Err(format!(
            "{server} redirected the request. Connect to the final server address directly."
        ))
    } else {
        Err(format!("{server} returned HTTP {}.", status.as_u16()))
    }
}

pub fn json<T: DeserializeOwned>(response: Response, server: &str) -> Result<T, String> {
    check_status(response.status(), server)?;
    let bytes = read_limited(response, 16 * 1024 * 1024)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| format!("{server} returned an unexpected JSON response."))
}

pub fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "The server response was interrupted.".to_owned())?;
    if bytes.len() as u64 > limit {
        return Err("The server response exceeded the size limit.".into());
    }
    Ok(bytes)
}
