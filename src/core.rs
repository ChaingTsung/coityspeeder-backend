use crate::detector::*;
use anyhow::{Context, Result};
use reqwest::{Client, Proxy};
use serde_json::json;
use serde_yaml::{Mapping, Value as YamlValue};
use std::fs::File;
use std::io::Write;
use std::process::{Child, Command};
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
        
        if !std::path::Path::new(bin_name).exists() {
            error!("❌ 严重错误: 找不到 Mihomo 内核文件 {}", bin_name);
        }

        info!("🚀 执行命令: {} -f {}", bin_name, config_path);
        let child = Command::new(bin_name)
            .arg("-f")
            .arg(&config_path)
            // 🌟 致命修正：去掉了 stdout 和 stderr 的 null 拦截！
            // 这样内核一旦报错退出，错误原因会直接打在 systemd 日志里！
            .spawn()
            .map_err(|e| {
                error!("❌ Mihomo 进程启动失败: {}", e);
                e
            })?;
        Ok(Self { child, config_path })
    }
}
impl Drop for MihomoProcess { 
    fn drop(&mut self) { 
        let _ = self.child.kill(); 
        let _ = self.child.wait(); 
        let _ = std::fs::remove_file(&self.config_path); 
    } 
}

pub struct XrayProcess { child: Child, config_path: String }
impl XrayProcess {
    pub fn start(json_content: &str, port: u16) -> Result<Self> {
        let config_path = format!("temp_xray_{}.json", port);
        File::create(&config_path)?.write_all(json_content.as_bytes())?;
        let bin_name = if cfg!(target_os = "windows") { "../xray.exe" } else { "../xray" };
        
        if !std::path::Path::new(bin_name).exists() {
            error!("❌ 严重错误: 找不到 Xray 内核文件 {}", bin_name);
        }

        info!("🚀 执行命令: {} run -c {}", bin_name, config_path);
        let child = Command::new(bin_name)
            .arg("run") // 🌟 兼容最新版 Xray 必须加 run 命令
            .arg("-c")
            .arg(&config_path)
            // 🌟 致命修正：放开错误输出
            .spawn()
            .map_err(|e| {
                error!("❌ Xray 进程启动失败: {}", e);
                e
            })?;
        Ok(Self { child, config_path })
    }
}
impl Drop for XrayProcess { 
    fn drop(&mut self) { 
        let _ = self.child.kill(); 
        let _ = self.child.wait(); 
        let _ = std::fs::remove_file(&self.config_path); 
    } 
}

enum ProxyProcess { Mihomo(MihomoProcess), Xray(XrayProcess), None }

fn generate_xray_json_from_vless(vless_link: &str, port: u16) -> Result<(String, String)> {
    let parsed = Url::parse(vless_link).context("解析 vless 链接失败")?;
    let uuid = parsed.username(); 
    let server = parsed.host_str().unwrap_or(""); 
    let server_port = parsed.port().unwrap_or(443);
    
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
    
    Ok((
        serde_json::to_string(&config)?, 
        url::form_urlencoded::parse(node_name.as_bytes()).map(|(k, v)| k.to_string()).next().unwrap_or(node_name.to_string())
    ))
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
                match MihomoProcess::start(&serde_yaml::to_string(&root).unwrap(), port) { 
                    Ok(process) => {
                        _process_guard = ProxyProcess::Mihomo(process); 
                        info!("✅ Mihomo 内核进程拉起成功！");
                    },
                    Err(_) => error!("❌ Mihomo 拉起失败！"),
                }
            }
        }
    } else if target.starts_with("vless://") {
        info!("🔍 解析 VLESS 并启动 Xray-core...");
        match generate_xray_json_from_vless(target, port) {
            Ok((json_config, name)) => {
                node_name = name;
                match XrayProcess::start(&json_config, port) { 
                    Ok(process) => {
                        _process_guard = ProxyProcess::Xray(process); 
                        info!("✅ Xray-core 进程拉起成功！");
                    },
                    Err(_) => error!("❌ Xray-core 拉起失败！"),
                }
            },
            Err(e) => error!("❌ VLESS 链接解析失败: {}", e),
        }
    } else {
        info!("🔍 请求 Subconvert 并启动 Mihomo...");
        if let Some(node) = fetch_proxies(target, sub_url).await.unwrap_or_default().first() {
            node_name = node.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown").to_string();
            let mut root = Mapping::new();
            root.insert(YamlValue::String("mixed-port".into()), YamlValue::Number(port.into()));
            root.insert(YamlValue::String("proxies".into()), YamlValue::Sequence(vec![node.clone()]));
            root.insert(YamlValue::String("rules".into()), YamlValue::Sequence(vec![YamlValue::String(format!("MATCH,{}", node_name))]));
            match MihomoProcess::start(&serde_yaml::to_string(&root).unwrap(), port) { 
                Ok(process) => {
                    _process_guard = ProxyProcess::Mihomo(process); 
                    info!("✅ Mihomo 内核进程拉起成功！");
                },
                Err(_) => error!("❌ Mihomo 拉起失败！"),
            }
        }
    }

    tokio::time::sleep(Duration::from_secs(2)).await;
    
    let proxy_url = format!("socks5h://127.0.0.1:{}", port);
    
    let client_result = Client::builder()
        .proxy(Proxy::all(&proxy_url).unwrap())
        .timeout(Duration::from_secs(30))
        .build();

    let client = match client_result {
        Ok(c) => c,
        Err(e) => {
            error!("构建 HTTP 客户端失败: {}", e);
            return TestResult { 
                node_name, 
                ip_type: "未知".into(), ip_risk: "内部错误".into(), ip_score: "N/A".into(), ip_stars: "🚫".into(), 
                netflix_unlock: "Timeout".into(), chatgpt_unlock: "Timeout".into(), claude_unlock: "Timeout".into(), gemini_unlock: "Timeout".into(), 
                http_delay: None, speed_mbps: 0.0, tcp_ping: None, icmp_ping: "N/A".into() 
            };
        }
    };

    let start = Instant::now();
    let http_delay = client.get("http://www.google.com/generate_204").send().await.ok().map(|_| start.elapsed().as_millis() as u64);
    
    let (ip_info, netflix, chatgpt, claude, gemini) = tokio::join!(
        check_ip_quality(&client), 
        check_netflix(&client), 
        check_ai(&client, "https://chatgpt.com/cdn-cgi/trace", "_ChatGPT"), 
        check_ai(&client, "https://claude.ai/login", "_Claude"), 
        check_ai(&client, "https://gemini.google.com/app", "_Gemini")
    );

    TestResult { 
        node_name, 
        ip_type: ip_info.0, ip_risk: ip_info.1, ip_score: ip_info.2, ip_stars: ip_info.3, 
        netflix_unlock: netflix, chatgpt_unlock: chatgpt, claude_unlock: claude, gemini_unlock: gemini, 
        http_delay, speed_mbps: 0.0, tcp_ping: None, icmp_ping: "N/A".into() 
    }
}
