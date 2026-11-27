use crate::media::{SourceError, SourceResult};
use reqwest::{
    StatusCode, Url,
    blocking::{Client, RequestBuilder, Response},
    header::HeaderMap,
};
use serde::de::DeserializeOwned;
use std::{io::Read, net::IpAddr, time::Duration};

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

/// Whether `server` sends sign-ins and media without TLS to a host outside this network.
/// Loopback, private, link-local and single-label or `.local` names count as local.
pub fn unencrypted_remote(server: &str) -> bool {
    let Ok(url) = Url::parse(server.trim()) else {
        return false;
    };
    let Some(host) = url.host_str().filter(|_| url.scheme() == "http") else {
        return false;
    };
    match host.trim_start_matches('[').trim_end_matches(']').parse() {
        Ok(IpAddr::V4(ip)) => {
            !(ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified())
        }
        Ok(IpAddr::V6(ip)) => {
            let unique_local = (ip.segments()[0] & 0xfe00) == 0xfc00;
            let link_local = (ip.segments()[0] & 0xffc0) == 0xfe80;
            !(ip.is_loopback() || ip.is_unspecified() || unique_local || link_local)
        }
        Err(_) => {
            let name = host.trim_end_matches('.').to_ascii_lowercase();
            !(name == "localhost"
                || !name.contains('.')
                || [".local", ".lan", ".home.arpa", ".internal", ".localhost"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix)))
        }
    }
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

/// Sends a request. Transport failures never include the request URL in the message.
pub fn send(request: RequestBuilder, server: &str) -> SourceResult<Response> {
    request.send().map_err(|_| {
        SourceError::Unreachable(format!(
            "Cannot reach {server}. Check the address, network, and TLS certificate."
        ))
    })
}

/// An endpoint on the connected origin. Only fixed paths and validated identifiers are joined.
pub fn endpoint(base: &Url, path: &str) -> Url {
    let mut url = base.clone();
    url.set_path(path);
    url
}

pub fn check_status(status: StatusCode, server: &str) -> SourceResult<()> {
    if status.is_success() {
        Ok(())
    } else if status == StatusCode::UNAUTHORIZED {
        Err(SourceError::Unauthorized(format!(
            "{server} no longer accepts this sign-in. Sign in again."
        )))
    } else if status == StatusCode::FORBIDDEN {
        Err(SourceError::Failed(format!(
            "{server} denied access. Check your sign-in and library permissions."
        )))
    } else if status.is_redirection() {
        Err(SourceError::Failed(format!(
            "{server} redirected the request. Connect to the final server address directly."
        )))
    } else {
        Err(SourceError::Failed(format!(
            "{server} returned HTTP {}.",
            status.as_u16()
        )))
    }
}

pub fn json<T: DeserializeOwned>(response: Response, server: &str) -> SourceResult<T> {
    check_status(response.status(), server)?;
    let bytes = read_limited(response, 16 * 1024 * 1024)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| format!("{server} returned an unexpected JSON response.").into())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_remote_plain_http_is_unencrypted() {
        for remote in [
            "http://203.0.113.7:32400",
            "http://media.example.com",
            "http://[2001:db8::1]:8096",
        ] {
            assert!(unencrypted_remote(remote), "{remote}");
        }
        for local in [
            "https://media.example.com",
            "http://localhost:32400",
            "http://127.0.0.1:32499",
            "http://192.168.1.30:8096",
            "http://10.0.0.2",
            "http://nas:13378",
            "http://nas.local:8096",
            "http://[::1]:8096",
            "http://[fd00::2]",
            "not a url",
        ] {
            assert!(!unencrypted_remote(local), "{local}");
        }
    }

    #[test]
    fn statuses_map_to_typed_errors() {
        assert!(
            check_status(StatusCode::UNAUTHORIZED, "Plex")
                .unwrap_err()
                .is_unauthorized()
        );
        let forbidden = check_status(StatusCode::FORBIDDEN, "Plex").unwrap_err();
        assert!(!forbidden.is_unauthorized() && forbidden.message().contains("denied access"));
        assert!(check_status(StatusCode::OK, "Plex").is_ok());
    }
}
