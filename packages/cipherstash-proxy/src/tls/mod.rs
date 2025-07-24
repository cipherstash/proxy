use crate::{config::TlsConfig, error::Error};
use crate::{DatabaseConfig, TandemConfig};
use rustls::client::danger::ServerCertVerifier;
use rustls::ClientConfig;
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, ServerName};
use rustls_platform_verifier::ConfigVerifierExt;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

///
/// Create a Server TLS connection
/// The returned type is the higher-level TlsStream that wraps both Client & Server variants
///
pub async fn client(
    stream: TcpStream,
    config: &TandemConfig,
) -> Result<TlsStream<TcpStream>, Error> {
    let tls_config = configure_client(&config.database);
    let connector = TlsConnector::from(Arc::new(tls_config));
    let domain = config.database.server_name()?.to_owned();
    let tls_stream = connector.connect(domain, stream).await?;

    Ok(tls_stream.into())
}

///
/// Create a Server TLS connection
/// The returned type is the higher-level TlsStream that wraps both Client & Server variants
///
pub async fn server(stream: TcpStream, config: &TlsConfig) -> Result<TlsStream<TcpStream>, Error> {
    let server_config = configure_server(config)?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let tls_stream = acceptor.accept(stream).await?;

    Ok(tls_stream.into())
}

///
/// Configure the server TLS settings
/// These are the settings for the listener
///
/// Depending on whether the config is Pem or Path, this function will try to parse certificate and
/// private_key from the string contents or the file contents.
///
pub fn configure_server(config: &TlsConfig) -> Result<rustls::ServerConfig, Error> {
    let certs = match config {
        TlsConfig::Pem {
            certificate_pem: certificate,
            ..
        } => {
            CertificateDer::pem_slice_iter(certificate.as_bytes()).collect::<Result<Vec<_>, _>>()?
        }
        TlsConfig::Path {
            certificate_path: certificate,
            ..
        } => CertificateDer::pem_file_iter(certificate)?.collect::<Result<Vec<_>, _>>()?,
    };

    let key = match config {
        TlsConfig::Pem {
            private_key_pem: private_key,
            ..
        } => PrivateKeyDer::from_pem_slice(private_key.as_bytes()),
        TlsConfig::Path {
            private_key_path: private_key,
            ..
        } => PrivateKeyDer::from_pem_file(private_key),
    }?;

    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(server_config)
}

///
/// Configure the client TLS settings
/// These are the settings for connecting to the database with TLS
/// The client will use the system root certificates
///
pub fn configure_client(config: &DatabaseConfig) -> ClientConfig {
    let mut tls_config = ClientConfig::with_platform_verifier();

    if !config.with_tls_verification {
        let mut dangerous = tls_config.dangerous();
        dangerous.set_certificate_verifier(Arc::new(NoCertificateVerification {}));
    }

    tls_config
}

