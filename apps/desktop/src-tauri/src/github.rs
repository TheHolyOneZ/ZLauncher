use serde::{Deserialize, Serialize};

pub const CLIENT_ID: Option<&str> = option_env!("ZL_GITHUB_CLIENT_ID");
pub const TOKEN_KEY: &str = "github_token";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub login: Option<String>,
    pub device_flow: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_in: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Poll {
    Pending { interval: u64 },
    Done { token: String },
    Expired,
    Denied,
}

#[derive(Deserialize)]
struct User {
    login: String,
}

pub async fn whoami(client: &reqwest::Client, token: &str) -> Result<String, String> {
    let resp = client
        .get("https://api.github.com/user")
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    match resp.status().as_u16() {
        200 => resp.json::<User>().await.map(|u| u.login).map_err(|e| e.to_string()),
        401 => Err("GitHub doesn't accept this token. It may be mistyped, expired or revoked.".into()),
        s => Err(format!("GitHub answered {s}.")),
    }
}

pub async fn device_start(client: &reqwest::Client) -> Result<DeviceCode, String> {
    let id = CLIENT_ID.ok_or("This build has no GitHub sign-in. Paste a token instead.")?;
    let resp = client
        .post("https://github.com/login/device/code")
        .header("Accept", "application/json")
        .json(&serde_json::json!({ "client_id": id, "scope": "" }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GitHub answered {}.", resp.status().as_u16()));
    }
    #[derive(Deserialize)]
    struct Raw {
        device_code: String,
        user_code: String,
        verification_uri: String,
        interval: u64,
        expires_in: u64,
    }
    let r: Raw = resp.json().await.map_err(|e| e.to_string())?;
    Ok(DeviceCode {
        device_code: r.device_code,
        user_code: r.user_code,
        verification_uri: r.verification_uri,
        interval: r.interval.max(5),
        expires_in: r.expires_in,
    })
}

pub async fn device_poll(client: &reqwest::Client, device_code: &str, interval: u64) -> Result<Poll, String> {
    let id = CLIENT_ID.ok_or("This build has no GitHub sign-in.")?;
    #[derive(Deserialize)]
    struct Raw {
        access_token: Option<String>,
        error: Option<String>,
        interval: Option<u64>,
    }
    let r: Raw = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "client_id": id,
            "device_code": device_code,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(token) = r.access_token {
        return Ok(Poll::Done { token });
    }
    Ok(match r.error.as_deref() {
        Some("authorization_pending") => Poll::Pending { interval },
        Some("slow_down") => Poll::Pending { interval: r.interval.unwrap_or(interval + 5) },
        Some("expired_token") => Poll::Expired,
        Some("access_denied") => Poll::Denied,
        Some(e) => return Err(format!("GitHub sign-in failed: {e}")),
        None => return Err("GitHub sent an empty answer.".into()),
    })
}
