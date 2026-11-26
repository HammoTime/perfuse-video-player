//! The complete session lives in Windows Credential Manager, never a plaintext file.
use perfuse::media::Source;
pub use perfuse::plugins::Credentials as Session;

pub fn open(session: &Session) -> Result<Source, String> {
    if session.client_id.is_empty() || session.client_id.len() > 200 {
        return Err("The saved device identifier is invalid. Sign in again.".into());
    }
    (session.kind.plugin().open)(session, !cfg!(test))
}

#[cfg(windows)]
pub use windows::{clear, load, save};

pub fn origin(server: &str) -> Option<String> {
    reqwest::Url::parse(server.trim())
        .ok()
        .map(|u| u.origin().ascii_serialization())
}

#[cfg(windows)]
mod windows {
    use super::Session;
    use perfuse::plugins::{BUILTIN, ServerKind};
    use std::{
        fs::{self, File},
        os::windows::fs::OpenOptionsExt,
        path::PathBuf,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_NOT_FOUND, ERROR_SHARING_VIOLATION, GetLastError},
        Security::Credentials::*,
    };

    const TARGET: &str = "Perfuse/PlexSession";

    // An exclusive Windows handle serializes the credential check and update across
    // app instances. The lock file is empty and Windows releases the handle on exit.
    fn lock_store() -> Result<File, String> {
        let base = std::env::var_os("LOCALAPPDATA")
            .ok_or("Could not locate your local app data folder.")?;
        let dir = PathBuf::from(base).join("Perfuse");
        fs::create_dir_all(&dir).map_err(|_| "Could not open the local Perfuse folder.")?;
        let path = dir.join("credentials.lock");
        let started = Instant::now();
        loop {
            match File::options()
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .open(&path)
            {
                Ok(file) => return Ok(file),
                Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) => {
                    if started.elapsed() >= Duration::from_secs(3) {
                        return Err(
                            "Another Perfuse instance is updating saved sign-ins. Please retry."
                                .into(),
                        );
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => return Err(
                    "Could not lock saved sign-ins. Check the local Perfuse folder permissions."
                        .into(),
                ),
            }
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    fn target(session: &Session) -> Result<String, String> {
        super::origin(&session.server)
            .map(|id| format!("Perfuse/{}/{id}", session.kind.plugin().name))
            .ok_or_else(|| "Invalid server address.".into())
    }
    pub fn load() -> Result<Vec<Session>, String> {
        let mut sessions = Vec::new();
        for plugin in BUILTIN {
            load_kind(plugin.kind, &mut sessions)?;
        }
        if let Some(legacy) = read(TARGET)?
            && legacy.kind == ServerKind::Plex
            && !sessions.iter().any(|s| {
                s.kind == legacy.kind && super::origin(&s.server) == super::origin(&legacy.server)
            })
        {
            sessions.push(legacy);
        }
        Ok(sessions)
    }

    fn load_kind(kind: ServerKind, sessions: &mut Vec<Session>) -> Result<(), String> {
        let filter = wide(&format!("Perfuse/{}/*", kind.plugin().name));
        let mut count = 0;
        let mut list = std::ptr::null_mut();
        // SAFETY: the filter is terminated and both output pointers are valid.
        if unsafe { CredEnumerateW(filter.as_ptr(), 0, &mut count, &mut list) } != 0 {
            // SAFETY: Windows owns the array and each terminated target name until CredFree.
            let names: Vec<_> = unsafe { std::slice::from_raw_parts(list, count as usize) }
                .iter()
                .map(|entry| unsafe {
                    let name = (**entry).TargetName;
                    let mut len = 0;
                    while *name.add(len) != 0 {
                        len += 1;
                    }
                    String::from_utf16_lossy(std::slice::from_raw_parts(name, len))
                })
                .collect();
            // SAFETY: releases the entire result returned by CredEnumerateW.
            unsafe { CredFree(list.cast()) };
            for name in names {
                if let Some(session) = read(&name)?
                    && session.kind == kind
                    && target(&session)? == name
                {
                    if kind == ServerKind::Audiobookshelf {
                        remember(&session, false)?;
                    }
                    sessions.push(session);
                }
            }
        } else if unsafe { GetLastError() } != ERROR_NOT_FOUND {
            return Err(format!(
                "Could not list saved {} sign-ins.",
                kind.plugin().name
            ));
        }
        Ok(())
    }
    pub fn save(session: &Session) -> Result<(), String> {
        if session.kind == ServerKind::Audiobookshelf {
            remember(session, true)
        } else {
            write(&target(session)?, session)
        }
    }

    fn remember(session: &Session, write_now: bool) -> Result<(), String> {
        let name = target(session)?;
        let (kind, server, client_id) = (
            session.kind,
            session.server.clone(),
            session.client_id.clone(),
        );
        // Capture only the address and device identity. Capturing the live session here
        // would keep its shared tokens and their rotation callback alive forever.
        let save = move |name: &str, token| {
            write(
                name,
                &Session {
                    kind,
                    server: server.clone(),
                    client_id: client_id.clone(),
                    token,
                },
            )
        };
        let rotate = save.clone();
        let rotate_name = name.clone();
        session.token.remember(
            |token| {
                if write_now {
                    let _store = lock_store()?;
                    save(&name, token.clone())?;
                }
                Ok(())
            },
            move |pair, refresh| {
                let store = lock_store()?;
                let saved = read(&rotate_name)?.and_then(|session| session.token.snapshot());
                match saved {
                    Some(saved) if saved.login == pair.login => {
                        if saved.revision > pair.revision {
                            *pair = saved;
                        } else {
                            refresh(pair)?;
                            rotate(&rotate_name, pair.clone().into())?;
                        }
                        Ok(())
                    }
                    _ => {
                        // Another instance removed or replaced the login. Only refresh in memory.
                        drop(store);
                        refresh(pair)
                    }
                }
            },
        )
    }
    pub fn clear(session: &Session) -> Result<(), String> {
        let name = target(session)?;
        if session.kind == ServerKind::Audiobookshelf {
            return session.token.forget(|expected| {
                let _store = lock_store()?;
                if let Some(saved) = read(&name)?
                    && match expected {
                        Some(id) => saved.token.snapshot().is_some_and(|pair| pair.login == id),
                        None => saved.token.fixed().ok() == session.token.fixed().ok(),
                    }
                {
                    delete(&name)?;
                }
                Ok(())
            });
        }
        if session.kind == ServerKind::Plex
            && read(TARGET)?
                .is_some_and(|s| super::origin(&s.server) == super::origin(&session.server))
        {
            delete(TARGET)?;
        }
        delete(&name)
    }

    fn read(target: &str) -> Result<Option<Session>, String> {
        let target = wide(target);
        let mut credential = std::ptr::null_mut();
        // SAFETY: target is NUL-terminated, and credential is a valid out-pointer.
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
            // SAFETY: immediately reads the calling thread's error from CredReadW.
            return if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err("Could not read the saved sign-in from Windows Credential Manager.".into())
            };
        }
        // SAFETY: a successful CredReadW owns this allocation until CredFree below.
        let result = unsafe {
            let value = &*credential;
            if value.CredentialBlobSize == 0
                || value.CredentialBlobSize > CRED_MAX_CREDENTIAL_BLOB_SIZE
                || value.CredentialBlob.is_null()
            {
                Err("The saved sign-in is invalid. Forget it and sign in again.".into())
            } else {
                let bytes = std::slice::from_raw_parts(
                    value.CredentialBlob,
                    value.CredentialBlobSize as usize,
                );
                serde_json::from_slice::<Session>(bytes).map_err(|_| {
                    "The saved sign-in is invalid. Forget it and sign in again.".to_owned()
                })
            }
        };
        // SAFETY: free exactly the allocation returned by CredReadW, after copying its data.
        unsafe { CredFree(credential.cast()) };
        let session = result?;
        super::open(&session)?;
        Ok(Some(session))
    }

    fn write(target: &str, session: &Session) -> Result<(), String> {
        super::open(session)?;
        let mut bytes = serde_json::to_vec(session)
            .map_err(|_| "Could not prepare the saved sign-in.".to_owned())?;
        if bytes.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
            return Err("This credential is too large for Windows Credential Manager.".into());
        }
        let mut target = wide(target);
        let mut username = wide(session.kind.plugin().name);
        let value = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: bytes.len() as u32,
            CredentialBlob: bytes.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: username.as_mut_ptr(),
            ..Default::default()
        };
        // SAFETY: all pointed-to buffers remain alive throughout this synchronous call.
        if unsafe { CredWriteW(&value, 0) } == 0 {
            // SAFETY: immediately reads the calling thread's error from CredWriteW.
            let code = unsafe { GetLastError() };
            Err(format!(
                "Windows could not save your sign-in (error {code}). You can continue for this session or try again in settings."
            ))
        } else {
            Ok(())
        }
    }

    fn delete(target: &str) -> Result<(), String> {
        let target = wide(target);
        // SAFETY: target is a live NUL-terminated string identifying only Perfuse's credential.
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0
            // SAFETY: on failure, immediately reads the calling thread's error.
            || unsafe { GetLastError() } == ERROR_NOT_FOUND
        {
            Ok(())
        } else {
            Err("Windows could not remove your saved sign-in. Try Sign out again.".into())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn targets_separate_sources_at_the_same_origin() {
            let mut session = Session {
                kind: ServerKind::Plex,
                server: "https://MEDiA.example.test:443/".into(),
                token: "synthetic".into(),
                client_id: "perfuse-test".into(),
            };
            assert_eq!(
                target(&session).unwrap(),
                "Perfuse/Plex/https://media.example.test"
            );
            session.kind = ServerKind::Jellyfin;
            assert_eq!(
                target(&session).unwrap(),
                "Perfuse/Jellyfin/https://media.example.test"
            );
            session.kind = ServerKind::Audiobookshelf;
            assert_eq!(
                target(&session).unwrap(),
                "Perfuse/Audiobookshelf/https://media.example.test"
            );
        }

        #[test]
        #[ignore = "requires an interactive Windows Credential Manager session"]
        fn credential_manager_roundtrip() {
            let target = format!(
                "Perfuse/Test/{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let session = Session {
                kind: ServerKind::Plex,
                server: "http://127.0.0.1:32499".into(),
                token: "synthetic-credential".into(),
                client_id: "perfuse-test".into(),
            };
            assert!(read(&target).unwrap().is_none());
            write(&target, &session).unwrap();
            let loaded = read(&target);
            let removed = delete(&target);
            assert!(removed.is_ok());
            let loaded = loaded.unwrap().unwrap();
            assert!(
                loaded.server == session.server
                    && loaded.token.fixed().unwrap() == session.token.fixed().unwrap()
                    && loaded.client_id == session.client_id
            );
            assert!(read(&target).unwrap().is_none());
        }
    }
}

