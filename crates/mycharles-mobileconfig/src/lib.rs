//! Builder for Apple `.mobileconfig` profiles.
//!
//! Two flavours:
//!  - `build_ca_profile`: just our root CA, used when the user already has a
//!    proxy configured.
//!  - `build_full_profile`: CA + a Wi-Fi global HTTP proxy payload pointing at
//!    the desktop. Pushed during USB setup.
//!
//! Signing the profile is optional; unsigned profiles install fine but show
//! "Unverified" on iOS. Self-signing is a follow-up task — we keep the XML
//! generation pure so a signing step can wrap it.

use anyhow::Result;
use base64::Engine as _;
use uuid::Uuid;

pub fn build_ca_profile(ca_pem: &str) -> Result<String> {
    Ok(profile_xml(&[ca_payload(ca_pem)]))
}

pub fn build_full_profile(ca_pem: &str, proxy_host: &str, proxy_port: u16) -> Result<String> {
    Ok(profile_xml(&[
        ca_payload(ca_pem),
        proxy_payload(proxy_host, proxy_port),
    ]))
}

fn ca_payload(ca_pem: &str) -> String {
    let der_b64 = pem_to_der_b64(ca_pem);
    let uuid = Uuid::new_v4();
    format!(
        r#"  <dict>
    <key>PayloadCertificateFileName</key><string>my-charles-root.crt</string>
    <key>PayloadContent</key>
    <data>
{der_b64}
    </data>
    <key>PayloadDescription</key><string>my-charles root CA</string>
    <key>PayloadDisplayName</key><string>my-charles CA</string>
    <key>PayloadIdentifier</key><string>tech.thothlab.mycharles.ca.{uuid}</string>
    <key>PayloadType</key><string>com.apple.security.root</string>
    <key>PayloadUUID</key><string>{uuid}</string>
    <key>PayloadVersion</key><integer>1</integer>
  </dict>"#
    )
}

fn proxy_payload(host: &str, port: u16) -> String {
    let uuid = Uuid::new_v4();
    format!(
        r#"  <dict>
    <key>PayloadType</key><string>com.apple.proxy.http.global</string>
    <key>PayloadIdentifier</key><string>tech.thothlab.mycharles.proxy.{uuid}</string>
    <key>PayloadUUID</key><string>{uuid}</string>
    <key>PayloadDisplayName</key><string>my-charles proxy</string>
    <key>PayloadDescription</key><string>Routes device traffic through my-charles</string>
    <key>PayloadVersion</key><integer>1</integer>
    <key>ProxyType</key><string>Manual</string>
    <key>ProxyServer</key><string>{host}</string>
    <key>ProxyServerPort</key><integer>{port}</integer>
    <key>ProxyCaptiveLoginAllowed</key><false/>
  </dict>"#
    )
}

fn profile_xml(payloads: &[String]) -> String {
    let top_uuid = Uuid::new_v4();
    let body = payloads.join("\n");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>PayloadContent</key>
  <array>
{body}
  </array>
  <key>PayloadDescription</key><string>my-charles HTTPS inspection profile</string>
  <key>PayloadDisplayName</key><string>my-charles</string>
  <key>PayloadIdentifier</key><string>tech.thothlab.mycharles.profile.{top_uuid}</string>
  <key>PayloadOrganization</key><string>my-charles</string>
  <key>PayloadRemovalDisallowed</key><false/>
  <key>PayloadType</key><string>Configuration</string>
  <key>PayloadUUID</key><string>{top_uuid}</string>
  <key>PayloadVersion</key><integer>1</integer>
</dict>
</plist>
"#
    )
}

fn pem_to_der_b64(pem: &str) -> String {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    // Indent for readability inside the <data> block.
    body.as_bytes()
        .chunks(60)
        .map(std::str::from_utf8)
        .filter_map(Result::ok)
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ca_profile_contains_payload() {
        let pem = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
        let xml = build_ca_profile(pem).unwrap();
        assert!(xml.contains("com.apple.security.root"));
        assert!(xml.contains("my-charles"));
    }

    #[test]
    fn full_profile_contains_proxy() {
        let pem = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
        let xml = build_full_profile(pem, "192.168.1.1", 8888).unwrap();
        assert!(xml.contains("ProxyServer"));
        assert!(xml.contains("192.168.1.1"));
        assert!(xml.contains("<integer>8888</integer>"));
    }
}
