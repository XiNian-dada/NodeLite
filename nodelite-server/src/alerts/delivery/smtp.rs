use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use nodelite_proto::{AlertSmtpConfig, AlertSmtpTransport};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::{ClientConfig, RootCertStore, pki_types::ServerName};

use crate::alerts::AlertEvent;

use super::{AlertDeliveryError, InspectionSummary};
use template::{build_alert_message, build_inspection_message, validate_header_value};

mod template;

const SMTP_TIMEOUT: Duration = Duration::from_secs(15);
const SMTP_MAX_RESPONSE_BYTES: usize = 16 * 1024;
const SMTP_HELO_NAME: &str = "nodelite.local";

pub(super) async fn send_alert_event(
    config: &AlertSmtpConfig,
    event: &AlertEvent,
) -> Result<(), AlertDeliveryError> {
    let message = build_alert_message(config, event)?;
    send_smtp_with_timeout(config, message, SMTP_TIMEOUT).await
}

pub(super) async fn send_inspection_summary(
    config: &AlertSmtpConfig,
    summary: &InspectionSummary<'_>,
) -> Result<(), AlertDeliveryError> {
    let message = build_inspection_message(config, summary)?;
    send_smtp_with_timeout(config, message, SMTP_TIMEOUT).await
}

async fn send_smtp_with_timeout(
    config: &AlertSmtpConfig,
    message: String,
    delivery_timeout: Duration,
) -> Result<(), AlertDeliveryError> {
    timeout(delivery_timeout, send_smtp_inner(config, message))
        .await
        .map_err(|_| AlertDeliveryError::SmtpTimeout)?
}

async fn send_smtp_inner(
    config: &AlertSmtpConfig,
    message: String,
) -> Result<(), AlertDeliveryError> {
    validate_smtp_config(config)?;
    let tcp = TcpStream::connect((config.host.as_str(), config.port)).await?;
    match config.transport {
        AlertSmtpTransport::Tls => {
            let mut stream = tls_connect(tcp, &config.host).await?;
            run_smtp_dialog(&mut stream, config, &message, false).await
        }
        AlertSmtpTransport::StartTls => {
            let mut stream = tcp;
            expect_response(&mut stream, &[220]).await?;
            send_ehlo(&mut stream).await?;
            send_command(&mut stream, "STARTTLS").await?;
            expect_response(&mut stream, &[220]).await?;
            let mut stream = tls_connect(stream, &config.host).await?;
            run_smtp_dialog(&mut stream, config, &message, true).await
        }
        AlertSmtpTransport::Plain => {
            let mut stream = tcp;
            run_smtp_dialog(&mut stream, config, &message, false).await
        }
    }
}

async fn run_smtp_dialog<S>(
    stream: &mut S,
    config: &AlertSmtpConfig,
    message: &str,
    greeted: bool,
) -> Result<(), AlertDeliveryError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if !greeted {
        expect_response(stream, &[220]).await?;
        send_ehlo(stream).await?;
    } else {
        send_ehlo(stream).await?;
    }

    if !config.username.is_empty() {
        authenticate(stream, config).await?;
    }
    send_command(stream, &format!("MAIL FROM:<{}>", config.sender)).await?;
    expect_response(stream, &[250]).await?;
    for recipient in &config.recipients {
        send_command(stream, &format!("RCPT TO:<{recipient}>")).await?;
        expect_response(stream, &[250, 251]).await?;
    }
    send_command(stream, "DATA").await?;
    expect_response(stream, &[354]).await?;
    stream.write_all(dot_stuff(message).as_bytes()).await?;
    stream.write_all(b"\r\n.\r\n").await?;
    stream.flush().await?;
    expect_response(stream, &[250]).await?;
    send_command(stream, "QUIT").await?;
    let _ = read_response(stream).await;
    Ok(())
}

async fn authenticate<S>(stream: &mut S, config: &AlertSmtpConfig) -> Result<(), AlertDeliveryError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let password = config.password.as_deref().unwrap_or_default();
    let mut raw_auth = auth_plain_bytes(&config.username, password);
    let payload = encode_auth_plain_payload(&mut raw_auth);
    let send_result = send_auth_plain_command(stream, payload.as_slice()).await;
    drop(payload);
    send_result?;
    expect_response(stream, &[235]).await
}

