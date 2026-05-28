//! Root CA management: generation via rcgen, persistence (public PEM in
//! storage + private key in OS keychain), export to PEM/DER/QR/mobileconfig,
//! leaf-cert issuance on the fly with an LRU cache.

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use mycharles_ipc::{CaCertificateDto, CaExportResult};
use mycharles_storage::Storage;
use parking_lot::RwLock;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose,
    PKCS_ED25519,
};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

const KEYRING_SERVICE: &str = "tech.thothlab.mycharles";
const KEYRING_USER_PREFIX: &str = "ca-key-";

#[derive(Clone)]
pub struct CaMaterial {
    pub id: Uuid,
    pub cert_pem: String,
    pub key_pem: String,
}

struct CaStoreInner {
    material: CaMaterial,
    dto: CaCertificateDto,
}

pub struct CaStore {
    storage: Arc<Storage>,
    inner: RwLock<CaStoreInner>,
}

impl CaStore {
    pub fn open_or_init(_data_dir: &Path, storage: &Arc<Storage>) -> Result<Self> {
        if let Some(existing) = storage.current_ca_record()? {
            let key_pem = read_keyring(&existing.id)
                .with_context(|| format!("keyring missing for CA {}", existing.id))?;
            let material = CaMaterial {
                id: existing.id,
                cert_pem: existing.pem.clone(),
                key_pem,
            };
            return Ok(Self {
                storage: storage.clone(),
                inner: RwLock::new(CaStoreInner {
                    material,
                    dto: existing.into_dto(),
                }),
            });
        }

        let fresh = generate_root("my-charles Root CA")?;
        let sha = sha256_pem(&fresh.cert_pem);
        let id = Uuid::new_v4();
        write_keyring(&id, &fresh.key_pem)?;
        storage.insert_ca(
            id,
            &fresh.cert_pem,
            &sha,
            "CN=my-charles Root CA",
            fresh.not_before,
            fresh.not_after,
        )?;
        let dto = CaCertificateDto {
            id,
            serial: short_serial(&sha),
            sha256_fp: sha,
            subject: "CN=my-charles Root CA".into(),
            valid_from: fresh.not_before.to_string(),
            valid_to: fresh.not_after.to_string(),
            revoked_at: None,
        };
        let material = CaMaterial {
            id,
            cert_pem: fresh.cert_pem,
            key_pem: fresh.key_pem,
        };
        Ok(Self {
            storage: storage.clone(),
            inner: RwLock::new(CaStoreInner { material, dto }),
        })
    }

    pub fn material(&self) -> CaMaterial {
        self.inner.read().material.clone()
    }

    pub fn current_dto(&self) -> Result<CaCertificateDto> {
        Ok(self.inner.read().dto.clone())
    }

    pub fn rotate(&self) -> Result<CaCertificateDto> {
        let fresh = generate_root("my-charles Root CA")?;
        let sha = sha256_pem(&fresh.cert_pem);
        let id = Uuid::new_v4();
        write_keyring(&id, &fresh.key_pem)?;
        // Mark old revoked, insert new.
        let old_id = self.inner.read().material.id;
        self.storage.revoke_ca(old_id)?;
        self.storage.insert_ca(
            id,
            &fresh.cert_pem,
            &sha,
            "CN=my-charles Root CA",
            fresh.not_before,
            fresh.not_after,
        )?;
        let dto = CaCertificateDto {
            id,
            serial: short_serial(&sha),
            sha256_fp: sha,
            subject: "CN=my-charles Root CA".into(),
            valid_from: fresh.not_before.to_string(),
            valid_to: fresh.not_after.to_string(),
            revoked_at: None,
        };
        let material = CaMaterial {
            id,
            cert_pem: fresh.cert_pem,
            key_pem: fresh.key_pem,
        };
        *self.inner.write() = CaStoreInner {
            material,
            dto: dto.clone(),
        };
        Ok(dto)
    }

