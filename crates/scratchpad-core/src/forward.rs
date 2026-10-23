//! Forwarding captured emails through a real SMTP server.

use lettre::address::Envelope;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::settings::{ForwardEncryption, ForwardSettings};

pub fn transport(settings: &ForwardSettings) -> anyhow::Result<AsyncSmtpTransport<Tokio1Executor>> {
    if settings.host.trim().is_empty() {
        anyhow::bail!("No forwarding SMTP server configured. Set one up in Settings → Forwarding.");
    }
    let builder = match settings.encryption {
        ForwardEncryption::None => {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(settings.host.trim())
        }
        ForwardEncryption::StartTls => {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(settings.host.trim())?
        }
        ForwardEncryption::Tls => {
            AsyncSmtpTransport::<Tokio1Executor>::relay(settings.host.trim())?
        }
    };
    let mut builder = builder.port(settings.port);
    if !settings.username.is_empty() {
        builder = builder.credentials(Credentials::new(
            settings.username.clone(),
            settings.password.clone(),
        ));
    }
    Ok(builder.build())
}

pub async fn forward(
    settings: &ForwardSettings,
    raw: &[u8],
    original_from: &str,
    recipients: &[String],
) -> anyhow::Result<()> {
    if recipients.is_empty() {
        anyhow::bail!("No recipients given");
    }
    let from = if settings.from_override.trim().is_empty() {
        original_from
    } else {
        settings.from_override.trim()
    };
    let from = if from.is_empty() {
        None
    } else {
        Some(
            from.parse()
                .map_err(|e| anyhow::anyhow!("Invalid sender address \"{from}\": {e}"))?,
        )
    };
    let to = recipients
        .iter()
        .map(|r| {
            r.trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid recipient address \"{}\": {e}", r.trim()))
        })
        .collect::<anyhow::Result<Vec<lettre::Address>>>()?;
    let envelope = Envelope::new(from, to)?;
    transport(settings)?.send_raw(&envelope, raw).await?;
    Ok(())
}

/// Sends a sample email through Scratchpad's own SMTP server so users can verify their setup.
pub async fn send_test_email(host: &str, port: u16, mailbox: &str) -> anyhow::Result<()> {
    use lettre::message::{header::ContentType, MultiPart, SinglePart};
    // A wildcard listen address isn't connectable on every platform (e.g. Windows).
    let host = if matches!(host, "0.0.0.0" | "::" | "[::]" | "") {
        "127.0.0.1"
    } else {
        host
    };
    let html = include_str!("test_email.html");
    let message = lettre::Message::builder()
        .from("Scratchpad <hello@scratchpad.local>".parse()?)
        .to("Developer <developer@example.com>".parse()?)
        .subject("Welcome to Scratchpad 👋")
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(String::from("Hello from Scratchpad!\n\nThis is a test email sent to your local SMTP server.\nCheck out https://github.com and https://example.invalid/broken\n")),
                )
                .singlepart(SinglePart::builder().header(ContentType::TEXT_HTML).body(html.to_string())),
        )?;
    let transport = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
        .port(port)
        .credentials(Credentials::new(mailbox.to_string(), String::new()))
        .build();
    transport.send(message).await?;
    Ok(())
}
