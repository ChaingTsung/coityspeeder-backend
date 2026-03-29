use crate::detector::*;
use anyhow::{Context, Result};
use reqwest::{Client, Proxy};
use serde_json::json;
use serde_yaml::{Mapping, Value as YamlValue};
use std::fs::File;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use url::Url;
use tracing::{info, error};

const DEFAULT_SUB_URL: &str = "https://coity.app/convert";

pub struct MihomoProcess { child: Child, config_path: String }
impl MihomoProcess {
    pub fn start(yaml_content: &str, port: u16) -> Result<Self> {
        let config_path = format!("temp_mihomo_{}.yaml", port);
        File::create(&config_path)?.write_all(yaml_content.as_bytes())?;
        let bin_name = if cfg!(target_os = "windows") { "../mihomo.exe" } else { "../mihomo" };
        let child = Command::new(bin_name).arg("-f").arg(&config_path).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        Ok(Self { child, config_path })
    }
}
impl Drop for MihomoProcess { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); let _ = std::fs::remove_file(&self.config_path); } }

pub struct XrayProcess { child: Child, config_path: String }
impl XrayProcess {
    pub fn start(json_content: &str, port: u16) -> Result<Self> {
        let config_path = format!("temp_xray_{}.json", port);
        File::create(&config_path)?.write_all(json_content.as_bytes())?;
        let bin_name = if cfg!(target_os = "windows") { "../xray.exe" } else { "../xray" };
        let child = Command::new(bin_name).arg("-c").arg(&config_path).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        Ok(Self { child, config_path })
    }
}
impl Drop for XrayProcess { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); let _ = std::fs::remove_file(&self.config_path); } }

enum ProxyProcess { Mihomo(MihomoProcess), Xray(XrayProcess), None }

fn generate_xray_json_from_vless(vless_link: &str, port: u16) -> Result<(String, String)> {
    let parsed = Url::parse(vless_link).context("解析 vless 链接失败")?;
    let uuid = parsed.username(); let server = parsed.host_str().unwrap_or(""); let server_port = parsed.port().unwrap_or(443);
    let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
    let network = query.get("type").map(|s| s.as_str()).unwrap_or("tcp");
    let security = query.get("security").map(|s| s.as_str()).unwrap_or("none");
    let sni = query.get("sni").map(|s| s.as_str()).unwrap_or(server);
    let flow = query.get("flow").map(|s| s.as_str()).unwrap_or("");
    let node_name = parsed.fragment().unwrap_or("VLESS_Node");

    let mut stream_settings = json!({ "network": network, "security": security });
    
    if security == "tls" { 
        stream_settings["tlsSettings"] = json!({ "serverName": sni }); 
    } else if security == "reality" { 
        // 🌟 修复点：添加 map(|s| s.as_str()) 进行类型转换
        stream_settings["realitySettings"] = json!({ 
            "serverName": sni, 
            "publicKey": query.get("pbk").map(|s| s.as_str()).unwrap_or(""), 
            "shortId": query.get("sid").map(|s| s.as_str()).unwrap_or(""), 
            "fingerprint": query.get("fp").map(|s| s.as_str()).unwrap_or("chrome"), 
            "spiderX": query.get("spx").map(|s| s.as_str()).unwrap_or("/") 
        }); 
    }
    
    if network == "ws" { 
        stream_settings["wsSettings"] = json!({ 
            "path": query.get("path").map(|s| s.as_str()).unwrap_or("/"), 
            "headers": { "Host": query.get("host").map(|s| s.as_str()).unwrap_or(sni) } 
        }); 
    }
    
    if network == "grpc" { 
        stream_settings["grpcSettings"] = json!({ 
            "serviceName": query.get("serviceName").map(|s| s.as_str()).unwrap_or("") 
        }); 
    }
    
    let mut user_obj = json!({ "id": uuid, "encryption": "none" }); 
    if !flow.is_empty() { user_obj["flow"] = json!(flow); }

    let config = json!({
        "log": { "loglevel": "error" },
        "inbounds": [{ "port": port, "listen": "127.0.0.1", "protocol": "socks", "settings": { "udp": true } }],
        "outbounds": [{ "protocol": "vless", "settings": { "vnext": [{ "address": server, "port": server_port, "users": [user_obj] }] }, "streamSettings": stream_settings }]
    });
    Ok((serde_json::to_string(&config)?, url::form_urlencoded::parse(node_name.as_bytes()).map(|(k, v)| k.to_string()).next().unwrap_or(node_name.to_string())))
}

