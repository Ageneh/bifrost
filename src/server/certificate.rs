use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::str::FromStr;

use bifrost_api::admin::CertificateInfo;
use camino::Utf8Path;
use der::asn1::{GeneralizedTime, OctetString};
use der::oid::db::rfc4519::COMMON_NAME;
use der::oid::db::rfc5280::ID_KP_SERVER_AUTH;
use der::pem::LineEnding;
use der::{DateTime, EncodePem};
use mac_address::MacAddress;
use p256::ecdsa::DerSignature;
use p256::pkcs8::EncodePrivateKey;
use rsa::pkcs8::SubjectPublicKeyInfoRef;
use rsa::rand_core::OsRng;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::attr::AttributeTypeAndValue;
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::certificate::CertificateInner;
use x509_cert::der::{Decode, Encode};
use x509_cert::ext::pkix::{self, ExtendedKeyUsage, name::GeneralName};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::spki::SubjectPublicKeyInfoOwned;
use x509_cert::time::{Time, Validity};

use crate::error::{ApiError, ApiResult};

/// The certificate holds the private key, so only the owner may read it.
const PRIVATE_FILE_MODE: u32 = 0o600;

/// Generate a self-signed X509 certificate, closely matching the type and style
/// used by a real Philips Hue bridge.
///
/// Great care has been taken to match the real certificates as close as
/// possible. These certificates match on the following parameters:
///
///  - Style of subject name
///  - Valid from/to
///  - Type of private key, signing key, and signature type
///  - Choice of X509v3 extensions, including the order of them
///
/// Only these differences are known to remain:
///
///  - This version generates "Extended Key Usage" *with* the "critical"
///    flag set, while the real certificates (unusually) don't.
///    This seems to be a benign difference, and would roughly double
///    the code size needed.
///  - Real hue bridge certificates are signed by a "root-bridge" cert,
///    acting as a kind of CA certificate for the instance certificate.
///    This also seems to have no negative impact.
///
pub fn generate(secret_key: &p256::SecretKey, mac: MacAddress) -> ApiResult<CertificateInner> {
    let public_key = secret_key.public_key();

    let bridge_id = hue::bridge_id(mac);

    let subject = Name::from_str(&format!("CN={bridge_id},O=Philips Hue,C=NL"))?;

    /* self-signed certificate, so subject == issuer */
    let issuer = subject.clone();

    let serial_number = SerialNumber::new(&hue::bridge_id_raw(mac))?;

    /* Philips Hue seems to start their certificates at the beginning of 2017.. */
    let not_before = GeneralizedTime::from_date_time(DateTime::new(2017, 1, 1, 0, 0, 0)?).into();

    /* ..and end on the Y38K boundary (https://en.wikipedia.org/wiki/Year_2038_problem) */
    let not_after = GeneralizedTime::from_date_time(DateTime::new(2038, 1, 19, 3, 14, 7)?).into();

    let validity = Validity {
        not_before,
        not_after,
    };

    /* Use "Manual" profile, since Hue certs have an unusual combination of X509 extensions */
    let profile = Profile::Manual {
        issuer: Some(issuer.clone()),
    };

    let signer = ecdsa::SigningKey::<p256::NistP256>::from(secret_key);
    let pub_key = SubjectPublicKeyInfoOwned::from_key(public_key)?;

    /* Make certificate builder, which will allow us to build the final cert */
    let mut builder = CertificateBuilder::new(
        profile,
        serial_number.clone(),
        validity,
        subject,
        pub_key.clone(),
        &signer,
    )?;

    /* Basic constraints extension */
    builder.add_extension(&pkix::BasicConstraints {
        ca: false,
        path_len_constraint: None,
    })?;

    /* Key Usage extension */
    builder.add_extension(&pkix::KeyUsage(pkix::KeyUsages::DigitalSignature.into()))?;

    let der = pub_key.to_der()?;
    let spki = SubjectPublicKeyInfoRef::from_der(&der)?;

    /* Extended Key Usage extension */
    builder.add_extension(&ExtendedKeyUsage(vec![ID_KP_SERVER_AUTH]))?;

    /* Subject Key Identifier extension */
    builder.add_extension(&pkix::SubjectKeyIdentifier::try_from(spki.clone())?)?;

    /* Authority Key Identifier extension */
    let mut aki = pkix::AuthorityKeyIdentifier::try_from(spki.clone())?;
    aki.key_identifier = Some(OctetString::new(
        Sha1::digest(spki.subject_public_key.raw_bytes()).as_slice(),
    )?);
    aki.authority_cert_issuer = Some(vec![GeneralName::DirectoryName(issuer)]);
    aki.authority_cert_serial_number = Some(serial_number);
    builder.add_extension(&aki)?;

    /* Finally ready to build the certificate */
    Ok(builder.build::<DerSignature>()?)
}

pub fn extract_common_name(rdr: impl Read) -> ApiResult<Option<String>> {
    let bufread = &mut BufReader::new(rdr);

    for chunk in rustls_pemfile::certs(bufread) {
        let cert = Certificate::from_der(&chunk?)?;

        if let Some(cn) = common_name(&cert) {
            return Ok(Some(cn));
        }
    }

    Ok(None)
}

