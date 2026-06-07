//! On-the-fly leaf certificate issuance with an in-memory cache keyed by SNI.
//!
//! Leaf keys are ECDSA P-256: Ed25519 server certs are not accepted by most
//! TLS stacks (iOS/Android system trust, OkHttp, older Chromium), so an
//! Ed25519 leaf would hand-shake-fail with real mobile clients even though the
//! root CA is trusted. P-256 is universally supported and ~same key size.

use std::collections::HashMap;
use std::sync::Arc;

use pane_ca::CaMaterial;
use parking_lot::Mutex;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, KeyPair, SanType, PKCS_ECDSA_P256_SHA256,
};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;

pub struct LeafCache {
    ca: Arc<CaMaterial>,
    cache: Mutex<HashMap<String, Arc<CertifiedKey>>>,
}

impl std::fmt::Debug for LeafCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeafCache")
            .field("cached", &self.cache.lock().len())
            .finish()
    }
}

impl LeafCache {
    pub fn new(ca: Arc<CaMaterial>) -> Self {
        Self {
            ca,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn resolve_sni(&self, sni: &str) -> anyhow::Result<Arc<CertifiedKey>> {
        if let Some(c) = self.cache.lock().get(sni).cloned() {
            return Ok(c);
        }

        let issuer_kp = KeyPair::from_pem(&self.ca.key_pem)?;
        let issuer_params = CertificateParams::from_ca_cert_pem(&self.ca.cert_pem)?;
        let issuer = issuer_params.self_signed(&issuer_kp)?;

        let mut params = CertificateParams::new(vec![sni.to_string()])?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, sni);
        params.distinguished_name = dn;
        params
            .subject_alt_names
            .push(SanType::DnsName(sni.try_into()?));
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(1);
        params.not_after = now + time::Duration::days(90);

        let leaf_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let leaf_cert = params.signed_by(&leaf_kp, &issuer, &issuer_kp)?;

        let cert_der = CertificateDer::from(leaf_cert.der().to_vec());
        let key_der = PrivatePkcs8KeyDer::from(leaf_kp.serialize_der());
        let signing_key = CryptoProvider::get_default()
            .ok_or_else(|| anyhow::anyhow!("rustls CryptoProvider not installed"))?
            .key_provider
            .load_private_key(PrivateKeyDer::Pkcs8(key_der))?;

        let certified = Arc::new(CertifiedKey::new(vec![cert_der], signing_key));
        self.cache.lock().insert(sni.to_string(), certified.clone());
        Ok(certified)
    }
}

impl ResolvesServerCert for LeafCache {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let sni = hello.server_name()?;
        match self.resolve_sni(sni) {
            Ok(k) => Some(k),
            Err(e) => {
                tracing::warn!(sni, error = %e, "leaf issuance failed");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, PKCS_ED25519};

    fn test_ca() -> CaMaterial {
        let kp = KeyPair::generate_for(&PKCS_ED25519).unwrap();
        let mut params = CertificateParams::new(vec!["test-ca".into()]).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let cert = params.self_signed(&kp).unwrap();
        CaMaterial {
            id: uuid::Uuid::new_v4(),
            cert_pem: cert.pem(),
            key_pem: kp.serialize_pem(),
        }
    }

    #[test]
    fn issues_and_caches_leaf() {
        // install_default is idempotent across tests in the same process.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let cache = LeafCache::new(Arc::new(test_ca()));
        let k1 = cache.resolve_sni("example.com").expect("first issue");
        assert!(!k1.cert.is_empty());
        let k2 = cache.resolve_sni("example.com").expect("cached");
        assert!(Arc::ptr_eq(&k1, &k2), "second resolve should hit the cache");
    }
}
