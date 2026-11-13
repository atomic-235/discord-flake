//! Discord REST CLI.
//!
//! The `discord` wrapper runs this under `with-nono personal`: the process only
//! ever holds a phantom `DISCORD_TOKEN`, and the credential proxy substitutes
//! the real user token on requests to discord.com.

use std::env;
use std::process::exit;
use std::time::Duration;

use chrono::{DateTime, Local};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};

const API: &str = "https://discord.com/api/v10";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

const CHANNEL_TYPES: &[(i64, &str)] = &[
    (0, "text"),
    (2, "voice"),
    (4, "category"),
    (5, "announcement"),
    (10, "thread"),
    (11, "thread"),
    (12, "thread"),
    (13, "stage"),
    (15, "forum"),
    (16, "media"),
];

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("{}", message.as_ref());
    exit(1);
}

fn token() -> String {
    match env::var("DISCORD_TOKEN") {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => fail(
            "DISCORD_TOKEN is not set — run through the `discord` wrapper \
             (the nono proxy injects it from secrets/personal.yaml)",
        ),
    }
}

fn channel_kind(value: &Value) -> String {
    match value.as_i64() {
        Some(n) => CHANNEL_TYPES
            .iter()
            .find(|(k, _)| *k == n)
            .map(|(_, name)| (*name).to_string())
            .unwrap_or_else(|| n.to_string()),
        None => "?".to_string(),
    }
}

fn api(method: &str, path: &str, params: &[(&str, String)], body: Option<Value>) -> Value {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(client) => client,
        Err(error) => fail(format!("Network error: {error}")),
    };
    let url = format!("{API}{path}");
    let request_method = match reqwest::Method::from_bytes(method.as_bytes()) {
        Ok(m) => m,
        Err(error) => fail(format!("Invalid method {method}: {error}")),
    };

    for attempt in 0..3 {
        let mut request = client
            .request(request_method.clone(), &url)
            .header("Authorization", token())
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json");
        if !params.is_empty() {
            request = request.query(params);
        }
        if let Some(payload) = &body {
            request = request.json(payload);
        }

        let response = match request.send() {
            Ok(response) => response,
            Err(error) => fail(format!("Network error: {error}")),
        };
        let status = response.status().as_u16();
        let raw = response.text().unwrap_or_default();

        if status == 429 && attempt < 2 {
            let delay = serde_json::from_str::<Value>(&raw)
                .ok()
                .and_then(|value| value.get("retry_after").and_then(Value::as_f64))
                .unwrap_or(1.0)
                .min(30.0);
            std::thread::sleep(Duration::from_secs_f64(delay));
            continue;
        }
        if status == 401 {
            fail("Discord rejected the token (401) — it expired or was revoked");
        }
        if status == 403 {
            fail(format!("Missing permission (403) for {method} {path}"));
        }
        if !(200..300).contains(&status) {
            fail(format!("Discord returned {status} for {method} {path}: {raw}"));
        }
        if raw.is_empty() {
            return Value::Null;
        }
        return serde_json::from_str(&raw).unwrap_or(Value::Null);
    }
    fail("Rate limited by Discord — try again shortly")
}

fn local_time(value: &Value) -> String {
    let raw = value.as_str().unwrap_or("");
    match DateTime::parse_from_rfc3339(raw) {
        Ok(parsed) => parsed
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        Err(_) => raw.to_string(),
    }
}

fn guilds() -> Vec<Value> {
    api(
        "GET",
        "/users/@me/guilds",
        &[("limit", "200".to_string())],
        None,
    )
    .as_array()
    .cloned()
    .unwrap_or_default()
}

