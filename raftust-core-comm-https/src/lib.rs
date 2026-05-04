use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, OnceLock};
use std::thread;

use axum::{Json, Router, extract::State, routing::post};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use log::{debug, error, info, warn};
use reqwest::blocking::Client;
use rustls::ServerConfig;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use raftust_core::{
    CommunicationError, InboundMessage, NodeId, RaftCommunication, RaftMessage, SendOutcome,
};

mod wire;

use wire::WireMessage;

static RUSTLS_PROVIDER_INSTALL_RESULT: OnceLock<Result<(), String>> = OnceLock::new();

fn ensure_rustls_provider_installed() -> Result<(), String> {
    RUSTLS_PROVIDER_INSTALL_RESULT
        .get_or_init(|| {
            if rustls::crypto::aws_lc_rs::default_provider()
                .install_default()
                .is_ok()
            {
                return Ok(());
            }

            if rustls::crypto::ring::default_provider()
                .install_default()
                .is_ok()
            {
                return Ok(());
            }

            Err("failed to install a rustls crypto provider".to_string())
        })
        .clone()
}

pub struct HttpsCommunication {
    local_id: NodeId,
    peer_urls: HashMap<NodeId, String>,
    inbound_rx: Option<Receiver<WireMessage>>,
    http_client: Option<Client>,
}

impl HttpsCommunication {
    pub fn new(local_id: NodeId, peer_addrs: HashMap<NodeId, String>) -> Self {
        let peer_urls = peer_addrs
            .into_iter()
            .map(|(id, addr)| (id, format!("https://{}/raft", addr)))
            .collect();

        Self {
            local_id,
            peer_urls,
            inbound_rx: None,
            http_client: None,
        }
    }
}

async fn handle_raft(
    State(tx): State<Arc<SyncSender<WireMessage>>>,
    Json(msg): Json<WireMessage>,
) -> axum::http::StatusCode {
    if let Err(err) = tx.try_send(msg) {
        warn!("event=https_listener_queue_full err={}", err);
    }
    axum::http::StatusCode::OK
}

impl RaftCommunication for HttpsCommunication {
    fn start(&mut self, address: String) -> Result<(), CommunicationError> {
        ensure_rustls_provider_installed().map_err(CommunicationError::Other)?;

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .map_err(|e| CommunicationError::Other(format!("cert generation: {}", e)))?;
        let cert_der = cert.cert.der().to_vec();
        let key_der = cert.key_pair.serialize_der();

        let (tx, rx) = mpsc::sync_channel::<WireMessage>(256);
        let tx = Arc::new(tx);
        let listen_address = address.clone();

        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build tokio runtime for HTTPS server");

            rt.block_on(async move {
                info!("event=https_listener_start addr={}", listen_address);
                let cert_chain = vec![rustls::pki_types::CertificateDer::from(cert_der)];
                let private_key =
                    rustls::pki_types::PrivateKeyDer::try_from(key_der).expect("valid private key");

                let server_config = ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(cert_chain, private_key)
                    .expect("build TLS server config");

                let tls_acceptor = TlsAcceptor::from(Arc::new(server_config));

                let router = Router::new()
                    .route("/raft", post(handle_raft))
                    .with_state(Arc::clone(&tx));

                let addr: SocketAddr = listen_address.parse().expect("parse listen address");
                let listener = TcpListener::bind(addr).await.expect("bind HTTPS listener");

                loop {
                    let (tcp_stream, _) = match listener.accept().await {
                        Ok(v) => v,
                        Err(e) => {
                            warn!("event=https_listener_accept_error addr={} err={}", addr, e);
                            continue;
                        }
                    };

                    let tls_acceptor = tls_acceptor.clone();
                    let svc = router.clone();

                    tokio::spawn(async move {
                        let tls_stream = match tls_acceptor.accept(tcp_stream).await {
                            Ok(s) => s,
                            Err(e) => {
                                warn!("event=https_listener_tls_handshake_error err={}", e);
                                return;
                            }
                        };
                        let io = TokioIo::new(tls_stream);
                        let hyper_svc = hyper_util::service::TowerToHyperService::new(svc);
                        if let Err(err) = AutoBuilder::new(TokioExecutor::new())
                            .serve_connection(io, hyper_svc)
                            .await
                        {
                            debug!("event=https_listener_connection_closed err={}", err);
                        }
                    });
                }
            });
        });

        let client = Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|e| CommunicationError::Other(format!("build http client: {}", e)))?;

        self.inbound_rx = Some(rx);
        self.http_client = Some(client);
        info!(
            "event=https_communication_started node_id={} address={} peer_count={}",
            self.local_id,
            address,
            self.peer_urls.len()
        );
        Ok(())
    }

    fn poll(&mut self) -> Result<Option<InboundMessage>, CommunicationError> {
        let rx = self
            .inbound_rx
            .as_ref()
            .ok_or(CommunicationError::NotStarted)?;

        match rx.try_recv() {
            Ok(msg) => {
                debug!(
                    "event=https_poll_inbound node_id={} peer_id={}",
                    self.local_id, msg.from
                );
                Ok(Some(wire::wire_to_message(msg)))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(CommunicationError::Disconnected),
        }
    }

    fn send(&mut self, to: NodeId, message: RaftMessage) -> SendOutcome {
        let client = match &self.http_client {
            Some(c) => c,
            None => {
                warn!(
                    "event=https_send_drop_not_started node_id={} peer_id={}",
                    self.local_id, to
                );
                return SendOutcome::Dropped("not started".to_string());
            }
        };

        let url = match self.peer_urls.get(&to) {
            Some(u) => u.clone(),
            None => {
                warn!(
                    "event=https_send_drop_unconfigured_peer node_id={} peer_id={}",
                    self.local_id, to
                );
                return SendOutcome::Dropped(format!("peer {} not configured", to));
            }
        };

        let wire_msg = wire::message_to_wire(self.local_id, to, message);
        match client.post(&url).json(&wire_msg).send() {
            Ok(resp) if resp.status().is_success() => {
                debug!(
                    "event=https_send_ok node_id={} peer_id={} url={}",
                    self.local_id, to, url
                );
                SendOutcome::Sent
            }
            Ok(resp) => {
                warn!(
                    "event=https_send_drop_http_status node_id={} peer_id={} url={} status={}",
                    self.local_id,
                    to,
                    url,
                    resp.status()
                );
                SendOutcome::Dropped(format!("peer {} returned HTTP {}", to, resp.status()))
            }
            Err(e) => {
                error!(
                    "event=https_send_error node_id={} peer_id={} url={} err={}",
                    self.local_id, to, url, e
                );
                SendOutcome::Dropped(format!("send to peer {}: {}", to, e))
            }
        }
    }
}
