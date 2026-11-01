//! WeCom replies report failures inside HTTP 200 bodies (#373).

use super::check_response;
use crate::alerts::delivery::AlertDeliveryError;

async fn check(raw: &str, wecom: bool) -> Result<(), AlertDeliveryError> {
    let mut stream = raw.as_bytes();
    check_response(&mut stream, wecom).await
}

fn ok_with_body(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn wecom_success_requires_errcode_zero() {
    check(&ok_with_body(r#"{"errcode":0,"errmsg":"ok"}"#), true)
        .await
        .expect("errcode 0 is delivered");

    for (errcode, retryable) in [(93000, false), (45009, true), (-1, true)] {
        let body = format!(r#"{{"errcode":{errcode},"errmsg":"rejected"}}"#);
        let error = check(&ok_with_body(&body), true)
            .await
            .expect_err("a non-zero errcode is a failure");
        assert!(
            matches!(error, AlertDeliveryError::WeComRejected { errcode: code } if code == errcode),
            "{error:?}"
        );
        assert_eq!(error.is_retryable(), retryable, "errcode {errcode}");
    }
}

#[tokio::test]
async fn wecom_reply_without_errcode_is_not_treated_as_delivered() {
    for body in ["", "<html>ok</html>", r#"{"errmsg":"ok"}"#] {
        let error = check(&ok_with_body(body), true)
            .await
            .expect_err("an unreadable reply cannot confirm delivery");
        assert!(
            matches!(error, AlertDeliveryError::InvalidResponse),
            "{body:?}"
        );
    }
}

#[tokio::test]
async fn wecom_chunked_reply_is_decoded() {
    let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
               a\r\n{\"errcode\"\r\n\
               9\r\n:45009, \"\r\n\
               c\r\nerrmsg\":\"x\"}\r\n\
               0\r\n\r\n";
    let error = check(raw, true).await.expect_err("errcode in chunks");
    assert!(matches!(
        error,
        AlertDeliveryError::WeComRejected { errcode: 45009 }
    ));
}

#[tokio::test]
async fn other_webhooks_keep_plain_2xx_semantics() {
    check(&ok_with_body("<html>anything</html>"), false)
        .await
        .expect("2xx is enough for generic webhooks");
    let error = check("HTTP/1.1 503 Busy\r\n\r\n", false)
        .await
        .expect_err("non-2xx fails");
    assert!(matches!(
        error,
        AlertDeliveryError::HttpStatus { status: 503 }
    ));
    let error = check(
        &ok_with_body(r#"{"errcode":0}"#).replace("200 OK", "500 Error"),
        true,
    )
    .await
    .expect_err("WeCom still needs a 2xx status");
    assert!(matches!(
        error,
        AlertDeliveryError::HttpStatus { status: 500 }
    ));
}

#[tokio::test]
async fn oversized_wecom_body_is_rejected() {
    let body = format!(r#"{{"errcode":0,"pad":"{}"}}"#, "x".repeat(64 * 1024));
    let error = check(&ok_with_body(&body), true)
        .await
        .expect_err("bounded body read");
    assert!(matches!(error, AlertDeliveryError::ResponseTooLarge));
}
