use std::collections::BTreeMap;

use cozo::{DataValue, MultiTransaction};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use super::{PlanStore, PlanStoreIdentity, db_err};

/// Opaque authority required to persist terminal approval or task generations.
///
/// Only [`PlanStore::bootstrap_approval_authority`] can create this capability.
/// Its fields deliberately remain private and it does not implement Clone, Debug,
/// or serialization traits.
pub struct PlanApprovalAuthority {
    store_identity: PlanStoreIdentity,
    session_id: String,
    secret: [u8; 32],
}

impl PlanStore {
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn bootstrap_approval_authority_for_test(
        &self,
        session_id: &str,
    ) -> Result<PlanApprovalAuthority, std::io::Error> {
        self.bootstrap_approval_authority(session_id, [0xA5; 32])
    }

    /// Authenticate the trusted CLI's session capability against the durable
    /// store-root verifier, then insert or authenticate the session verifier.
    ///
    /// The interactive runtime must load/create its owner-only secret and call
    /// this before exposing or cloning `PlanStore` to task management,
    /// rehydration, or other downstream code. A pre-start hostile first writer
    /// is an excluded initialization-race/denial-of-service threat; it cannot
    /// silently authorize execution in the normal startup ordering. Once the
    /// store root exists, every session requires the same process-private secret;
    /// a caller with only `PlanStore` cannot authorize an attacker-chosen session.
    pub fn bootstrap_approval_authority(
        &self,
        session_id: &str,
        secret: [u8; 32],
    ) -> Result<PlanApprovalAuthority, std::io::Error> {
        let authority = PlanApprovalAuthority {
            store_identity: self.identity.clone(),
            session_id: session_id.into(),
            secret,
        };
        let transaction = self.db.multi_transaction(true);
        let result = self.ensure_authority_in(&transaction, &authority);
        self.finish_transaction(transaction, result)?;
        Ok(authority)
    }

