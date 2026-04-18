use crate::config::Credentials;
use crate::error::CliResult;

/// Save a JWT bearer token for the given profile.
pub fn save_token(profile: &str, token: &str) -> CliResult<()> {
    let mut creds = Credentials::load()?;
    creds.set_token(profile, token.to_string());
    creds.save()?;
    Ok(())
}

pub fn clear_token(profile: &str) -> CliResult<()> {
    let mut creds = Credentials::load()?;
    creds.clear(profile);
    creds.save()?;
    Ok(())
}
