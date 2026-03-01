use anyhow::Result;
use rustls::ServerConfig;
use rustls_pemfile::{certs, pkcs8_private_keys};
use std::{fs::File, io::BufReader, sync::Arc};
use tokio_rustls::TlsAcceptor;

pub fn build_acceptor(cert_path: &str, key_path: &str) -> Result<TlsAcceptor> {
    let cert_chain: Vec<_> = certs(&mut BufReader::new(File::open(cert_path)?))
        .collect::<std::result::Result<_, _>>()?;
    anyhow::ensure!(!cert_chain.is_empty(), "no certificates found in {}", cert_path);
    let mut keys: Vec<_> = pkcs8_private_keys(&mut BufReader::new(File::open(key_path)?))
        .collect::<std::result::Result<_, _>>()?;
    anyhow::ensure!(!keys.is_empty(), "no PKCS8 keys found in {}", key_path);
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, rustls::pki_types::PrivateKeyDer::Pkcs8(keys.remove(0)))?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}
