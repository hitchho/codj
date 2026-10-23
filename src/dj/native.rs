//! codj's own agent loop, for any OpenAI-compatible chat API: OpenRouter (one key, most models),
//! OpenAI, a local server, any router with a `base_url`. The model gets codj's MCP tools (the
//! `codj mcp` child, so the `--live` guardrail is the same as for `claude`) and runs until it
//! answers without a tool call. What it does arrives as the pane's usual lines: its text,
//! `→ tool args`, `✓`/`✗` results and a closing summary.
//!
//! The loop (the shape of grok-build's turn loop, without its machinery): sample, run every tool
//! call of the reply in order and answer each by its id, sample again; stop on a plain answer, at
//! [`MAX_TURNS`], or when the model repeats the same call [`REPEATS`] times.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::{model::CallToolRequestParams, transport::TokioChildProcess, ServiceExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use super::prompt::PromptEvent;
use crate::parser::AgentSettings;

/// Model replies per request at most
pub const MAX_TURNS: usize = 20;
/// The same tool call with the same arguments this many times in a row ends the run
pub const REPEATS: usize = 3;
const HTTP_TIMEOUT: Duration = Duration::from_secs(180);

/// Where and what to ask: a resolved `[agent] provider` / `base_url` / `model`
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    /// what the pane's first line says: "openrouter anthropic/claude-sonnet-4.5"
    pub label: String,
    pub base_url: String,
    /// the environment variable holding the key ("" = no key, e.g. a local server)
    pub key_env: String,
    pub model: String,
}

/// The conversation of one DJ session with the native agent: everything but the system message
#[derive(Debug, Clone, Default)]
pub struct History(pub Arc<Mutex<Vec<Value>>>);

