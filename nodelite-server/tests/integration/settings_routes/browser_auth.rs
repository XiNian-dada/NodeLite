//! Real upgraded connections must stop when the authorization that admitted them ends.

use super::*;
use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn serve(harness: &SettingsHarness) -> Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let app = harness.app.clone();
    let shutdown = harness.state.shutdown.clone();
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await
        .expect("serve settings routes");
    });
    Ok((addr, task))
}

async fn connect(
    addr: SocketAddr,
    password: Option<&str>,
    cookie: Option<&str>,
) -> Result<(Socket, String)> {
    let mut request = format!("ws://{addr}/ws/browser").into_client_request()?;
    if let Some(password) = password {
        request
            .headers_mut()
            .insert(header::AUTHORIZATION, basic_auth_header(password).parse()?);
    }
    if let Some(cookie) = cookie {
        request
            .headers_mut()
            .insert(header::COOKIE, cookie.parse()?);
    }
    let (mut socket, response) = connect_async(request).await?;
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .to_string();
    let message = timeout(Duration::from_secs(2), socket.next())
        .await?
        .expect("initial state")?;
    assert!(matches!(message, Message::Text(text) if text.contains("initial_state")));
    Ok((socket, cookie))
}

async fn assert_revoked(socket: &mut Socket) -> Result<()> {
    let next = timeout(Duration::from_secs(3), socket.next()).await?;
    assert!(
        matches!(next, None | Some(Err(_)) | Some(Ok(Message::Close(_)))),
        "revoked WS still received data: {next:?}"
    );
    Ok(())
}

async fn assert_live(socket: &mut Socket) -> Result<()> {
    socket
        .send(Message::Text(
            serde_json::to_string(&nodelite_proto::BrowserMessage::Ping)?.into(),
        ))
        .await?;
    let message = timeout(Duration::from_secs(2), socket.next())
        .await?
        .expect("pong")?;
    assert!(matches!(message, Message::Text(text) if text.contains("pong")));
    Ok(())
}

async fn finish(harness: SettingsHarness, server: tokio::task::JoinHandle<()>) -> Result<()> {
    harness.state.shutdown.cancel();
    timeout(Duration::from_secs(3), server).await??;
    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn browser_ws_password_change_closes_existing_connection_and_accepts_new_credentials()
-> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    let (addr, server) = serve(&harness).await?;
    let (mut old, _) = connect(addr, Some("secret"), None).await?;
    let response = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/settings/password",
            &basic_auth_header("secret"),
            None,
            json!({"current_password":"secret", "new_password":"VeryStrong123!"}),
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    harness
        .state
        .shared
        .register_node(
            synthetic_identity("after-password", "New node", "1", None, "edge"),
            None,
            None,
            None,
        )
        .await;
    assert_revoked(&mut old).await?;
    assert!(connect(addr, Some("secret"), None).await.is_err());
    let (mut new, _) = connect(addr, Some("VeryStrong123!"), None).await?;
    assert_live(&mut new).await?;
    new.close(None).await?;
    finish(harness, server).await
}

#[tokio::test]
async fn browser_ws_basic_logout_revokes_only_its_session() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    let (addr, server) = serve(&harness).await?;
    let (mut first, cookie) = connect(addr, Some("secret"), None).await?;
    assert!(!cookie.is_empty());
    let (mut second, _) = connect(addr, Some("secret"), None).await?;
    let request = Request::builder()
        .uri("/logout-and-reauth")
        .header(header::COOKIE, cookie)
        .body(Body::empty())?;
    assert_eq!(
        harness.app.clone().oneshot(request).await?.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_revoked(&mut first).await?;
    assert_live(&mut second).await?;
    second.close(None).await?;
    finish(harness, server).await
}

#[tokio::test]
async fn browser_ws_two_factor_revocation_logout_and_expiry_close_connections() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(true, Some(TEST_TOTP_SECRET))).await?;
    let (addr, server) = serve(&harness).await?;
    let other_token = harness.state.two_factor_sessions.create_authenticated()?;
    let (mut other, _) = connect(addr, None, Some(&auth_cookie(&other_token))).await?;
    for action in ["revoke", "logout", "expire"] {
        let token = harness.state.two_factor_sessions.create_authenticated()?;
        if action == "expire" {
            harness
                .state
                .two_factor_sessions
                .expire_authenticated_after(&token, Duration::from_secs(1));
        }
        let (mut socket, _) = connect(addr, None, Some(&auth_cookie(&token))).await?;
        match action {
            "revoke" => harness
                .state
                .two_factor_sessions
                .remove_authenticated(&token),
            "logout" => {
                let request = Request::builder()
                    .uri("/logout-and-reauth")
                    .header(header::COOKIE, auth_cookie(&token))
                    .body(Body::empty())?;
                assert_eq!(
                    harness.app.clone().oneshot(request).await?.status(),
                    StatusCode::UNAUTHORIZED
                );
            }
            _ => {}
        }
        assert_revoked(&mut socket).await?;
        assert!(
            connect(addr, None, Some(&auth_cookie(&token)))
                .await
                .is_err()
        );
        assert_live(&mut other).await?;
    }
    other.close(None).await?;
    finish(harness, server).await
}