    fn approval_verifier(secret: &[u8; 32]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"archon.plan-approval-authority.v2\\0");
        hasher.update(secret);
        hex::encode(hasher.finalize())
    }

    fn session_verifier(session_id: &str, secret: &[u8; 32]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"archon.plan-approval-session.v2\\0");
        hasher.update((session_id.len() as u64).to_be_bytes());
        hasher.update(session_id.as_bytes());
        hasher.update(secret);
        hex::encode(hasher.finalize())
    }

    fn verify_authority_in(
        &self,
        transaction: &MultiTransaction,
        authority: &PlanApprovalAuthority,
    ) -> Result<(), std::io::Error> {
        if authority.store_identity != self.identity {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "plan approval authority belongs to another store",
            ));
        }
        self.verify_root_in(transaction, &authority.secret)?;
        let verifier = Self::session_verifier(&authority.session_id, &authority.secret);
        let mut params = BTreeMap::new();
        params.insert("sid".into(), DataValue::from(authority.session_id.as_str()));
        let rows = transaction
            .run_script(
                "?[verifier] := *plan_approval_authorities{session_id, verifier}, session_id = $sid",
                params,
            )
            .map_err(db_err)?;
        match rows
            .rows
            .first()
            .and_then(|row| row.first())
            .and_then(DataValue::get_str)
        {
            Some(stored) if stored == verifier => Ok(()),
            Some(_) => Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "plan approval authority verification failed",
            )),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "plan approval authority is not initialized",
            )),
        }
    }

    fn verify_root_in(
        &self,
        transaction: &MultiTransaction,
        secret: &[u8; 32],
    ) -> Result<(), std::io::Error> {
        let rows = transaction
            .run_script(
                "?[verifier] := *plan_approval_roots{authority_id, verifier}, authority_id = 'store-root'",
                BTreeMap::new(),
            )
            .map_err(db_err)?;
        let expected = Self::approval_verifier(secret);
        match rows
            .rows
            .first()
            .and_then(|row| row.first())
            .and_then(DataValue::get_str)
        {
            Some(stored) if stored == expected => Ok(()),
            Some(_) => Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "plan approval root verification failed",
            )),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "plan approval root is not initialized",
            )),
        }
    }

    fn ensure_authority_in(
        &self,
        transaction: &MultiTransaction,
        authority: &PlanApprovalAuthority,
    ) -> Result<(), std::io::Error> {
        if authority.store_identity != self.identity {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "plan approval authority belongs to another store",
            ));
        }
        self.ensure_root_in(transaction, &authority.secret)?;
        let verifier = Self::session_verifier(&authority.session_id, &authority.secret);
        let mut params = BTreeMap::new();
        params.insert("sid".into(), DataValue::from(authority.session_id.as_str()));
        let rows = transaction
            .run_script(
                "?[verifier] := *plan_approval_authorities{session_id, verifier}, session_id = $sid",
                params,
            )
            .map_err(db_err)?;
        match rows
            .rows
            .first()
            .and_then(|row| row.first())
            .and_then(DataValue::get_str)
        {
            Some(stored) if stored == verifier => Ok(()),
            Some(_) => Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "plan approval authority verification failed",
            )),
            None => {
                let mut params = BTreeMap::new();
                params.insert("sid".into(), DataValue::from(authority.session_id.as_str()));
                params.insert("verifier".into(), DataValue::from(verifier.as_str()));
                transaction
                    .run_script(
                        "?[session_id, verifier] <- [[$sid, $verifier]] :insert plan_approval_authorities {session_id => verifier}",
                        params,
                    )
                    .map_err(db_err)?;
                Ok(())
            }
        }
    }

    fn ensure_root_in(
        &self,
        transaction: &MultiTransaction,
        secret: &[u8; 32],
    ) -> Result<(), std::io::Error> {
        let rows = transaction
            .run_script(
                "?[verifier] := *plan_approval_roots{authority_id, verifier}, authority_id = 'store-root'",
                BTreeMap::new(),
            )
            .map_err(db_err)?;
        let verifier = Self::approval_verifier(secret);
        match rows
            .rows
            .first()
            .and_then(|row| row.first())
            .and_then(DataValue::get_str)
        {
            Some(stored) if stored == verifier => Ok(()),
            Some(_) => Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "plan approval root verification failed",
            )),
            None => {
                let existing_sessions = transaction
                    .run_script(
                        "?[session_id] := *plan_approval_authorities{session_id}",
                        BTreeMap::new(),
                    )
                    .map_err(db_err)?;
                if !existing_sessions.rows.is_empty() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "plan approval root is missing from a nonempty authority store",
                    ));
                }
                let mut params = BTreeMap::new();
                params.insert("verifier".into(), DataValue::from(verifier.as_str()));
                transaction
                    .run_script(
                        "?[authority_id, verifier] <- [['store-root', $verifier]] :insert plan_approval_roots {authority_id => verifier}",
                        params,
                    )
                    .map_err(db_err)?;
                Ok(())
            }
        }
    }

    pub(super) fn require_authority_in(
        &self,
        transaction: &MultiTransaction,
        authority: &PlanApprovalAuthority,
        session_id: &str,
    ) -> Result<(), std::io::Error> {
        if authority.session_id != session_id {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "plan approval authority belongs to another session",
            ));
        }
        self.verify_authority_in(transaction, authority)
    }

    pub(super) fn evidence_signature_in(
        &self,
        transaction: &MultiTransaction,
        authority: &PlanApprovalAuthority,
        session_id: &str,
        payload: &[u8],
    ) -> Result<String, std::io::Error> {
        self.require_authority_in(transaction, authority, session_id)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&authority.secret)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        mac.update(b"archon.authoritative-bash-evidence.v1\0");
        mac.update(&(payload.len() as u64).to_be_bytes());
        mac.update(payload);
        Ok(hex::encode(mac.finalize().into_bytes()))
    }

    pub(super) fn verify_evidence_signature_in(
        &self,
        transaction: &MultiTransaction,
        authority: &PlanApprovalAuthority,
        session_id: &str,
        payload: &[u8],
        signature: &str,
    ) -> Result<(), std::io::Error> {
        self.require_authority_in(transaction, authority, session_id)?;
        let signature = hex::decode(signature).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "authoritative Bash evidence signature is malformed",
            )
        })?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&authority.secret)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        mac.update(b"archon.authoritative-bash-evidence.v1\0");
        mac.update(&(payload.len() as u64).to_be_bytes());
        mac.update(payload);
        mac.verify_slice(&signature).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "authoritative Bash evidence signature verification failed",
            )
        })
    }

    /// Validate that an opaque authority is bound to this durable store and session.
    pub fn validate_approval_authority(
        &self,
        authority: &PlanApprovalAuthority,
        session_id: &str,
    ) -> Result<(), std::io::Error> {
        let transaction = self.db.multi_transaction(true);
        let result = self.require_authority_in(&transaction, authority, session_id);
        self.finish_transaction(transaction, result)
    }
}