impl PartialEq for History {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The native endpoint `[agent]` asks for, or None when the agent is a command (`claude -p`,
/// the default, or `command = ...`)
pub fn endpoint(cfg: &AgentSettings) -> Result<Option<Endpoint>, String> {
    let preset = |name: &str, url: &str, env: &str, model: &str| Endpoint {
        label: name.to_string(),
        base_url: url.to_string(),
        key_env: env.to_string(),
        model: model.to_string(),
    };
    let mut ep = match (cfg.provider.as_deref(), cfg.base_url.as_deref()) {
        (None | Some("claude") | Some("command"), None) => return Ok(None),
        (Some("openrouter"), _) => preset("openrouter", "https://openrouter.ai/api/v1", "OPENROUTER_API_KEY", "anthropic/claude-sonnet-4.5"),
        (Some("openai"), _) => preset("openai", "https://api.openai.com/v1", "OPENAI_API_KEY", "gpt-4.1"),
        (Some(p @ ("codex" | "grok")), _) => {
            return Err(format!("[agent] provider = \"{p}\" is not supported yet: use openrouter, openai or a base_url"))
        }
        (None | Some("openai-compatible"), Some(url)) => preset("api", url, "", ""),
        (Some(other), _) => {
            return Err(format!("[agent] provider = \"{other}\": use openrouter, openai, claude, or base_url for another API"))
        }
    };
    if let Some(url) = cfg.base_url.as_deref() {
        ep.base_url = url.trim_end_matches('/').to_string();
    }
    if let Some(env) = cfg.api_key_env.as_deref() {
        ep.key_env = env.to_string();
    }
    if let Some(m) = cfg.model.as_deref().filter(|m| !m.trim().is_empty()) {
        ep.model = m.trim().to_string();
    }
    if ep.model.is_empty() {
        return Err("[agent] model is needed with base_url (the model name the API knows)".into());
    }
    ep.label = format!("{} {}", ep.label, ep.model);
    Ok(Some(ep))
}

/// An HTTP error as one readable line
fn http_problem(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.chars().take(160).collect());
    let what = match status {
        401 | 403 => "the API key was refused",
        402 => "no credits left on the account",
        404 => "unknown model or endpoint",
        429 => "rate limited",
        500..=599 => "the provider had an error",
        _ => "the request failed",
    };
    format!("✗ {what} (HTTP {status}): {}", detail.trim())
}

/// The text of a tool result (MCP content parts, text only)
fn result_text(v: &Value) -> String {
    v["content"]
        .as_array()
        .map(|parts| parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

/// Runs one request: `system` (the live preamble, rebuilt every time), the history, then `request`.
/// Sends lines and `Exited` like a child process does; the history gains this exchange.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    ep: Endpoint,
    cfg: AgentSettings,
    system: String,
    request: String,
    history: History,
    allowed: Vec<String>,
    tx: mpsc::UnboundedSender<PromptEvent>,
    mut kill: oneshot::Receiver<()>,
) {
    let say = |text: String, err: bool| {
        let _ = tx.send(PromptEvent::Line { text, err });
    };
    let started = Instant::now();
    let outcome = tokio::select! {
        r = turns(&ep, &cfg, system, request, &history, &allowed, &say) => r,
        _ = &mut kill => Err("cancelled".to_string()),
    };
    let secs = started.elapsed().as_secs_f64();
    let status = match outcome {
        Ok((n, tokens)) => {
            say(format!("-- done: {n} turns, {secs:.1} s, {tokens} tokens"), false);
            "exit status: 0".to_string()
        }
        Err(e) if e == "cancelled" => "cancelled".to_string(),
        Err(e) => {
            say(e.clone(), true);
            say(format!("-- failed: {secs:.1} s"), true);
            "failed".to_string()
        }
    };
    let _ = tx.send(PromptEvent::Exited(status));
}

async fn turns(
    ep: &Endpoint,
    cfg: &AgentSettings,
    system: String,
    request: String,
    history: &History,
    allowed: &[String],
    say: &impl Fn(String, bool),
) -> Result<(usize, u64), String> {
    let key = if ep.key_env.is_empty() {
        String::new()
    } else {
        std::env::var(&ep.key_env).map_err(|_| format!("✗ ${} is not set: put the API key there", ep.key_env))?
    };
    // codj's MCP server as a child, the same tools and guardrail `claude` gets
    let exe = std::env::current_exe().map_err(|e| format!("✗ can't find codj: {e}"))?;
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg("mcp");
    if cfg.live.unwrap_or(false) {
        cmd.arg("--live");
    }
    let transport = TokioChildProcess::new(cmd).map_err(|e| format!("✗ can't start codj mcp: {e}"))?;
    let mcp = ().serve(transport).await.map_err(|e| format!("✗ codj mcp did not start: {e}"))?;
    let tools: Vec<Value> = mcp
        .list_all_tools()
        .await
        .map_err(|e| format!("✗ codj mcp tools: {e}"))?
        .into_iter()
        .filter(|t| allowed.iter().any(|a| a.rsplit("__").next() == Some(&t.name)))
        .map(|t| {
            json!({"type": "function", "function": {
                "name": t.name, "description": t.description.unwrap_or_default(), "parameters": *t.input_schema }})
        })
        .collect();
    say(format!("-- agent ready: {}, {} tools", ep.model, tools.len()), false);

    let client = reqwest::Client::builder().timeout(HTTP_TIMEOUT).build().map_err(|e| e.to_string())?;
    let mut convo: Vec<Value> = history.0.lock().unwrap().clone();
    convo.push(json!({"role": "user", "content": request}));
    let mut tokens = 0u64;
    let mut last_call = (String::new(), 0usize);
    for turn in 1..=MAX_TURNS {
        let mut messages = vec![json!({"role": "system", "content": system})];
        messages.extend(convo.iter().cloned());
        let mut req = client
            .post(format!("{}/chat/completions", ep.base_url))
            .json(&json!({"model": ep.model, "messages": messages, "tools": tools}));
        if !key.is_empty() {
            req = req.bearer_auth(&key);
        }
        // OpenRouter ranks apps by these; harmless elsewhere
        req = req.header("X-Title", "codj").header("HTTP-Referer", "https://github.com/rotkonetworks/codj");
        let resp = req.send().await.map_err(|e| format!("✗ can't reach {}: {e}", ep.base_url))?;
        let status = resp.status().as_u16();
        let body = resp.text().await.map_err(|e| e.to_string())?;
        if status != 200 {
            return Err(http_problem(status, &body));
        }
        let v: Value = serde_json::from_str(&body).map_err(|e| format!("✗ not a chat reply: {e}"))?;
        if let Some(e) = v["error"]["message"].as_str() {
            return Err(format!("✗ {e}"));
        }
        tokens += v["usage"]["total_tokens"].as_u64().unwrap_or(0);
        let msg = v["choices"][0]["message"].clone();
        if let Some(text) = msg["content"].as_str() {
            for l in text.lines() {
                say(super::prompt::clean_line(l), false);
            }
        }
        let calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
        // the reply goes into the conversation as the API returned it (tool_calls included)
        convo.push(json!({"role": "assistant", "content": msg["content"].clone(), "tool_calls": if calls.is_empty() { Value::Null } else { msg["tool_calls"].clone() }}));
        if calls.is_empty() {
            *history.0.lock().unwrap() = convo;
            return Ok((turn, tokens));
        }
        for call in calls {
            let name = call["function"]["name"].as_str().unwrap_or("").to_string();
            let raw = call["function"]["arguments"].as_str().unwrap_or("{}");
            let args: Value = serde_json::from_str(raw).unwrap_or(json!({}));
            say(format!("→ {name} {}", super::prompt::tool_args_line(&args)).trim_end().to_string(), false);
            let fingerprint = format!("{name} {raw}");
            last_call = if last_call.0 == fingerprint { (fingerprint, last_call.1 + 1) } else { (fingerprint, 1) };
            if last_call.1 >= REPEATS {
                *history.0.lock().unwrap() = convo;
                return Err(format!("✗ the agent repeated the same call {REPEATS} times: stopped"));
            }
            let (text, err) = if allowed.iter().any(|a| a.rsplit("__").next() == Some(name.as_str())) {
                let params = CallToolRequestParams::new(name.clone()).with_arguments(args.as_object().cloned().unwrap_or_default());
                match mcp.call_tool(params).await {
                    Ok(r) => {
                        let v = serde_json::to_value(&r).unwrap_or_default();
                        (result_text(&v), v["isError"].as_bool().unwrap_or(false))
                    }
                    Err(e) => (format!("tool call failed: {e}"), true),
                }
            } else {
                (format!("{name} is not allowed here"), true)
            };
            let first = super::prompt::one_line_pub(&text, 80);
            say(format!("  {} {first}", if err { "✗" } else { "✓" }).trim_end().to_string(), err);
            convo.push(json!({"role": "tool", "tool_call_id": call["id"].clone(), "content": text}));
        }
    }
    *history.0.lock().unwrap() = convo;
    Err(format!("✗ stopped after {MAX_TURNS} turns"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(provider: Option<&str>, base: Option<&str>, model: Option<&str>) -> AgentSettings {
        AgentSettings {
            provider: provider.map(str::to_string),
            base_url: base.map(str::to_string),
            model: model.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn providers_resolve_to_endpoints() {
        assert_eq!(endpoint(&cfg(None, None, None)), Ok(None), "default: the claude command");
        assert_eq!(endpoint(&cfg(Some("claude"), None, None)), Ok(None));
        let or = endpoint(&cfg(Some("openrouter"), None, Some("x-ai/grok-4"))).unwrap().unwrap();
        assert_eq!((or.base_url.as_str(), or.key_env.as_str(), or.label.as_str()), ("https://openrouter.ai/api/v1", "OPENROUTER_API_KEY", "openrouter x-ai/grok-4"));
        assert!(!endpoint(&cfg(Some("openrouter"), None, None)).unwrap().unwrap().model.is_empty(), "a default model");
        // any other OpenAI-compatible API by its base_url (no key unless api_key_env names one)
        let local = endpoint(&cfg(None, Some("http://127.0.0.1:8080/v1/"), Some("llama"))).unwrap().unwrap();
        assert_eq!((local.base_url.as_str(), local.key_env.as_str()), ("http://127.0.0.1:8080/v1", ""));
        assert!(endpoint(&cfg(None, Some("http://x/v1"), None)).unwrap_err().contains("model"));
        assert!(endpoint(&cfg(Some("codex"), None, None)).unwrap_err().contains("not supported yet"));
        assert!(endpoint(&cfg(Some("nope"), None, None)).is_err());
    }

    #[test]
    fn http_errors_read_as_words() {
        assert!(http_problem(402, r#"{"error":{"message":"Insufficient credits"}}"#).contains("no credits left") );
        assert!(http_problem(429, "slow down").contains("rate limited"));
        assert!(http_problem(401, "").contains("key was refused"));
    }
}
