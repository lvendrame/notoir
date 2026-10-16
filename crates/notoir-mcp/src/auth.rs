//! Client tokens (docs/specs/29 §Client Identity): random 256-bit bearer
//! tokens, stored only as SHA-256 hashes, and the registry the HTTP layer
//! checks every request against.

use std::sync::{Arc, RwLock};

use notoir_core::permissions::ClientId;
use sha2::{Digest, Sha256};

/// Prefix that makes Notoir tokens recognisable in configs.
const PREFIX: &str = "ntr_";

/// A new random token (`ntr_` + 64 hex digits).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("system random source");
    format!("{PREFIX}{}", hex(&bytes))
}

/// A new random client id.
pub fn generate_client_id() -> ClientId {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("system random source");
    ClientId(hex(&bytes))
}

/// Hex SHA-256 of a token (what is stored).
pub fn hash_token(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time string equality (hash comparison).
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientAuth {
    pub id: ClientId,
    pub token_hash: String,
    pub revoked: bool,
}

/// Registered clients, shared between the UI thread (adds, revokes) and the
/// server (checks). Changes apply to the next request.
#[derive(Clone, Default)]
pub struct ClientRegistry(Arc<RwLock<Vec<ClientAuth>>>);

impl ClientRegistry {
    pub fn set(&self, clients: Vec<ClientAuth>) {
        *self.0.write().expect("registry lock") = clients;
    }

    pub fn add(&self, client: ClientAuth) {
        self.0.write().expect("registry lock").push(client);
    }

    pub fn revoke(&self, id: &ClientId) {
        for c in self.0.write().expect("registry lock").iter_mut() {
            if &c.id == id {
                c.revoked = true;
            }
        }
    }

    /// The client owning `token`, unless revoked.
    pub fn authenticate(&self, token: &str) -> Option<ClientId> {
        let hash = hash_token(token);
        self.0
            .read()
            .expect("registry lock")
            .iter()
            .find(|c| !c.revoked && same(&c.token_hash, &hash))
            .map(|c| c.id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_only_hashes_match() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
        assert!(a.starts_with("ntr_") && a.len() == 4 + 64);
        let reg = ClientRegistry::default();
        let id = generate_client_id();
        reg.add(ClientAuth {
            id: id.clone(),
            token_hash: hash_token(&a),
            revoked: false,
        });
        assert_eq!(reg.authenticate(&a), Some(id.clone()));
        assert_eq!(reg.authenticate(&b), None);
        assert_eq!(reg.authenticate(&hash_token(&a)), None);
        reg.revoke(&id);
        assert_eq!(reg.authenticate(&a), None);
    }
}
