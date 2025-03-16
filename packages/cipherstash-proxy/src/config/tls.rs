use std::path::PathBuf;

use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use serde::Deserialize;
use tracing::debug;

use crate::log::CONFIG;

///
/// Server TLS Configuration
/// This is listener/inbound connection config
///
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
pub enum TlsConfig {
    Pem {
        certificate: String,
        private_key: String,
    },
    Path {
        certificate: String,
        private_key: String,
    },
}

#[derive(Clone, Debug, Deserialize)]
pub struct KeyCertPair {
    pub certificate: String,
    pub private_key: String,
}

impl TlsConfig {
    pub fn cert_exists(&self) -> bool {
        match self {
            TlsConfig::Pem { certificate, .. } => {
                debug!(target: CONFIG, msg = "TLS certificate is a pem string (content omitted)");
                let certs = CertificateDer::pem_slice_iter(certificate.as_bytes())
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap_or(Vec::new());
                !certs.is_empty()
            }
            TlsConfig::Path { certificate, .. } => {
                debug!(target: CONFIG, msg = "TLS certificate is a path: {}", certificate);
                PathBuf::from(certificate).exists()
            }
        }
    }

    pub fn private_key_exists(&self) -> bool {
        match self {
            TlsConfig::Pem { private_key, .. } => {
                debug!(target: CONFIG, msg = "TLS private_key is a pem string (content omitted)");
                PrivateKeyDer::from_pem_slice(private_key.as_bytes()).is_ok()
            }
            TlsConfig::Path { private_key, .. } => {
                debug!(target: CONFIG, msg = "TLS private_key is a path: {}", private_key);
                PathBuf::from(private_key).exists()
            }
        }
    }

    pub fn certificate(&self) -> &str {
        match self {
            Self::Pem { certificate, .. } | Self::Path { certificate, .. } => certificate,
        }
    }

    pub fn private_key(&self) -> &str {
        match self {
            Self::Pem { private_key, .. } | Self::Path { private_key, .. } => private_key,
        }
    }

    pub fn certificate_err_msg(&self) -> &str {
        match self {
            Self::Pem { .. } => "Transport Layer Security (TLS) Certificate is invalid",
            Self::Path { .. } => "Transport Layer Security (TLS) Certificate not found",
        }
    }

    pub fn private_key_err_msg(&self) -> &str {
        match self {
            Self::Pem { .. } => "Transport Layer Security (TLS) Private key is invalid",
            Self::Path { .. } => "Transport Layer Security (TLS) Private key not found",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config_with_path() -> TlsConfig {
        TlsConfig::Path {
            certificate: "../../tests/tls/server.cert".to_string(),
            private_key: "../../tests/tls/server.key".to_string(),
        }
    }

    fn test_config_with_invalid_path() -> TlsConfig {
        TlsConfig::Path {
            certificate: "/path/to/non-existent/file".to_string(),
            private_key: "/path/to/non-existent/file".to_string(),
        }
    }

    fn test_config_with_pem() -> TlsConfig {
        TlsConfig::Pem {
            certificate: "\
-----BEGIN CERTIFICATE-----
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
            .to_string(),
            private_key: "\
-----BEGIN PRIVATE KEY-----
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
            .to_string(),
        }
    }

    fn test_config_with_invalid_pem() -> TlsConfig {
        TlsConfig::Pem {
            certificate: "-----INVALID PEM-----".to_string(),
            private_key: "-----INVALID PEM-----".to_string(),
        }
    }

    #[test]
    fn test_tls_cert_exists_with_path() {
        assert!(test_config_with_path().cert_exists());
        assert!(!test_config_with_invalid_path().cert_exists());
    }

    #[test]
    fn test_tls_cert_exists_with_pem() {
        assert!(test_config_with_pem().cert_exists());
        assert!(!test_config_with_invalid_pem().cert_exists());
    }

    #[test]
    fn test_tls_private_key_exists_with_path() {
        assert!(test_config_with_path().private_key_exists());
        assert!(!test_config_with_invalid_path().private_key_exists());
    }

    #[test]
    fn test_tls_private_key_exists_with_pem() {
        assert!(test_config_with_pem().private_key_exists());
        assert!(!test_config_with_invalid_pem().private_key_exists());
    }
}
