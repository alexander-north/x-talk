use crate::domain::*;
pub const PROVIDERS: [&str; 5] = [
    "openai",
    "assemblyai",
    "elevenlabs",
    "deepgram",
    "anthropic",
];
pub trait CredentialStore: Send + Sync {
    fn get(&self, provider: &str) -> Result<String>;
    fn set(&self, provider: &str, key: &str) -> Result<()>;
    fn delete(&self, provider: &str) -> Result<()>;
}
pub struct NativeCredentials;
fn check(provider: &str) -> Result<()> {
    if !PROVIDERS.contains(&provider) {
        return Err(AppError::input("Unknown credential provider."));
    }
    Ok(())
}
pub fn display_name(provider: &str) -> &str {
    match provider {
        "openai" => "OpenAI",
        "assemblyai" => "AssemblyAI",
        "elevenlabs" => "ElevenLabs",
        "deepgram" => "Deepgram",
        "anthropic" => "Anthropic",
        other => other,
    }
}
pub fn missing_key_message(provider: &str) -> String {
    format!(
        "Add your {} API key in Settings → API Keys.",
        display_name(provider)
    )
}
#[cfg(windows)]
mod native {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_NOT_FOUND},
        Security::Credentials::*,
    };
    fn target(provider: &str) -> Vec<u16> {
        format!("x-talk/{provider}\0").encode_utf16().collect()
    }
    impl CredentialStore for NativeCredentials {
        fn get(&self, provider: &str) -> Result<String> {
            check(provider)?;
            let target = target(provider);
            let mut ptr = std::ptr::null_mut();
            unsafe {
                if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut ptr) == 0 {
                    return Err(AppError::new(
                        "credentials",
                        "missing_key",
                        missing_key_message(provider),
                    ));
                }
                let bytes = std::slice::from_raw_parts(
                    (*ptr).CredentialBlob,
                    (*ptr).CredentialBlobSize as usize,
                );
                let key = String::from_utf8(bytes.to_vec()).map_err(|_| {
                    AppError::new(
                        "credentials",
                        "invalid_key",
                        "Stored API key is invalid. Replace it.",
                    )
                });
                CredFree(ptr.cast());
                key
            }
        }
        fn set(&self, provider: &str, key: &str) -> Result<()> {
            check(provider)?;
            valid_key(key)?;
            let mut target = target(provider);
            let mut user: Vec<u16> = "x-talk\0".encode_utf16().collect();
            let mut bytes = key.trim().as_bytes().to_vec();
            let mut cred: CREDENTIALW = unsafe { std::mem::zeroed() };
            cred.Type = CRED_TYPE_GENERIC;
            cred.TargetName = target.as_mut_ptr();
            cred.CredentialBlobSize = bytes.len() as u32;
            cred.CredentialBlob = bytes.as_mut_ptr();
            cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
            cred.UserName = user.as_mut_ptr();
            let ok = unsafe { CredWriteW(&cred, 0) };
            bytes.fill(0);
            if ok == 0 {
                return Err(AppError::new(
                    "credentials",
                    "write",
                    "Windows Credential Manager could not save the key.",
                ));
            }
            Ok(())
        }
        fn delete(&self, provider: &str) -> Result<()> {
            check(provider)?;
            let target = target(provider);
            unsafe {
                if CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) == 0
                    && GetLastError() != ERROR_NOT_FOUND
                {
                    return Err(AppError::new(
                        "credentials",
                        "delete",
                        "Windows Credential Manager could not delete the key.",
                    ));
                }
            }
            Ok(())
        }
    }
}
/// Keychain service shared by every stored API key. Matches the bundle identifier, so the
/// login keychain shows the items under the app that owns them.
#[cfg(any(target_os = "macos", test))]
const KEYCHAIN_SERVICE: &str = "com.xander.xtalk";
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
fn valid_key(key: &str) -> Result<()> {
    if key.trim().is_empty() || key.len() > 2500 || key.chars().any(char::is_control) {
        return Err(AppError::input("Enter a valid API key."));
    }
    Ok(())
}
#[cfg(target_os = "macos")]
mod native {
    //! Generic passwords in the login keychain: service = bundle identifier, account = provider.
    use super::*;
    use security_framework::passwords::{
        delete_generic_password, generic_password, set_generic_password, PasswordOptions,
    };
    /// `errSecItemNotFound` from Security.framework's SecBase.h.
    const ITEM_NOT_FOUND: i32 = -25300;
    /// One keychain service. The app uses [`KEYCHAIN_SERVICE`]; tests use their own.
    pub(super) struct Keychain(pub(super) &'static str);
    impl CredentialStore for Keychain {
        fn get(&self, provider: &str) -> Result<String> {
            check(provider)?;
            let bytes = match generic_password(PasswordOptions::new_generic_password(
                self.0, provider,
            )) {
                Ok(bytes) => bytes,
                Err(e) if e.code() == ITEM_NOT_FOUND => {
                    return Err(AppError::new(
                        "credentials",
                        "missing_key",
                        missing_key_message(provider),
                    ))
                }
                Err(e) => {
                    tracing::warn!(status = e.code(), "keychain_read_failed");
                    return Err(AppError::new(
                            "credentials",
                            "read",
                            "macOS Keychain could not read the key. Unlock the login keychain or allow x-talk access, then try again.",
                        ));
                }
            };
            String::from_utf8(bytes).map_err(|_| {
                AppError::new(
                    "credentials",
                    "invalid_key",
                    "Stored API key is invalid. Replace it.",
                )
            })
        }
        fn set(&self, provider: &str, key: &str) -> Result<()> {
            check(provider)?;
            valid_key(key)?;
            set_generic_password(self.0, provider, key.trim().as_bytes()).map_err(|e| {
                tracing::warn!(status = e.code(), "keychain_write_failed");
                AppError::new(
                    "credentials",
                    "write",
                    "macOS Keychain could not save the key.",
                )
            })
        }
        fn delete(&self, provider: &str) -> Result<()> {
            check(provider)?;
            match delete_generic_password(self.0, provider) {
                Err(e) if e.code() != ITEM_NOT_FOUND => {
                    tracing::warn!(status = e.code(), "keychain_delete_failed");
                    Err(AppError::new(
                        "credentials",
                        "delete",
                        "macOS Keychain could not delete the key.",
                    ))
                }
                _ => Ok(()),
            }
        }
    }
    impl CredentialStore for NativeCredentials {
        fn get(&self, provider: &str) -> Result<String> {
            Keychain(KEYCHAIN_SERVICE).get(provider)
        }
        fn set(&self, provider: &str, key: &str) -> Result<()> {
            Keychain(KEYCHAIN_SERVICE).set(provider, key)
        }
        fn delete(&self, provider: &str) -> Result<()> {
            Keychain(KEYCHAIN_SERVICE).delete(provider)
        }
    }
}
/// Development builds on other platforms (Linux) have no native secret store.
#[cfg(not(any(windows, target_os = "macos")))]
impl CredentialStore for NativeCredentials {
    fn get(&self, p: &str) -> Result<String> {
        check(p)?;
        Err(AppError::new("credentials","missing_key","Credential storage is supported on Windows and macOS. This build is for development verification."))
    }
    fn set(&self, p: &str, _: &str) -> Result<()> {
        self.get(p).map(|_| ())
    }
    fn delete(&self, p: &str) -> Result<()> {
        self.get(p).map(|_| ())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_validation_rejects_blank_oversized_and_control_characters() {
        assert!(valid_key("sk-test").is_ok());
        assert!(valid_key("  ").is_err());
        assert!(valid_key("a\nb").is_err());
        assert!(valid_key(&"x".repeat(2501)).is_err());
    }
    /// Writes to the login keychain under a separate test service, never the app's keys.
    /// Ignored by default: a locked keychain or a CI runner without a login keychain can
    /// block or prompt. Run on a Mac with `cargo test keychain -- --ignored`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Uses the login keychain; may prompt or fail on locked/headless keychains."]
    fn keychain_round_trip_replaces_and_deletes_like_credential_manager() {
        let store = native::Keychain("com.xander.xtalk.tests");
        let _ = store.delete("openai");
        assert_eq!(store.get("openai").unwrap_err().category, "missing_key");
        store.set("openai", " sk-first ").unwrap();
        assert_eq!(store.get("openai").unwrap(), "sk-first");
        store.set("openai", "sk-second").unwrap();
        assert_eq!(store.get("openai").unwrap(), "sk-second");
        store.delete("openai").unwrap();
        store.delete("openai").unwrap();
        assert_eq!(store.get("openai").unwrap_err().category, "missing_key");
        assert!(store.set("unknown", "sk").is_err());
        assert!(store.set("openai", "a\nb").is_err());
    }
    #[test]
    fn keychain_service_matches_the_bundle_identifier() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], KEYCHAIN_SERVICE);
    }
    #[test]
    fn missing_key_message_names_provider_and_settings_location() {
        assert_eq!(
            missing_key_message("openai"),
            "Add your OpenAI API key in Settings → API Keys."
        );
        for provider in PROVIDERS {
            assert_ne!(display_name(provider), provider);
        }
    }
}
