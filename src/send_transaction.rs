use std::net::{SocketAddr, ToSocketAddrs as _};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use quinn::{
    crypto::rustls::QuicClientConfig, ClientConfig, Connection, Endpoint, IdleTimeout,
    TransportConfig,
};
use rand::seq::IndexedRandom as _;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    instruction::Instruction,
    pubkey,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    transaction::Transaction,
};
use solana_tls_utils::{SkipServerVerification, new_dummy_x509_certificate};
use tokio::sync::Mutex;
use tracing::{info, warn};

const ALPN_TPU_PROTOCOL_ID: &[u8] = b"solana-tpu";
const SOLAMI_SERVER: &str = "solami-landing";
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(25);
const MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

pub const SOLAMI_ENDPOINTS: &[&str] = &[
    "nyc.landing.solami.fast:11000",
    "fra.landing.solami.fast:11000",
    "ams.landing.solami.fast:11000",
    "sgp.landing.solami.fast:11000",
    "landing.solami.fast:11000",
];

pub const SOLAMI_TIP_ACCOUNTS: &[Pubkey] = &[
    pubkey!("9N1JafTRq7jk8G3GLCV6yDjoiRLDj4mvmtGpcHTMKami"),
    pubkey!("ENKoQ3gq2kbqgmvuZVih8CHBmYzZHpxs27ZRKreNxami"),
    pubkey!("6kXV3jz3Zey4qqo5fL6EyG1tPqnwRZ9PxUDyxYX6iami"),
];
pub fn build_tip_ix(payer: &Pubkey, lamports: u64) -> Instruction {
    let tip_account = *SOLAMI_TIP_ACCOUNTS
        .choose(&mut rand::rng())
        .unwrap_or(&SOLAMI_TIP_ACCOUNTS[0]);

    solana_sdk::system_instruction::transfer(payer, &tip_account, lamports)
}

pub struct SolamiSender {
    endpoint: Endpoint,
    client_config: ClientConfig,
    addr: SocketAddr,
    connection: ArcSwap<Connection>,
    reconnect: Mutex<()>,
}

impl SolamiSender {
    pub async fn new(swqos_key: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let keypair = Keypair::from_base58_string(swqos_key);
        let (cert, key) = new_dummy_x509_certificate(&keypair);

        let mut crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(SkipServerVerification::new())
            .with_client_auth_cert(vec![cert], key)?;

        crypto.alpn_protocols = vec![ALPN_TPU_PROTOCOL_ID.to_vec()];

        let client_crypto = QuicClientConfig::try_from(crypto)
            .map_err(|e| format!("quinn crypto config: {e}"))?;

        let mut client_config = ClientConfig::new(Arc::new(client_crypto));
        let mut transport = TransportConfig::default();
        transport.keep_alive_interval(Some(KEEP_ALIVE_INTERVAL));
        transport.max_idle_timeout(Some(IdleTimeout::try_from(MAX_IDLE_TIMEOUT)?));
        client_config.transport_config(Arc::new(transport));

        let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
        endpoint.set_default_client_config(client_config.clone());
        let landing = *SOLAMI_ENDPOINTS
            .choose(&mut rand::rng())
            .unwrap_or(&SOLAMI_ENDPOINTS[0]);

        let addr = landing
            .to_socket_addrs()?
            .next()
            .ok_or("failed to resolve solami endpoint")?;

        info!(%landing, "connecting to solami landing");
        let connection = endpoint.connect(addr, SOLAMI_SERVER)?.await?;
        info!("solami QUIC connection established");

        Ok(Self {
            endpoint,
            client_config,
            addr,
            connection: ArcSwap::from_pointee(connection),
            reconnect: Mutex::new(()),
        })
    }

    async fn reconnect(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _guard = self
            .reconnect
            .try_lock()
            .map_err(|_| "reconnect already in progress")?;
        let connection = self
            .endpoint
            .connect_with(self.client_config.clone(), self.addr, SOLAMI_SERVER)?
            .await?;
        self.connection.store(Arc::new(connection));
        info!("solami reconnected");
        Ok(())
    }

    async fn try_send_bytes(
        connection: &Connection,
        payload: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut stream = connection.open_uni().await?;
        stream.write_all(payload).await?;
        stream.finish()?;
        Ok(())
    }

    pub async fn send_transaction(
        &self,
        tx: &Transaction,
    ) -> Result<Signature, Box<dyn std::error::Error + Send + Sync>> {

        info!("Sending via SWQoS");
        let sig = *tx.signatures.first().ok_or("transaction has no signature")?;
        let serialized = bincode::serialize(tx)?;

        let connection = self.connection.load_full();
        if Self::try_send_bytes(&connection, &serialized).await.is_err() {
            warn!("solami send failed, reconnecting");
            self.reconnect().await?;
            let connection = self.connection.load_full();
            Self::try_send_bytes(&connection, &serialized)
                .await
                .map_err(|e| format!("solami send after reconnect: {e}"))?;
        }

        info!(%sig, "tx sent via solami QUIC");
        Ok(sig)
    }
}
pub async fn send_transaction_rpc(
    rpc: &RpcClient,
    tx: &Transaction,
) -> Result<Signature, Box<dyn std::error::Error + Send + Sync>> {
    let sig = rpc
        .send_transaction_with_config(
            tx,
            solana_client::rpc_config::RpcSendTransactionConfig {
                skip_preflight: true,
                ..Default::default()
            },
        )
        .await?;
    info!(%sig, "tx sent via RPC");
    Ok(sig)
}
pub async fn poll_confirmation(
    rpc: &RpcClient,
    signature: Signature,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > timeout {
            return Err(format!("confirmation timeout for {signature}").into());
        }
        let status = rpc.get_signature_statuses(&[signature]).await?;
        if let Some(Some(status)) = status.value.first() {
            if status.err.is_some() {
                return Err(format!("tx {signature} failed: {:?}", status.err).into());
            }
            if matches!(
                status.confirmation_status.as_ref().map(|s| format!("{s:?}")),
                Some(s) if s == "Confirmed" || s == "Finalized"
            ) {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
