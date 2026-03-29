use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct TestResult {
    pub node_name: String, pub ip_type: String, pub ip_risk: String, pub ip_score: String, pub ip_stars: String,
    pub netflix_unlock: String, pub chatgpt_unlock: String, pub claude_unlock: String, pub gemini_unlock: String,
    pub tcp_ping: Option<u64>, pub icmp_ping: String, pub http_delay: Option<u64>, pub speed_mbps: f64,
}

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0 Safari/537.36";

pub async fn check_ip_quality(client: &Client) -> (String, String, String, String) {
    let api_url = "https://api.ipapi.is";
    let Ok(res) = client.get(api_url).send().await else { return ("未知".into(), "网络错误".into(), "N/A".into(), "🚫".into()) };
    let Ok(data) = res.json::<Value>().await else { return ("未知".into(), "解析失败".into(), "N/A".into(), "🚫".into()) };
    let is_vpn = data.get("is_vpn").and_then(|v| v.as_bool()).unwrap_or(false);
    let is_proxy = data.get("is_proxy").and_then(|v| v.as_bool()).unwrap_or(false);
    let is_datacenter = data.get("is_datacenter").and_then(|v| v.as_bool()).unwrap_or(false);
    let type_cn = match data.pointer("/company/type").and_then(|v| v.as_str()).unwrap_or("unknown") { "isp" => "家宽", "hosting" => "机房", "business" => "商宽", _ => "未知" };
    let risk_label = if is_vpn || is_proxy { "🌐 代理/VPN" } else if is_datacenter { "🏢 机房" } else { "✨ 原生" };
    let abuser_score = data.pointer("/company/abuser_score").and_then(|v| v.as_str()).unwrap_or("High");
    let stars = if abuser_score.contains("Low") { "⭐⭐⭐⭐⭐" } else if abuser_score.contains("Elevated") { "⭐⭐⭐" } else { "⭐" };
    (type_cn.into(), risk_label.into(), abuser_score.into(), stars.into())
}

pub async fn check_netflix(client: &Client) -> String {
    match client.get("https://www.netflix.com/title/70143836").header("User-Agent", USER_AGENT).send().await {
        Ok(res) if res.status().as_u16() == 403 => "Blocked (403)".into(),
        Ok(res) => { if res.text().await.unwrap_or_default().contains("Not Available") { "Originals Only".into() } else { "Full Unlocked".into() } },
        _ => "Timeout".into(),
    }
}

pub async fn check_ai(client: &Client, url: &str, _name: &str) -> String {
    match client.get(url).header("User-Agent", USER_AGENT).send().await {
        Ok(res) if res.status().is_success() => "Unlocked".into(),
        Ok(_) => "Blocked / Restricted".into(),
        _ => "Timeout".into(),
    }
}