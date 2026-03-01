use crate::error::ProtoError;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub struct HelloPayload {
    pub agent_name:     String,
    pub client_version: String,
    pub capabilities:   u32,
}

impl HelloPayload {
    pub fn encode(&self) -> Vec<u8> {
        let name = self.agent_name.as_bytes();
        let ver  = self.client_version.as_bytes();
        let mut buf = Vec::with_capacity(2 + name.len() + 2 + ver.len() + 4);
        buf.extend_from_slice(&(name.len() as u16).to_be_bytes());
        buf.extend_from_slice(name);
        buf.extend_from_slice(&(ver.len() as u16).to_be_bytes());
        buf.extend_from_slice(ver);
        buf.extend_from_slice(&self.capabilities.to_be_bytes());
        buf
    }

    pub fn decode(buf: &[u8]) -> Result<Self, ProtoError> {
        let mut p = 0;
        let nl = u16::from_be_bytes([buf[p], buf[p+1]]) as usize; p += 2;
        let agent_name = String::from_utf8_lossy(&buf[p..p+nl]).into_owned(); p += nl;
        let vl = u16::from_be_bytes([buf[p], buf[p+1]]) as usize; p += 2;
        let client_version = String::from_utf8_lossy(&buf[p..p+vl]).into_owned(); p += vl;
        let capabilities = u32::from_be_bytes(buf[p..p+4].try_into().unwrap());
        Ok(Self { agent_name, client_version, capabilities })
    }
}

pub fn compute_hmac(secret: &[u8], nonce: &[u8; 32]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(nonce);
    mac.finalize().into_bytes().to_vec()
}

/// Constant-time comparison to prevent timing attacks.
pub fn verify_hmac(secret: &[u8], nonce: &[u8; 32], response: &[u8]) -> bool {
    let expected = compute_hmac(secret, nonce);
    expected.len() == response.len()
        && expected.iter().zip(response.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hello_encode_decode() {
        let hello = HelloPayload { agent_name: "lex".into(), client_version: "0.1.0".into(), capabilities: 0 };
        let decoded = HelloPayload::decode(&hello.encode()).unwrap();
        assert_eq!(decoded.agent_name, "lex");
        assert_eq!(decoded.client_version, "0.1.0");
    }

    #[test]
    fn test_hmac_verifies() {
        let nonce = [0xab; 32];
        let resp = compute_hmac(b"fleet-secret", &nonce);
        assert!(verify_hmac(b"fleet-secret", &nonce, &resp));
    }

    #[test]
    fn test_hmac_wrong_secret_fails() {
        let nonce = [0xab; 32];
        let resp = compute_hmac(b"correct", &nonce);
        assert!(!verify_hmac(b"wrong", &nonce, &resp));
    }
}
