use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenClaims {
    pub project_id: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
}

#[derive(Debug, Error, PartialEq)]
pub enum AuthError {
    #[error("invalid token format")]
    InvalidFormat,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("wrong project")]
    WrongProject,
    #[error("token expired")]
    Expired,
    #[error("serialization error")]
    Serialization,
}

pub fn derive_project_secret(master_secret: &str, project_id: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(master_secret.as_bytes())
        .expect("hmac with arbitrary bytes must initialize");
    mac.update(format!("nexis.project.{project_id}").as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

pub fn decode_claims_unverified(token: &str) -> Result<TokenClaims, AuthError> {
    let mut parts = token.split('.');
    let payload_segment = parts.next().ok_or(AuthError::InvalidFormat)?;
    let _signature_segment = parts.next().ok_or(AuthError::InvalidFormat)?;
    if parts.next().is_some() {
        return Err(AuthError::InvalidFormat);
    }

    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_segment)
        .map_err(|_| AuthError::InvalidFormat)?;
    serde_json::from_slice(&payload_bytes).map_err(|_| AuthError::InvalidFormat)
}

pub fn mint_token(claims: &TokenClaims, secret: &str) -> Result<String, AuthError> {
    let payload = serde_json::to_vec(claims).map_err(|_| AuthError::Serialization)?;
    let encoded_payload = URL_SAFE_NO_PAD.encode(payload);

    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| AuthError::Serialization)?;
    mac.update(encoded_payload.as_bytes());
    let signature = mac.finalize().into_bytes();
    let encoded_signature = URL_SAFE_NO_PAD.encode(signature);

    Ok(format!("{}.{}", encoded_payload, encoded_signature))
}

pub fn verify_token(
    token: &str,
    expected_project_id: &str,
    secret: &str,
    now: DateTime<Utc>,
) -> Result<TokenClaims, AuthError> {
    let mut parts = token.split('.');
    let payload_segment = parts.next().ok_or(AuthError::InvalidFormat)?;
    let signature_segment = parts.next().ok_or(AuthError::InvalidFormat)?;
    if parts.next().is_some() {
        return Err(AuthError::InvalidFormat);
    }

    let signature = URL_SAFE_NO_PAD
        .decode(signature_segment)
        .map_err(|_| AuthError::InvalidFormat)?;

    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| AuthError::Serialization)?;
    mac.update(payload_segment.as_bytes());
    mac.verify_slice(&signature)
        .map_err(|_| AuthError::InvalidSignature)?;

    let claims = decode_claims_unverified(token)?;

    if claims.project_id != expected_project_id {
        return Err(AuthError::WrongProject);
    }

    if now > claims.expires_at {
        return Err(AuthError::Expired);
    }

    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn claims_fixture() -> TokenClaims {
        let issued_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        TokenClaims {
            project_id: "project-a".to_owned(),
            issued_at,
            expires_at: issued_at + Duration::minutes(30),
            key_id: None,
            aud: None,
        }
    }

    #[test]
    fn derived_project_secret_is_deterministic() {
        let one = derive_project_secret("master-1", "project-a");
        let two = derive_project_secret("master-1", "project-a");
        let three = derive_project_secret("master-1", "project-b");
        assert_eq!(one, two);
        assert_ne!(one, three);
    }

    #[test]
    fn unverified_claim_decode_roundtrip() {
        let claims = claims_fixture();
        let token = mint_token(&claims, "secret-1").expect("token mint should work");
        let decoded = decode_claims_unverified(&token).expect("token claims should decode");
        assert_eq!(decoded, claims);
    }

    #[test]
    fn valid_hmac_token_accepted() {
        let claims = claims_fixture();
        let token = mint_token(&claims, "secret-1").expect("token mint should work");
        let now = claims.issued_at + Duration::minutes(1);

        let verified =
            verify_token(&token, "project-a", "secret-1", now).expect("verify should pass");
        assert_eq!(verified, claims);
    }

    #[test]
    fn invalid_signature_rejected() {
        let claims = claims_fixture();
        let token = mint_token(&claims, "secret-1").expect("token mint should work");
        let now = claims.issued_at + Duration::minutes(1);

        let err = verify_token(&token, "project-a", "wrong-secret", now).unwrap_err();
        assert_eq!(err, AuthError::InvalidSignature);
    }

    #[test]
    fn wrong_project_rejected() {
        let claims = claims_fixture();
        let token = mint_token(&claims, "secret-1").expect("token mint should work");
        let now = claims.issued_at + Duration::minutes(1);

        let err = verify_token(&token, "project-b", "secret-1", now).unwrap_err();
        assert_eq!(err, AuthError::WrongProject);
    }

    #[test]
    fn expired_token_rejected() {
        let claims = claims_fixture();
        let token = mint_token(&claims, "secret-1").expect("token mint should work");
        let now = claims.expires_at + Duration::seconds(1);

        let err = verify_token(&token, "project-a", "secret-1", now).unwrap_err();
        assert_eq!(err, AuthError::Expired);
    }
}
