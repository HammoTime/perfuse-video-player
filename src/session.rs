//! The complete session lives in Windows Credential Manager, never a plaintext file.
pub use perfuse::plugins::Credentials as Session;
use perfuse::{media::Source, plugins::plex};

pub fn open(session: &Session) -> Result<Source, String> {
    if session.client_id.is_empty() || session.client_id.len() > 200 {
        return Err("The saved device identifier is invalid. Sign in again.".into());
    }
    (plex::PLUGIN.open)(session, !cfg!(test))
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
    use windows_sys::Win32::{
        Foundation::{ERROR_NOT_FOUND, GetLastError},
        Security::Credentials::*,
    };

    const TARGET: &str = "Perfuse/PlexSession";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    const PREFIX: &str = "Perfuse/Plex/";
    fn target(server: &str) -> Result<String, String> {
        super::origin(server)
            .map(|id| format!("{PREFIX}{id}"))
            .ok_or_else(|| "Invalid server address.".into())
    }
    pub fn load() -> Result<Vec<Session>, String> {
        let mut sessions = Vec::new();
        let mut count = 0;
        let mut list = std::ptr::null_mut();
        // SAFETY: the filter is terminated and both output pointers are valid.
        if unsafe {
            CredEnumerateW(
                wide(&format!("{PREFIX}*")).as_ptr(),
                0,
                &mut count,
                &mut list,
            )
        } != 0
        {
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
                if let Some(session) = read(&name)? {
                    sessions.push(session);
                }
            }
        } else if unsafe { GetLastError() } != ERROR_NOT_FOUND {
            return Err("Could not list saved Plex sign-ins.".into());
        }
        if let Some(legacy) = read(TARGET)?
            && !sessions
                .iter()
                .any(|s| super::origin(&s.server) == super::origin(&legacy.server))
        {
            sessions.push(legacy);
        }
        Ok(sessions)
    }
    pub fn save(session: &Session) -> Result<(), String> {
        write(&target(&session.server)?, session)
    }
    pub fn clear(server: &str) -> Result<(), String> {
        if read(TARGET)?.is_some_and(|s| super::origin(&s.server) == super::origin(server)) {
            delete(TARGET)?;
        }
        delete(&target(server)?)
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
        let mut username = wide("Plex");
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
                    && loaded.token == session.token
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
pub fn clear(_server: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_session_rejects_invalid_origin_and_header() {
        let mut session = Session {
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
}
