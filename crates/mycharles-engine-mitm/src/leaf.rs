//! On-the-fly leaf certificate issuance with an in-memory cache keyed by SNI.

use std::collections::HashMap;
use std::sync::Arc;

use mycharles_ca::CaMaterial;
use parking_lot::Mutex;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType, PKCS_ED25519};

pub struct LeafCert {
    pub cert_pem: String,
    pub key_pem: String,
}

pub struct LeafCache {
    ca: Arc<CaMaterial>,
    cache: Mutex<HashMap<String, Arc<LeafCert>>>,
}

impl LeafCache {
    pub fn new(ca: Arc<CaMaterial>) -> Self {
        Self { ca, cache: Mutex::new(HashMap::new()) }
    }

    pub fn issue(&self, sni: &str) -> anyhow::Result<Arc<LeafCert>> {
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
        params.subject_alt_names.push(SanType::DnsName(sni.try_into()?));
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(1);
        params.not_after = now + time::Duration::days(90);

        let leaf_kp = KeyPair::generate_for(&PKCS_ED25519)?;
        let leaf_cert = params.signed_by(&leaf_kp, &issuer, &issuer_kp)?;

        let result = Arc::new(LeafCert {
            cert_pem: leaf_cert.pem(),
            key_pem: leaf_kp.serialize_pem(),
        });
        self.cache.lock().insert(sni.to_string(), result.clone());
        Ok(result)
    }
}
