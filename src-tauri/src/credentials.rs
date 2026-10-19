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
                        format!("Add a {provider} API key in Providers."),
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
            if key.trim().is_empty() || key.len() > 2500 || key.chars().any(char::is_control) {
                return Err(AppError::input("Enter a valid API key."));
            }
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
#[cfg(not(windows))]
impl CredentialStore for NativeCredentials {
    fn get(&self, p: &str) -> Result<String> {
        check(p)?;
        Err(AppError::new("credentials","missing_key","Credential storage is supported on Windows. This build is for development verification."))
    }
    fn set(&self, p: &str, _: &str) -> Result<()> {
        self.get(p).map(|_| ())
    }
    fn delete(&self, p: &str) -> Result<()> {
        self.get(p).map(|_| ())
    }
}
