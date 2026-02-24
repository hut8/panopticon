use anyhow::{Context, Result};
use lettre::{
    message::{header::ContentType, Mailbox},
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use tracing::{error, info};

#[derive(Clone)]
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    base_url: String,
}

impl Mailer {
    pub fn new() -> Result<Self> {
        let smtp_host = std::env::var("SMTP_HOST").context("SMTP_HOST must be set")?;
        let smtp_username = std::env::var("SMTP_USERNAME").context("SMTP_USERNAME must be set")?;
        let smtp_password = std::env::var("SMTP_PASSWORD").context("SMTP_PASSWORD must be set")?;
        let smtp_from =
            std::env::var("SMTP_FROM").unwrap_or_else(|_| "panopticon@hut8.tools".into());
        let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:5173".into());

        let creds = Credentials::new(smtp_username, smtp_password);

        let transport = AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp_host)
            .context("Failed to create SMTP transport")?
            .credentials(creds)
            .build();

        let from: Mailbox = format!("Panopticon <{smtp_from}>")
            .parse()
            .context("Invalid SMTP_FROM address")?;

        info!("Mailer initialized (SMTP: {smtp_host})");

        Ok(Self {
            transport,
            from,
            base_url,
        })
    }

    pub async fn send_confirmation_email(&self, to_email: &str, token: &str) -> Result<()> {
        let confirm_url = format!("{}/api/auth/confirm-email?token={}", self.base_url, token);
        let subject = "Confirm your Panopticon account";
        let html = confirmation_template(&confirm_url);

        self.send(to_email, subject, &html).await
    }

    pub async fn send_password_reset_email(&self, to_email: &str, token: &str) -> Result<()> {
        let reset_url = format!("{}/reset-password?token={}", self.base_url, token);
        let subject = "Reset your Panopticon password";
        let html = password_reset_template(&reset_url);

        self.send(to_email, subject, &html).await
    }

    async fn send(&self, to_email: &str, subject: &str, html_body: &str) -> Result<()> {
        let to: Mailbox = to_email
            .parse()
            .with_context(|| format!("Invalid recipient address: {to_email}"))?;

        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(subject)
            .header(ContentType::TEXT_HTML)
            .body(html_body.to_string())
            .context("Failed to build email message")?;

        match self.transport.send(message).await {
            Ok(_) => {
                info!(to = to_email, subject, "Email sent");
                Ok(())
            }
            Err(e) => {
                error!(to = to_email, subject, error = %e, "Failed to send email");
                Err(e).context("Failed to send email")
            }
        }
    }
}

fn confirmation_template(confirm_url: &str) -> String {
    email_template(
        "Confirm your email",
        "Thanks for signing up for Panopticon. Click the button below to confirm your email address.",
        "Confirm Email",
        confirm_url,
        "This link expires in 24 hours. If you didn't create this account, you can ignore this email.",
    )
}

fn password_reset_template(reset_url: &str) -> String {
    email_template(
        "Reset your password",
        "We received a request to reset your Panopticon password. Click the button below to choose a new password.",
        "Reset Password",
        reset_url,
        "This link expires in 1 hour. If you didn't request this, you can ignore this email.",
    )
}

fn email_template(
    heading: &str,
    body: &str,
    button_text: &str,
    button_url: &str,
    footer: &str,
) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1.0"></head>
<body style="margin:0;padding:0;background-color:#1a1a2e;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background-color:#1a1a2e;">
<tr><td align="center" style="padding:40px 20px;">
<table role="presentation" width="480" cellpadding="0" cellspacing="0" style="background-color:#16213e;border-radius:12px;overflow:hidden;">
<tr><td style="padding:40px 32px;">
  <h1 style="margin:0 0 16px;color:#e2e8f0;font-size:24px;font-weight:600;">{heading}</h1>
  <p style="margin:0 0 32px;color:#94a3b8;font-size:16px;line-height:1.6;">{body}</p>
  <table role="presentation" cellpadding="0" cellspacing="0">
  <tr><td style="background-color:#6366f1;border-radius:8px;">
    <a href="{button_url}" style="display:inline-block;padding:14px 32px;color:#ffffff;font-size:16px;font-weight:600;text-decoration:none;">{button_text}</a>
  </td></tr>
  </table>
  <p style="margin:32px 0 0;color:#64748b;font-size:13px;line-height:1.5;">{footer}</p>
</td></tr>
</table>
</td></tr>
</table>
</body>
</html>"#
    )
}