fn guild_channels(guild_id: &str) -> Vec<Value> {
    api("GET", &format!("/guilds/{guild_id}/channels"), &[], None)
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn dm_channels() -> Vec<Value> {
    api("GET", "/users/@me/channels", &[], None)
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn dm_label(channel: &Value) -> String {
    if let Some(name) = channel["name"].as_str() {
        if !name.is_empty() {
            return name.to_string();
        }
    }
    let recipients: Vec<String> = channel["recipients"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|recipient| {
                    recipient["global_name"]
                        .as_str()
                        .or_else(|| recipient["username"].as_str())
                        .unwrap_or("?")
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    if recipients.is_empty() {
        channel["id"].as_str().unwrap_or("?").to_string()
    } else {
        recipients.join(", ")
    }
}

fn resolve_guild(spec: &str) -> Value {
    if spec.chars().all(|c| c.is_ascii_digit()) {
        if let Some(guild) = guilds()
            .into_iter()
            .find(|guild| guild["id"].as_str() == Some(spec))
        {
            return guild;
        }
        return json!({ "id": spec, "name": spec });
    }

    let needle = spec.to_lowercase();
    let matches: Vec<Value> = guilds()
        .into_iter()
        .filter(|guild| {
            guild["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains(&needle)
        })
        .collect();
    if matches.is_empty() {
        fail(format!("No guild matching {spec:?} — run `discord guilds`"));
    }
    if matches.len() > 1 {
        let listing = matches
            .iter()
            .map(|guild| {
                format!(
                    "  {}  {}",
                    guild["id"].as_str().unwrap_or("?"),
                    guild["name"].as_str().unwrap_or("?")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        fail(format!("Ambiguous guild {spec:?}, use an ID:\n{listing}"));
    }
    matches.into_iter().next().unwrap()
}

fn match_channel(channels: &[Value], spec: &str, where_: &str) -> Value {
    let needle = spec.trim_start_matches('#');
    if needle.chars().all(|c| c.is_ascii_digit()) {
        return json!({ "id": needle, "name": needle });
    }
    let needle = needle.to_lowercase();
    let matches: Vec<&Value> = channels
        .iter()
        .filter(|channel| {
            channel["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains(&needle)
        })
        .collect();
    if matches.is_empty() {
        fail(format!("No channel matching {spec:?} in {where_}"));
    }
    if matches.len() > 1 {
        let listing = matches
            .iter()
            .map(|channel| {
                format!(
                    "  {}  {}",
                    channel["id"].as_str().unwrap_or("?"),
                    channel["name"].as_str().unwrap_or("?")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        fail(format!("Ambiguous channel {spec:?} in {where_}:\n{listing}"));
    }
    let channel = matches[0];
    json!({ "id": channel["id"], "name": channel["name"] })
}

fn is_text_channel(channel: &Value) -> bool {
    matches!(channel["type"].as_i64(), Some(0 | 5 | 15 | 16))
}

fn resolve_channel(spec: &str) -> Value {
    if let Some((guild_spec, channel_spec)) = spec.split_once('/') {
        let guild = resolve_guild(guild_spec);
        let channels: Vec<Value> = guild_channels(guild["id"].as_str().unwrap_or_default())
            .into_iter()
            .filter(is_text_channel)
            .collect();
        let where_ = format!("guild {}", guild["name"].as_str().unwrap_or("?"));
        return match_channel(&channels, channel_spec, &where_);
    }

    let bare = spec.trim_start_matches('#');
    if bare.chars().all(|c| c.is_ascii_digit()) {
        return json!({ "id": bare, "name": spec });
    }

    let needle = bare.to_lowercase();
    for channel in dm_channels() {
        if dm_label(&channel).to_lowercase() == needle {
            return json!({ "id": channel["id"], "name": dm_label(&channel) });
        }
    }

    let mut matches: Vec<(String, Value)> = Vec::new();
    for guild in guilds() {
        for channel in guild_channels(guild["id"].as_str().unwrap_or_default()) {
            if !is_text_channel(&channel) {
                continue;
            }
            if channel["name"].as_str().unwrap_or("").to_lowercase() == needle {
                matches.push((
                    guild["name"].as_str().unwrap_or("?").to_string(),
                    channel.clone(),
                ));
            }
        }
    }

    if matches.is_empty() {
        fail(format!(
            "No channel matching {spec:?} — run `discord channels <guild>` or `discord dms`"
        ));
    }
    if matches.len() > 1 {
        let listing = matches
            .iter()
            .map(|(guild, channel)| {
                format!(
                    "  {guild}  {} ({})",
                    channel["name"].as_str().unwrap_or("?"),
                    channel["id"].as_str().unwrap_or("?")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        fail(format!(
            "Ambiguous channel {spec:?}, use ID or guild/channel:\n{listing}"
        ));
    }
    let (_, channel) = &matches[0];
    json!({ "id": channel["id"], "name": channel["name"] })
}

fn author_label(message: &Value) -> String {
    message["author"]["global_name"]
        .as_str()
        .or_else(|| message["author"]["username"].as_str())
        .unwrap_or("?")
        .to_string()
}

fn print_json(value: &Value) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(error) => fail(format!("Failed to render JSON: {error}")),
    }
}

fn print_messages(messages: &[Value]) {
    for message in messages.iter().rev() {
        let attachments = message["attachments"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0);
        let suffix = if attachments > 0 {
            format!("  [{attachments} attachment(s)]")
        } else {
            String::new()
        };
        let content = message["content"]
            .as_str()
            .unwrap_or("")
            .replace('\n', "\n    ");
        println!(
            "[{}] {}: {}{}",
            local_time(&message["timestamp"]),
            author_label(message),
            content,
            suffix
        );
    }
}

fn fetch_messages(channel_id: &str, limit: usize, before: Option<&str>, page: usize) -> Vec<Value> {
    let mut cursor = before.map(str::to_string);
    let mut messages: Vec<Value> = Vec::new();
    for _ in 0..page.max(1) {
        let mut params = vec![("limit", limit.min(100).to_string())];
        if let Some(cursor) = &cursor {
            params.push(("before", cursor.clone()));
        }
        messages = api(
            "GET",
            &format!("/channels/{channel_id}/messages"),
            &params,
            None,
        )
        .as_array()
        .cloned()
        .unwrap_or_default();
        if messages.is_empty() {
            return Vec::new();
        }
        cursor = messages
            .last()
            .and_then(|message| message["id"].as_str())
            .map(str::to_string);
    }
    messages
}

fn cmd_me(json_output: bool) {
    let user = api("GET", "/users/@me", &[], None);
    if json_output {
        print_json(&user);
        return;
    }
    println!(
        "{} ({})",
        user["global_name"]
            .as_str()
            .or_else(|| user["username"].as_str())
            .unwrap_or("?"),
        user["id"].as_str().unwrap_or("?")
    );
    println!("email: {}", user["email"].as_str().unwrap_or("n/a"));
    println!(
        "mfa: {}",
        if user["mfa_enabled"].as_bool().unwrap_or(false) {
            "enabled"
        } else {
            "disabled"
        }
    );
}

fn cmd_guilds(limit: usize, json_output: bool) {
    let items: Vec<Value> = guilds().into_iter().take(limit).collect();
    if json_output {
        print_json(&json!(items));
        return;
    }
    for guild in items {
        println!(
            "{}  {}",
            guild["id"].as_str().unwrap_or("?"),
            guild["name"].as_str().unwrap_or("?")
        );
    }
}

fn cmd_channels(guild_spec: &str, kind: Option<&str>, json_output: bool) {
    let guild = resolve_guild(guild_spec);
    let mut channels = guild_channels(guild["id"].as_str().unwrap_or_default());
    if let Some(kind) = kind {
        channels.retain(|channel| channel_kind(&channel["type"]) == kind);
    }
    if json_output {
        print_json(&json!(channels));
        return;
    }
    channels.sort_by_key(|channel| {
        (
            channel["position"].as_i64().unwrap_or(0),
            channel["name"].as_str().unwrap_or("").to_string(),
        )
    });
    for channel in channels {
        println!(
            "{}  #{}  ({})",
            channel["id"].as_str().unwrap_or("?"),
            channel["name"].as_str().unwrap_or("?"),
            channel_kind(&channel["type"])
        );
    }
}

fn cmd_dms(limit: usize, json_output: bool) {
    let items: Vec<Value> = dm_channels().into_iter().take(limit).collect();
    if json_output {
        print_json(&json!(items));
        return;
    }
    for channel in items {
        println!(
            "{}  {}",
            channel["id"].as_str().unwrap_or("?"),
            dm_label(&channel)
        );
    }
}

fn cmd_messages(
    channel_spec: &str,
    limit: usize,
    before: Option<&str>,
    page: usize,
    json_output: bool,
) {
    let channel = resolve_channel(channel_spec);
    let messages = fetch_messages(
        channel["id"].as_str().unwrap_or_default(),
        limit,
        before,
        page,
    );
    if json_output {
        print_json(&json!(messages));
        return;
    }
    if messages.is_empty() {
        println!("no messages");
        return;
    }
    print_messages(&messages);
}

fn cmd_send(channel_spec: &str, text: &[String], json_output: bool) {
    let channel = resolve_channel(channel_spec);
    let content = text.join(" ");
    if content.chars().count() > 2000 {
        fail("Message exceeds Discord's 2000 character limit");
    }
    let message = api(
        "POST",
        &format!("/channels/{}/messages", channel["id"].as_str().unwrap_or_default()),
        &[],
        Some(json!({ "content": content })),
    );
    if json_output {
        print_json(&message);
        return;
    }
    println!(
        "sent {} to {} at {}",
        message["id"].as_str().unwrap_or("?"),
        channel["name"].as_str().unwrap_or("?"),
        local_time(&message["timestamp"])
    );
}

fn cmd_search(guild_spec: &str, query: &str, limit: usize, page: usize, json_output: bool) {
    let guild = resolve_guild(guild_spec);
    let limit = limit.min(25);
    let offset = ((page.max(1) - 1) * limit).min(5000);
    let result = api(
        "GET",
        &format!(
            "/guilds/{}/messages/search",
            guild["id"].as_str().unwrap_or_default()
        ),
        &[
            ("content", query.to_string()),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
        ],
        None,
    );
    if result["code"].as_i64() == Some(110000) {
        fail(format!(
            "search index still building ({} messages indexed) — retry shortly",
            result["documents_indexed"]
                .as_i64()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".to_string())
        ));
    }
    let hits: Vec<Value> = result["messages"]
        .as_array()
        .map(|groups| {
            groups
                .iter()
                .filter_map(Value::as_array)
                .flatten()
                .filter(|message| message["hit"].as_bool().unwrap_or(false))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if json_output {
        print_json(&json!(hits));
        return;
    }
    if hits.is_empty() {
        println!("no matches");
        return;
    }
    for message in hits {
        let content = message["content"].as_str().unwrap_or("").replace('\n', " ");
        println!(
            "{}  #{}  {}: {}",
            local_time(&message["timestamp"]),
            message["channel_id"].as_str().unwrap_or("?"),
            author_label(&message),
            content
        );
    }
}

#[derive(Parser)]
#[command(name = "discord", about = "Discord CLI (token injected by the nono proxy)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the authenticated account
    Me {
        #[arg(long)]
        json: bool,
    },
    /// List servers
    Guilds {
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// List channels in a server
    Channels {
        /// Server ID or name
        guild: String,
        /// Filter by type (text, voice, category, forum, ...)
        #[arg(long = "type", value_name = "TYPE")]
        kind: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// List direct message channels
    Dms {
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Read messages in a channel
    Messages {
        /// Channel ID, name, or guild/channel
        channel: String,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        /// Page backwards from this message ID
        #[arg(long, value_name = "ID")]
        before: Option<String>,
        #[arg(long, default_value_t = 1)]
        page: usize,
        #[arg(long)]
        json: bool,
    },
    /// Send a message
    Send {
        /// Channel ID, name, or guild/channel
        channel: String,
        /// Message text
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Search messages in a server
    Search {
        /// Server ID or name
        guild: String,
        query: String,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long, default_value_t = 1)]
        page: usize,
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Me { json } => cmd_me(json),
        Command::Guilds { limit, json } => cmd_guilds(limit, json),
        Command::Channels { guild, kind, json } => {
            cmd_channels(&guild, kind.as_deref(), json)
        }
        Command::Dms { limit, json } => cmd_dms(limit, json),
        Command::Messages {
            channel,
            limit,
            before,
            page,
            json,
        } => cmd_messages(&channel, limit, before.as_deref(), page, json),
        Command::Send {
            channel,
            text,
            json,
        } => cmd_send(&channel, &text, json),
        Command::Search {
            guild,
            query,
            limit,
            page,
            json,
        } => cmd_search(&guild, &query, limit, page, json),
    }
}