async fn fetch_proxies(target: &str, sub_url: Option<String>) -> Result<Vec<YamlValue>> {
    let base_url = sub_url.unwrap_or_else(|| DEFAULT_SUB_URL.to_string());
    let encoded = url::form_urlencoded::byte_serialize(target.as_bytes()).collect::<String>();
    let req_url = format!("{}?target=clash&url={}", base_url, encoded);
    let yaml_str = Client::builder().timeout(Duration::from_secs(10)).build()?.get(&req_url).send().await?.text().await?;
    let parsed: YamlValue = serde_yaml::from_str(&yaml_str)?;
    Ok(parsed.get("proxies").and_then(|p| p.as_sequence()).cloned().unwrap_or_default())
}

pub async fn execute_test(target: &str, sub_url: Option<String>, is_file: bool, port: u16) -> TestResult {
    let mut node_name = String::from("Unknown");
    let mut _process_guard = ProxyProcess::None;

    if is_file {
        info!("📄 解析本地 YAML 配置...");
        if let Ok(yaml) = serde_yaml::from_str::<YamlValue>(target) {
            if let Some(node) = yaml.get("proxies").and_then(|p| p.as_sequence()).and_then(|seq| seq.first()) {
                node_name = node.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown").to_string();
                let mut root = Mapping::new();
                root.insert(YamlValue::String("mixed-port".into()), YamlValue::Number(port.into()));
                root.insert(YamlValue::String("proxies".into()), YamlValue::Sequence(vec![node.clone()]));
                root.insert(YamlValue::String("rules".into()), YamlValue::Sequence(vec![YamlValue::String(format!("MATCH,{}", node_name))]));
                if let Ok(process) = MihomoProcess::start(&serde_yaml::to_string(&root).unwrap(), port) { _process_guard = ProxyProcess::Mihomo(process); }
            }
        }
    } else if target.starts_with("vless://") {
        info!("🔍 启动 Xray-core...");
        if let Ok((json_config, name)) = generate_xray_json_from_vless(target, port) {
            node_name = name;
            if let Ok(process) = XrayProcess::start(&json_config, port) { _process_guard = ProxyProcess::Xray(process); }
        }
    } else {
        info!("🔍 请求 Subconvert 并启动 Mihomo...");
        if let Some(node) = fetch_proxies(target, sub_url).await.unwrap_or_default().first() {
            node_name = node.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown").to_string();
            let mut root = Mapping::new();
            root.insert(YamlValue::String("mixed-port".into()), YamlValue::Number(port.into()));
            root.insert(YamlValue::String("proxies".into()), YamlValue::Sequence(vec![node.clone()]));
            root.insert(YamlValue::String("rules".into()), YamlValue::Sequence(vec![YamlValue::String(format!("MATCH,{}", node_name))]));
            if let Ok(process) = MihomoProcess::start(&serde_yaml::to_string(&root).unwrap(), port) { _process_guard = ProxyProcess::Mihomo(process); }
        }
    }

    tokio::time::sleep(Duration::from_secs(2)).await;
    let proxy_url = format!("socks5://127.0.0.1:{}", port);
    let client = Client::builder().proxy(Proxy::all(&proxy_url).unwrap()).timeout(Duration::from_millis(8400)).build().unwrap();

    let start = Instant::now();
    let http_delay = client.get("http://www.google.com/generate_204").send().await.ok().map(|_| start.elapsed().as_millis() as u64);
    let (ip_info, netflix, chatgpt, claude, gemini) = tokio::join!(check_ip_quality(&client), check_netflix(&client), check_ai(&client, "https://chatgpt.com/cdn-cgi/trace", "_ChatGPT"), check_ai(&client, "https://claude.ai/login", "_Claude"), check_ai(&client, "https://gemini.google.com/app", "_Gemini"));

    TestResult { node_name, ip_type: ip_info.0, ip_risk: ip_info.1, ip_score: ip_info.2, ip_stars: ip_info.3, netflix_unlock: netflix, chatgpt_unlock: chatgpt, claude_unlock: claude, gemini_unlock: gemini, http_delay, speed_mbps: 0.0, tcp_ping: None, icmp_ping: "N/A".into() }
}