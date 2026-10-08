//! Ephemeral TLS credentials shared by HTTP transport tests.
use openssl::asn1::Asn1Time;
use openssl::bn::{BigNum, MsbOption};
use openssl::hash::MessageDigest;
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;
use openssl::x509::extension::{
    BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectAlternativeName,
};
use openssl::x509::{X509, X509NameBuilder};

pub struct Material {
    pub dir: tempfile::TempDir,
    pub ca: Vec<u8>,
    pub client_identity: Vec<u8>,
}

fn key() -> PKey<Private> {
    PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap()
}
fn certificate(key: &PKey<Private>, issuer: Option<(&X509, &PKey<Private>)>, client: bool) -> X509 {
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text(
        "CN",
        if issuer.is_none() {
            "Ephemeral test CA"
        } else {
            "localhost"
        },
    )
    .unwrap();
    let name = name.build();
    let mut builder = X509::builder().unwrap();
    builder.set_version(2).unwrap();
    let mut serial = BigNum::new().unwrap();
    serial.rand(128, MsbOption::MAYBE_ZERO, false).unwrap();
    builder
        .set_serial_number(&serial.to_asn1_integer().unwrap())
        .unwrap();
    builder.set_subject_name(&name).unwrap();
    builder
        .set_issuer_name(issuer.map_or(name.as_ref(), |(ca, _)| ca.subject_name()))
        .unwrap();
    builder.set_pubkey(key).unwrap();
    builder
        .set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    builder
        .set_not_after(&Asn1Time::days_from_now(2).unwrap())
        .unwrap();
    if let Some((ca, _)) = issuer {
        builder
            .append_extension(BasicConstraints::new().critical().build().unwrap())
            .unwrap();
        builder
            .append_extension(
                KeyUsage::new()
                    .digital_signature()
                    .key_encipherment()
                    .build()
                    .unwrap(),
            )
            .unwrap();
        let usage = if client {
            ExtendedKeyUsage::new().client_auth().build()
        } else {
            ExtendedKeyUsage::new().server_auth().build()
        };
        builder.append_extension(usage.unwrap()).unwrap();
        if !client {
            let san = SubjectAlternativeName::new()
                .ip("127.0.0.1")
                .dns("localhost")
                .build(&builder.x509v3_context(Some(ca), None))
                .unwrap();
            builder.append_extension(san).unwrap();
        }
    } else {
        builder
            .append_extension(BasicConstraints::new().critical().ca().build().unwrap())
            .unwrap();
        builder
            .append_extension(
                KeyUsage::new()
                    .critical()
                    .key_cert_sign()
                    .crl_sign()
                    .build()
                    .unwrap(),
            )
            .unwrap();
    }
    builder
        .sign(issuer.map_or(key, |(_, key)| key), MessageDigest::sha256())
        .unwrap();
    builder.build()
}

pub fn generate() -> Material {
    let dir = tempfile::tempdir().unwrap();
    let ca_key = key();
    let ca = certificate(&ca_key, None, false);
    let server_key = key();
    let server = certificate(&server_key, Some((&ca, &ca_key)), false);
    let client_key = key();
    let client = certificate(&client_key, Some((&ca, &ca_key)), true);
    let ca = ca.to_pem().unwrap();
    let mut client_identity = client.to_pem().unwrap();
    client_identity.extend(client_key.private_key_to_pem_pkcs8().unwrap());
    std::fs::write(dir.path().join("ca.pem"), &ca).unwrap();
    std::fs::write(dir.path().join("server.pem"), server.to_pem().unwrap()).unwrap();
    std::fs::write(
        dir.path().join("server-key.pem"),
        server_key.private_key_to_pem_pkcs8().unwrap(),
    )
    .unwrap();
    Material {
        dir,
        ca,
        client_identity,
    }
}
