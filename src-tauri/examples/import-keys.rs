use x_talk::credentials::{CredentialStore, NativeCredentials};
fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e.message);
        std::process::exit(1);
    }
}
fn run() -> x_talk::domain::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| x_talk::domain::AppError::input("Usage: import-keys <private .env path>"))?;
    let contents = std::fs::read_to_string(path)?;
    let store = NativeCredentials;
    for line in contents.lines() {
        let Some((name, value)) = line.trim().split_once('=') else {
            continue;
        };
        let provider = match name.trim() {
            "OPEN_AI" | "OPENAI_API_KEY" => "openai",
            "ELEVEN_LABS" | "ELEVENLABS_API_KEY" => "elevenlabs",
            "ASSEMBLYAI_API_KEY" => "assemblyai",
            "DEEPGRAM_API_KEY" => "deepgram",
            "ANTHROPIC_API_KEY" => "anthropic",
            _ => continue,
        };
        let value = value.trim().trim_matches(['\'', '"']);
        if !value.is_empty() {
            store.set(provider, value)?;
            println!("Stored {provider} key in Windows Credential Manager.");
        }
    }
    Ok(())
}