fn auth_plain_bytes(username: &str, password: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(username.len() + password.len() + 2);
    bytes.push(0);
    bytes.extend_from_slice(username.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(password.as_bytes());
    bytes
}

fn encode_auth_plain_payload(raw_auth: &mut [u8]) -> AuthPlainPayload {
    let payload = STANDARD.encode(&raw_auth[..]).into_bytes();
    raw_auth.fill(0);
    AuthPlainPayload::new(payload)
}

struct AuthPlainPayload {
    bytes: Vec<u8>,
    #[cfg(test)]
    drop_marker: Option<Arc<AtomicBool>>,
}

impl AuthPlainPayload {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            #[cfg(test)]
            drop_marker: None,
        }
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    #[cfg(test)]
    fn with_drop_marker(mut self, marker: Arc<AtomicBool>) -> Self {
        self.drop_marker = Some(marker);
        self
    }
}

impl Drop for AuthPlainPayload {
    fn drop(&mut self) {
        self.bytes.fill(0);
        #[cfg(test)]
        if let Some(marker) = &self.drop_marker {
            marker.store(true, Ordering::SeqCst);
        }
    }
}

async fn send_auth_plain_command<S>(
    stream: &mut S,
    payload: &[u8],
) -> Result<(), AlertDeliveryError>
where
    S: AsyncWrite + Unpin,
{
    stream.write_all(b"AUTH PLAIN ").await?;
    stream.write_all(payload).await?;
    stream.write_all(b"\r\n").await?;
    stream.flush().await?;
    Ok(())
}

async fn send_ehlo<S>(stream: &mut S) -> Result<(), AlertDeliveryError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    send_command(stream, &format!("EHLO {SMTP_HELO_NAME}")).await?;
    expect_response(stream, &[250]).await
}

async fn send_command<S>(stream: &mut S, command: &str) -> Result<(), AlertDeliveryError>
where
    S: AsyncWrite + Unpin,
{
    stream.write_all(command.as_bytes()).await?;
    stream.write_all(b"\r\n").await?;
    stream.flush().await?;
    Ok(())
}

async fn expect_response<S>(stream: &mut S, expected: &[u16]) -> Result<(), AlertDeliveryError>
where
    S: AsyncRead + Unpin,
{
    let response = read_response(stream).await?;
    if expected.contains(&response.code) {
        return Ok(());
    }
    Err(AlertDeliveryError::Smtp(response.message))
}

async fn read_response<S>(stream: &mut S) -> Result<SmtpResponse, AlertDeliveryError>
where
    S: AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    let mut line = Vec::new();
    let mut one = [0_u8; 1];
    loop {
        let read = stream.read(&mut one).await?;
        if read == 0 {
            return Err(AlertDeliveryError::Smtp(
                "connection closed before SMTP response completed".to_string(),
            ));
        }
        bytes.push(one[0]);
        line.push(one[0]);
        if bytes.len() > SMTP_MAX_RESPONSE_BYTES {
            return Err(AlertDeliveryError::Smtp(
                "SMTP response exceeded maximum size".to_string(),
            ));
        }
        if line.ends_with(b"\r\n") {
            if is_final_smtp_line(&line) {
                let message = String::from_utf8_lossy(&bytes).trim().to_string();
                let code = parse_smtp_code(&line)?;
                return Ok(SmtpResponse { code, message });
            }
            line.clear();
        }
    }
}

fn is_final_smtp_line(line: &[u8]) -> bool {
    line.len() >= 5 && line[0..3].iter().all(u8::is_ascii_digit) && line[3] == b' '
}

fn parse_smtp_code(line: &[u8]) -> Result<u16, AlertDeliveryError> {
    let code = std::str::from_utf8(&line[0..3])
        .map_err(|_| AlertDeliveryError::Smtp("SMTP response code was invalid".to_string()))?
        .parse::<u16>()
        .map_err(|_| AlertDeliveryError::Smtp("SMTP response code was invalid".to_string()))?;
    Ok(code)
}

#[derive(Debug)]
struct SmtpResponse {
    code: u16,
    message: String,
}

async fn tls_connect(
    stream: TcpStream,
    host: &str,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, AlertDeliveryError> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|error| AlertDeliveryError::Tls(error.to_string()))?;
    connector
        .connect(server_name, stream)
        .await
        .map_err(|error| AlertDeliveryError::Tls(error.to_string()))
}

fn validate_smtp_config(config: &AlertSmtpConfig) -> Result<(), AlertDeliveryError> {
    validate_header_value(&config.sender)?;
    validate_header_value(&config.host)?;
    validate_header_value(&config.username)?;
    if let Some(password) = config.password.as_deref() {
        validate_header_value(password)?;
    }
    for recipient in &config.recipients {
        validate_header_value(recipient)?;
    }
    Ok(())
}

fn dot_stuff(message: &str) -> String {
    message
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(|line| {
            if line.starts_with('.') {
                format!(".{line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

#[cfg(test)]
mod tests;