#[cfg(not(windows))]
pub fn load() -> Result<Vec<Session>, String> {
    Ok(vec![])
}
#[cfg(not(windows))]
pub fn save(session: &Session) -> Result<(), String> {
    open(session)?;
    Err("Saved sign-in currently requires Windows Credential Manager.".into())
}
#[cfg(not(windows))]
pub fn clear(_session: &Session) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use perfuse::plugins::ServerKind;

    #[test]
    fn saved_session_rejects_invalid_origin_and_header() {
        let mut session = Session {
            kind: ServerKind::Plex,
            server: "https://plex.example.test".into(),
            token: "synthetic".into(),
            client_id: "perfuse-test".into(),
        };
        assert!(open(&session).is_ok());
        session.server = "https://user:password@example.test/?token=secret".into();
        assert!(open(&session).is_err());
        session.server = "https://plex.example.test".into();
        session.token = "bad\r\nHeader: injected".into();
        assert!(open(&session).is_err());
    }

    #[test]
    fn legacy_sessions_open_as_plex_and_jellyfin_roundtrips_with_its_kind() {
        let legacy = r#"{"server":"https://media.example.test","client_id":"perfuse-test","token":"synthetic"}"#;
        let mut session: Session = serde_json::from_str(legacy).unwrap();
        assert!(session.kind == ServerKind::Plex && open(&session).is_ok());
        session.kind = ServerKind::Jellyfin;
        let restored: Session =
            serde_json::from_slice(&serde_json::to_vec(&session).unwrap()).unwrap();
        assert!(restored.kind == ServerKind::Jellyfin && open(&restored).is_ok());
    }
}