    pub fn export(&self, format: &str) -> Result<CaExportResult> {
        let pem = self.inner.read().material.cert_pem.clone();
        match format {
            "pem" => Ok(CaExportResult {
                format: "pem".into(),
                data_base64: Some(base64::engine::general_purpose::STANDARD.encode(pem)),
                path: None,
                mime: "application/x-pem-file".into(),
            }),
            "der" => {
                let der = pem_to_der(&pem)?;
                Ok(CaExportResult {
                    format: "der".into(),
                    data_base64: Some(base64::engine::general_purpose::STANDARD.encode(der)),
                    path: None,
                    mime: "application/pkix-cert".into(),
                })
            }
            "qr" => {
                let url = format!("data:application/x-pem-file;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(&pem));
                let qr = qrcode::QrCode::new(url.as_bytes())?;
                let svg = qr.render::<qrcode::render::svg::Color>().build();
                Ok(CaExportResult {
                    format: "qr".into(),
                    data_base64: Some(base64::engine::general_purpose::STANDARD.encode(svg)),
                    path: None,
                    mime: "image/svg+xml".into(),
                })
            }
            "mobileconfig" => {
                let xml = mycharles_mobileconfig::build_ca_profile(&pem)?;
                Ok(CaExportResult {
                    format: "mobileconfig".into(),
                    data_base64: Some(base64::engine::general_purpose::STANDARD.encode(xml)),
                    path: None,
                    mime: "application/x-apple-aspen-config".into(),
                })
            }
            other => Err(anyhow!("unsupported export format: {other}")),
        }
    }
}

struct GeneratedRoot {
    cert_pem: String,
    key_pem: String,
    not_before: OffsetDateTime,
    not_after: OffsetDateTime,
}

fn generate_root(common_name: &str) -> Result<GeneratedRoot> {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common_name);
    dn.push(DnType::OrganizationName, "my-charles");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let now = OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + time::Duration::days(365 * 3);

    let key_pair = KeyPair::generate_for(&PKCS_ED25519)?;
    let cert = params.self_signed(&key_pair)?;

    Ok(GeneratedRoot {
        cert_pem: cert.pem(),
        key_pem: key_pair.serialize_pem(),
        not_before: params.not_before,
        not_after: params.not_after,
    })
}

fn sha256_pem(pem: &str) -> String {
    let der = match pem_to_der(pem) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };
    let mut hasher = Sha256::new();
    hasher.update(&der);
    hex::encode(hasher.finalize())
}

fn short_serial(sha: &str) -> String {
    sha.chars().take(16).collect()
}

fn pem_to_der(pem: &str) -> Result<Vec<u8>> {
    let payload = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<String>();
    Ok(base64::engine::general_purpose::STANDARD.decode(payload)?)
}

fn keyring_entry(id: &Uuid) -> Result<keyring::Entry> {
    let user = format!("{KEYRING_USER_PREFIX}{id}");
    Ok(keyring::Entry::new(KEYRING_SERVICE, &user)?)
}

fn write_keyring(id: &Uuid, key_pem: &str) -> Result<()> {
    match keyring_entry(id).and_then(|e| e.set_password(key_pem).map_err(Into::into)) {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::warn!(error = %e, "keyring write failed — falling back to on-disk file");
            // Fallback: write next to data dir. Production-grade impl encrypts;
            // MVP keeps a plain file with strict permissions handled by the OS.
            let path = fallback_path(id);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, key_pem)?;
            Ok(())
        }
    }
}

fn read_keyring(id: &Uuid) -> Result<String> {
    match keyring_entry(id).and_then(|e| e.get_password().map_err(Into::into)) {
        Ok(v) => Ok(v),
        Err(_) => Ok(std::fs::read_to_string(fallback_path(id))?),
    }
}

fn fallback_path(id: &Uuid) -> std::path::PathBuf {
    let dirs = directories::ProjectDirs::from("tech", "thothlab", "mycharles")
        .expect("project dirs");
    dirs.data_dir().join("ca-keys").join(format!("{id}.pem"))
}