#[derive(Clone, Debug)]
pub struct NoCertificateVerification {}

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls_pki_types::CertificateDer<'_>,
        _intermediates: &[rustls_pki_types::CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn certificate_pem() -> String {
        "-----BEGIN CERTIFICATE-----
MIIDKzCCAhOgAwIBAgIUMXfu7Mj22j+e9Gt2gjV73TBg20wwDQYJKoZIhvcNAQEL
BQAwFDESMBAGA1UEAwwJbG9jYWxob3N0MB4XDTI1MDEyNjAxNDkzMVoXDTI2MDEy
NjAxNDkzMVowFDESMBAGA1UEAwwJbG9jYWxob3N0MIIBIjANBgkqhkiG9w0BAQEF
AAOCAQ8AMIIBCgKCAQEApuqOqv0P8IPe7TmQGO2HeO0DjPrIVyYYCtJXEyUhPSuq
20ePjb6PyCeAlG873fJW4+fSF6YP0nsb7PJQYYa7E5CxJNYtjJ9c19l0CpgkNmHP
ogK8HhAokvjxKGTwidj37KAVBXznaWPfFvf8SuS2xFSCknTou2m67Q68rCYM8h9e
AjB5L0kL2bM6ySgGt5m0lWsr73duaGrLEJxfjV+JVitDSqLqbeDWOKXHfaKBBwL1
BZWyl4f4MM0rhLoDcbGOYIlkZtadB2lqdaFqfuejIcmZd/Q41mRhNmwNnG9guSWC
YHMdPkIrAaZNZlL0drIeTVgPcVkP8lPEkXsxHhmybwIDAQABo3UwczAdBgNVHQ4E
FgQUWQ8oySVGv/BhOr1B6zMVyNDeobkwHwYDVR0jBBgwFoAUWQ8oySVGv/BhOr1B
6zMVyNDeobkwDAYDVR0TAQH/BAIwADAOBgNVHQ8BAf8EBAMCBaAwEwYDVR0lBAww
CgYIKwYBBQUHAwEwDQYJKoZIhvcNAQELBQADggEBAFzLi09kyRBE/H3RarjQdolv
eAPwpn16YqUgppYjKIbPHx6QtXBElhhqTlW104x8CMzx3pT0wBIaUPmhWj6DeWET
TZIDbXhWiMRhsB7cup7y5O9mlXvST4fyrcD30rgfO8XAL8nJLsAbCgL/BWlptC1m
2tFtY1Y8bYTD04TMVVVA20rvwwINg1Gd+JYRoHysBvnGuespMVuW0Ji49U7OWPp/
Iwy49Eyr7U0xg2VFPNBkNUmw6MQQVumt3OBydAKmd3XAJy/Nmzq/ZHvL3jdl1jlC
TU/T2RF2sDsSHrUIVMeifhYc0jfNlRwnUG5liN9BiGo1QxNZ9jGY/3ts5eu8+XM=
-----END CERTIFICATE-----
"
        .to_string()
    }

    fn private_key_pem() -> String {
        "-----BEGIN PRIVATE KEY-----
MIIEugIBADANBgkqhkiG9w0BAQEFAASCBKQwggSgAgEAAoIBAQCm6o6q/Q/wg97t
OZAY7Yd47QOM+shXJhgK0lcTJSE9K6rbR4+Nvo/IJ4CUbzvd8lbj59IXpg/Sexvs
8lBhhrsTkLEk1i2Mn1zX2XQKmCQ2Yc+iArweECiS+PEoZPCJ2PfsoBUFfOdpY98W
9/xK5LbEVIKSdOi7abrtDrysJgzyH14CMHkvSQvZszrJKAa3mbSVayvvd25oassQ
nF+NX4lWK0NKoupt4NY4pcd9ooEHAvUFlbKXh/gwzSuEugNxsY5giWRm1p0HaWp1
oWp+56MhyZl39DjWZGE2bA2cb2C5JYJgcx0+QisBpk1mUvR2sh5NWA9xWQ/yU8SR
ezEeGbJvAgMBAAECgf8E32YqIqznJ9ZwvCIg2FBdc1fHRFJ78Few64VugtCMwVu8
/fCsDTVzIOHR7dXlK5z7JY1VCURxInql5uwYsfIbcvfdtdt8GNL2tiNs7WHryZRP
CVgcnCkQ++Koy4RcjbI9FEgQPjPLFK8Hx8JDvG90nSfIVMkp34t3Hs4Hu8IRr5Cq
dv1PsYzoa2DJb/gsed7fefm7MQ2SGH0r9TrA+rzUx3Vb05z5Wi/AEsCReLaWbplJ
ARwQCcfvMOAA3CvDkLH2m1J64EqS/vt6fmiE9x8KOU9OZ0FK6pP8evvHpkyaopqN
59DcNzDvGVyxLtwJ6JoQXLsoZywHIjah+eGu6ikCgYEA1TT2Sgx2E+4NOefPvuMg
DkT/3EYnXEOufI+rrr01J84gn1IuukC4nfKxel5KgVhMxZHHmB25Kp8G9tdDgVMd
qHdT5oMZgYAW7+vtQOWf8Px7P80WvN38LlI/v2bngSPnNhrg3MsBzpqnXtOlBFfR
Zq3PhWkwzCnSvuSbLELszOsCgYEAyGsUjcFFyF/so9FA6rrNplwisUy3ykBO98Ye
KIa5Dz3UsGqYraqk59MIC5f1BdeYRlVKUNlxcmT089goc0MxwKbqJHJdTVqWrnnK
o5+jAddv/awbuMYbt+//zM296SyXgi8y6eUt6TN8ss4NztpcxzBNmCrny8s6Xd9K
OqX9P40CgYBhE4xQivv4dxtuki31LFUcKi6VjRu+1tJLxN7W4S+iwCf6YuEDzRRC
Vo6YuPYTjrDmBEps6Ju23FG/cqQ57i5C1pJNEsQ6Qqgu9a1BL0xz3YIAutDvjeOU
874y2BfwpPhRmktoPMbF24T5mEQ6hgHCTsF+bTbavvBGGrDMpmxLoQKBgFjsWeRD
esja9s4AjEMZuyEzBBmSpoFQYzlAaCUnEXkXwAS+Zxu2+Q/67DjopUiATgn20dBp
ihJthNmkcN4jVDHcXUrqi0dFCFJFq4lJzTOF+SSednZXP/kuvVqLdtW8eUTD2F06
2FH+DDfxgOLktAGVBvibINmlRDJeXjsDZwgJAoGAOL28xi4UqaFOu4CbB5BvCIxN
l0AUk9ZCx4hOwE7BUqG9winPtmwqoXGtMuamlKf7vxONhg68EHFyDuMxL8rgHjrH
eq8W0CchxrihmoEm6zGtDbrdJ6KkbhyeFJgZPKX8Nff7Nsi7FJyea53CCv3B5aQr
B+qwsnNEiDoJhgYj+cQ=
-----END PRIVATE KEY-----
"
        .to_string()
    }

    #[test]
    fn test_configure_server_with_paths() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let tls_config = TlsConfig::Path {
            private_key_path: "../../tests/tls/server.key".to_string(),
            certificate_path: "../../tests/tls/server.cert".to_string(),
        };
        let server_config = configure_server(&tls_config);

        assert!(server_config.is_ok());
    }

    #[test]
    fn test_configure_server_with_path_for_pem() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let tls_config = TlsConfig::Pem {
            private_key_pem: "../../tests/tls/server.key".to_string(),
            certificate_pem: "../../tests/tls/server.cert".to_string(),
        };
        let server_config = configure_server(&tls_config);

        assert!(server_config.is_err());
    }

    #[test]
    fn test_configure_server_with_pem_for_path() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let tls_config = TlsConfig::Path {
            private_key_path: private_key_pem(),
            certificate_path: certificate_pem(),
        };
        let server_config = configure_server(&tls_config);

        assert!(server_config.is_err());
    }

    #[test]
    fn test_configure_server_with_pems() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let tls_config = TlsConfig::Pem {
            private_key_pem: private_key_pem(),
            certificate_pem: certificate_pem(),
        };
        let server_config = configure_server(&tls_config);

        assert!(server_config.is_ok());
    }
}
