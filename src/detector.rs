use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct TestResult {
    pub node_name: String, pub ip_type: String, pub ip_risk: String, pub ip_score: String, pub ip_stars: String,
    pub netflix_unlock: String, pub chatgpt_unlock: String, pub claude_unlock: String, pub gemini_unlock: String,
    pub tcp_ping: Option<u64>, pub icmp_ping: String, pub http_delay: Option<u64>, pub speed_mbps: f64,
}

// 模拟正常的浏览器请求头，防止被流媒体平台轻易判定为爬虫
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub async fn check_ip_quality(client: &Client) -> (String, String, String, String) {
    let api_url = "https://api.ipapi.is";
    let Ok(res) = client.get(api_url).send().await else { return ("未知".into(), "网络错误".into(), "N/A".into(), "🚫".into()) };
    let Ok(data) = res.json::<Value>().await else { return ("未知".into(), "解析失败".into(), "N/A".into(), "🚫".into()) };
    let is_vpn = data.get("is_vpn").and_then(|v| v.as_bool()).unwrap_or(false);
    let is_proxy = data.get("is_proxy").and_then(|v| v.as_bool()).unwrap_or(false);
    let is_datacenter = data.get("is_datacenter").and_then(|v| v.as_bool()).unwrap_or(false);
    let type_cn = match data.pointer("/company/type").and_then(|v| v.as_str()).unwrap_or("unknown") { "isp" => "家宽", "hosting" => "机房", "business" => "商宽", "education" => "教育网", _ => "未知" };
    let risk_label = if is_vpn || is_proxy { "🌐 代理/VPN" } else if is_datacenter { "🏢 机房" } else { "✨ 原生" };
    let abuser_score = data.pointer("/company/abuser_score").and_then(|v| v.as_str()).unwrap_or("High");
    let stars = if abuser_score.contains("Low") { "⭐⭐⭐⭐⭐" } else if abuser_score.contains("Elevated") { "⭐⭐⭐" } else { "⭐" };
    (type_cn.into(), risk_label.into(), abuser_score.into(), stars.into())
}

pub async fn check_netflix(client: &Client) -> String {
    // 70143836 是《绝命毒师》，通常作为非自制剧的解锁探测标准
    match client.get("https://www.netflix.com/title/70143836").header("User-Agent", USER_AGENT).send().await {
        Ok(res) if res.status().as_u16() == 403 => "Blocked (403)".into(),
        Ok(res) => { 
            let text = res.text().await.unwrap_or_default();
            
            // 🌟 核心逻辑：从 Netflix 网页源码中暴力提取当前识别的区域代码 (如 "currentCountry":"US")
            let mut region = "Unknown".to_string();
            if let Some(idx) = text.find("\"currentCountry\":\"") {
                let start = idx + 18;
                if start + 2 <= text.len() {
                    region = text[start..start+2].to_uppercase();
                }
            } else if let Some(idx) = text.find("\"country\":\"") {
                let start = idx + 11;
                if start + 2 <= text.len() {
                    region = text[start..start+2].to_uppercase();
                }
            }

            if text.contains("Not Available") { 
                format!("Originals Only ({})", region) 
            } else { 
                // 确保包含 Unlocked 关键字，前端会把它渲染为绿色
                format!("Unlocked ({})", region) 
            } 
        },
        _ => "Timeout".into(),
    }
}

pub async fn check_ai(client: &Client, url: &str, name: &str) -> String {
    match client.get(url).header("User-Agent", USER_AGENT).send().await {
        Ok(res) if res.status().is_success() => {
            let text = res.text().await.unwrap_or_default();
            
            // 🌟 核心逻辑：针对 ChatGPT 的 Cloudflare trace 探针，提取 loc 字段
            if name == "_ChatGPT" && text.contains("loc=") {
                for line in text.lines() {
                    if line.starts_with("loc=") {
                        let region = line.trim_start_matches("loc=").to_uppercase();
                        return format!("Unlocked ({})", region);
                    }
                }
            }
            
            "Unlocked".into()
        },
        Ok(res) => {
            if res.status().as_u16() == 403 {
                "Blocked (403)".into()
            } else {
                "Blocked / Restricted".into()
            }
        },
        _ => "Timeout".into(),
    }
}