/// The subject common name of `cert`, which is the bridge id.
fn common_name(cert: &CertificateInner) -> Option<String> {
    for name in &cert.tbs_certificate.subject.0 {
        if let [
            AttributeTypeAndValue {
                oid: COMMON_NAME,
                value,
            },
        ] = name.0.as_slice()
        {
            return String::from_utf8(value.value().to_vec()).ok();
        }
    }

    None
}

/// Read the first certificate in `certpath`.
#[allow(clippy::result_large_err)]
fn read_certificate(certpath: &Utf8Path) -> ApiResult<(Vec<u8>, CertificateInner)> {
    let bufread = &mut BufReader::new(File::open(certpath)?);
    let chunk = rustls_pemfile::certs(bufread)
        .next()
        .ok_or_else(|| ApiError::CertificateInvalid(certpath.to_owned()))??;
    let cert = Certificate::from_der(&chunk)?;
    Ok((chunk.to_vec(), cert))
}

/// The bridge certificate as the certificate panel shows it.
///
/// `mac` is the configured address the certificate is derived from; the
/// common name it was issued with is reported as it is, so a certificate for
/// another MAC is visible rather than silently corrected.
#[allow(clippy::result_large_err)]
pub fn describe(certpath: &Utf8Path, mac: MacAddress) -> ApiResult<CertificateInfo> {
    let (der, cert) = read_certificate(certpath)?;
    let tbs = &cert.tbs_certificate;

    Ok(CertificateInfo {
        common_name: common_name(&cert)
            .ok_or_else(|| ApiError::CertificateInvalid(certpath.to_owned()))?,
        bridge_id: hue::bridge_id(mac),
        mac: mac.to_string(),
        not_before: format_time(&tbs.validity.not_before),
        not_after: format_time(&tbs.validity.not_after),
        serial: hex::encode(tbs.serial_number.as_bytes()),
        fingerprint: fingerprint(&der),
    })
}

/// Replace the bridge certificate with a freshly generated one for `mac`,
/// and describe the result.
///
/// Every paired client trusts the old certificate and has to pair again.
#[allow(clippy::result_large_err)]
pub fn regenerate(certpath: &Utf8Path, mac: MacAddress) -> ApiResult<CertificateInfo> {
    generate_and_save(certpath, mac)?;
    describe(certpath, mac)
}

/// RFC 3339 in UTC, the format the admin API uses for times.
fn format_time(time: &Time) -> String {
    let time = time.to_date_time();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        time.year(),
        time.month(),
        time.day(),
        time.hour(),
        time.minutes(),
        time.seconds()
    )
}

/// SHA-256 over the DER encoding, as `openssl x509 -fingerprint -sha256`
/// prints it.
fn fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    digest
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<String>>()
        .join(":")
}

pub fn generate_and_save(certpath: &Utf8Path, mac: MacAddress) -> ApiResult<()> {
    let secret_key = p256::SecretKey::random(&mut OsRng);
    let cert = generate(&secret_key, mac)?;
    // the file holds the private key as well, so only the owner may read it
    let mut fd = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(PRIVATE_FILE_MODE)
        .open(certpath)?;
    fd.write_all(secret_key.to_pkcs8_pem(LineEnding::LF)?.as_bytes())?;
    fd.write_all(cert.to_pem(LineEnding::LF)?.as_bytes())?;
    fd.sync_all()?;
    Ok(())
}

/// The certificate file verbatim: the private key, then the certificate.
///
/// Only the identity bundle reads this; nothing logs or displays it.
#[allow(clippy::result_large_err)]
pub fn read_pem(certpath: &Utf8Path) -> ApiResult<String> {
    Ok(std::fs::read_to_string(certpath)?)
}

/// Replace the certificate file with `pem`, written as privately as a
/// generated one.
///
/// Text that holds no certificate, or a certificate without its private key,
/// is refused: a damaged bundle must not leave the bridge unable to serve
/// HTTPS.
#[allow(clippy::result_large_err)]
pub fn save_pem(certpath: &Utf8Path, pem: &str) -> ApiResult<()> {
    if extract_common_name(pem.as_bytes())?.is_none() {
        return Err(ApiError::CertificateInvalid(certpath.to_owned()));
    }

    let key = rustls_pemfile::private_key(&mut BufReader::new(pem.as_bytes()))?;
    if key.is_none() {
        return Err(ApiError::CertificateInvalid(certpath.to_owned()));
    }

    let mut fd = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(PRIVATE_FILE_MODE)
        .open(certpath)?;
    fd.write_all(pem.as_bytes())?;
    fd.sync_all()?;
    Ok(())
}

pub fn check_certificate(certpath: &Utf8Path, mac: MacAddress) -> ApiResult<()> {
    let cn = extract_common_name(File::open(certpath)?)?;
    let id = hue::bridge_id(mac);
    match cn {
        Some(cn) => {
            if cn == id {
                log::debug!("Found existing certificate for bridge id [{id}]");
            } else {
                log::warn!("Certificate found, but common name (CN) does not match!");
                log::warn!("  [{id}] (expected)");
                log::warn!("  [{cn}] {certpath}");
                return Ok(());
            }
        }
        None => {
            return Err(ApiError::CertificateInvalid(certpath.to_owned()));
        }
    }
    Ok(())
}
