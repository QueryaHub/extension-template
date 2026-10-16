//! The driver: what each `db.*` method does. This is the file to change.
//!
//! Every handler returns `Ok(result)` or `Err(DriverError)`; `main.rs` turns
//! that into a JSON-RPC response. Secrets (the password) arrive in
//! `system.injectCredentials` or in `db.connect`; never log them.

use crate::resp::{split_command, Client, Reply};
use serde_json::{json, Value};
use std::collections::HashMap;

pub struct DriverError {
    pub code: i64,
    pub message: String,
}

impl DriverError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self::new(-32700, message)
    }
    pub fn method_not_found(method: &str) -> Self {
        Self::new(-32601, format!("method not found: {method}"))
    }
    /// A failure of the database itself (bad query, connection lost, ...).
    fn database(message: impl Into<String>) -> Self {
        Self::new(-32000, message)
    }
}

type Result<T> = std::result::Result<T, DriverError>;

#[derive(Default)]
pub struct Driver {
    /// Open connections by the `connectionId` the host chose.
    sessions: HashMap<i64, Client>,
    /// Passwords from `system.injectCredentials`, waiting for `db.connect`.
    secrets: HashMap<i64, String>,
}

impl Driver {
    pub fn handle(&mut self, method: &str, params: &Value) -> Result<Value> {
        match method {
            "system.handshake" => Ok(json!({
                "name": env!("CARGO_PKG_NAME"),
                "version": env!("CARGO_PKG_VERSION"),
            })),
            "system.ping" => Ok(json!({ "ok": true })),
            // main.rs exits after this answer; the host waits for exit code 0.
            "system.shutdown" => Ok(json!({ "ok": true })),
            "system.injectCredentials" => self.inject_credentials(params),
            "db.connect" => self.connect(params),
            "db.disconnect" => self.disconnect(params),
            "db.getCapabilities" => Ok(json!({
                "supportsTransactions": false,
                "supportsCancel": false,
                "supportsDDLInspection": false,
                "supportsPrivileges": false,
                "hasServerStats": true,
                "supportsMutations": false,
                "supportsBatchMutations": false,
            })),
            "db.getServerStats" => self.server_stats(params),
            "db.getSchemaTree" => self.schema_tree(params),
            "db.getTableSchema" => self.table_schema(params),
            "db.query" => self.query(params),
            other => Err(DriverError::method_not_found(other)),
        }
    }

    fn inject_credentials(&mut self, params: &Value) -> Result<Value> {
        let id = connection_id(params)?;
        if let Some(password) = params.get("password").and_then(Value::as_str) {
            self.secrets.insert(id, password.to_string());
        }
        Ok(json!({ "ok": true }))
    }

    fn connect(&mut self, params: &Value) -> Result<Value> {
        let id = connection_id(params)?;
        let (host, port) = endpoint(params)?;
        let password = params
            .get("password")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| self.secrets.remove(&id));

        let mut client = Client::connect(&host, port).map_err(DriverError::database)?;
        if let Some(password) = password.filter(|p| !p.is_empty()) {
            // The username is optional (Redis 6 ACL); `user` is what the host sends.
            let user = params
                .get("user")
                .or_else(|| params.get("username"))
                .and_then(Value::as_str);
            let auth: Vec<&str> = match user {
                Some(user) if !user.is_empty() => vec!["AUTH", user, &password],
                _ => vec!["AUTH", &password],
            };
            client.command(&auth).map_err(DriverError::database)?;
        }
        client.command(&["PING"]).map_err(DriverError::database)?;
        let version = info(&mut client)?
            .get("redis_version")
            .cloned()
            .unwrap_or_default();
        self.sessions.insert(id, client);
        Ok(json!({ "connected": true, "serverVersion": version }))
    }

    fn disconnect(&mut self, params: &Value) -> Result<Value> {
        let id = connection_id(params)?;
        self.sessions.remove(&id);
        self.secrets.remove(&id);
        Ok(json!({ "ok": true }))
    }

    fn server_stats(&mut self, params: &Value) -> Result<Value> {
        let info = info(self.session(params)?)?;
        let number = |key: &str| info.get(key).and_then(|v| v.parse::<i64>().ok());
        Ok(json!({
            "serverVersion": info.get("redis_version"),
            "uptimeSeconds": number("uptime_in_seconds"),
            "activeConnections": number("connected_clients"),
            "memoryUsageBytes": number("used_memory"),
            "extraMetrics": { "role": info.get("role") },
        }))
    }

    /// The tree in the sidebar. Every key is a leaf under one "Keys" folder.
    fn schema_tree(&mut self, params: &Value) -> Result<Value> {
        let client = self.session(params)?;
        let keys = scan_keys(client, 200)?;
        let children: Vec<Value> = keys
            .into_iter()
            .map(|key| json!({ "id": format!("key:{key}"), "label": key, "nodeType": "table" }))
            .collect();
        Ok(json!({ "roots": [{
            "id": "keys",
            "label": "Keys",
            "nodeType": "database",
            "children": children,
        }] }))
    }

    /// Column metadata for the data grid. A key has the same shape for every name.
    fn table_schema(&mut self, params: &Value) -> Result<Value> {
        self.session(params)?;
        let table = params
            .get("tableName")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let column = |name: &str, pk: bool| json!({ "name": name, "dataType": "string", "isNullable": !pk, "isPrimaryKey": pk });
        Ok(json!({
            "tableName": table,
            "columns": [column("key", true), column("type", false), column("ttl", false)],
            "primaryKeys": ["key"],
        }))
    }

    /// `sql` is a command line of the database ("GET some:key"). The result is
    /// always `{"columns": [...], "rows": [[...], ...]}`, the shape the grid reads.
    fn query(&mut self, params: &Value) -> Result<Value> {
        let sql = params
            .get("sql")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(1000) as usize;
        let words = split_command(sql);
        if words.is_empty() {
            return Err(DriverError::invalid_params("empty command"));
        }
        let args: Vec<&str> = words.iter().map(String::as_str).collect();
        let reply = self
            .session(params)?
            .command(&args)
            .map_err(DriverError::database)?;

        let mut cells = Vec::new();
        flatten(&reply, &mut cells);
        cells.truncate(limit);
        Ok(json!({
            "columns": ["result"],
            "rows": cells.into_iter().map(|c| vec![c]).collect::<Vec<_>>(),
        }))
    }

    fn session(&mut self, params: &Value) -> Result<&mut Client> {
        let id = connection_id(params)?;
        self.sessions
            .get_mut(&id)
            .ok_or_else(|| DriverError::database(format!("connection {id} is not open")))
    }
}

fn connection_id(params: &Value) -> Result<i64> {
    params
        .get("connectionId")
        .and_then(Value::as_i64)
        .ok_or_else(|| DriverError::invalid_params("connectionId is required"))
}

/// Host and port from `host` / `port`, or from the `connectionString`
/// (`scheme://host:port/database`) that Querya Desktop sends.
fn endpoint(params: &Value) -> Result<(String, u16)> {
    if let Some(host) = params
        .get("host")
        .and_then(Value::as_str)
        .filter(|h| !h.is_empty())
    {
        let port = params.get("port").and_then(Value::as_u64).unwrap_or(6379);
        return Ok((host.to_string(), port as u16));
    }
    if let Some(url) = params.get("connectionString").and_then(Value::as_str) {
        let rest = url.split("://").last().unwrap_or(url);
        let authority = rest.split('/').next().unwrap_or(rest);
        let authority = authority.rsplit('@').next().unwrap_or(authority);
        let (host, port) = authority.split_once(':').unwrap_or((authority, "6379"));
        if let (false, Ok(port)) = (host.is_empty(), port.parse::<u16>()) {
            return Ok((host.to_string(), port));
        }
    }
    Err(DriverError::invalid_params(
        "host (and port) or connectionString is required",
    ))
}

fn info(client: &mut Client) -> Result<HashMap<String, String>> {
    let Reply::Bulk(Some(text)) = client.command(&["INFO"]).map_err(DriverError::database)? else {
        return Err(DriverError::database("INFO returned an unexpected reply"));
    };
    Ok(text
        .lines()
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect())
}

/// Up to `max` key names, in the order SCAN returns them.
fn scan_keys(client: &mut Client, max: usize) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    let mut cursor = "0".to_string();
    loop {
        let reply = client
            .command(&["SCAN", &cursor, "COUNT", "100"])
            .map_err(DriverError::database)?;
        let Reply::Array(parts) = reply else { break };
        let [Reply::Bulk(Some(next)), Reply::Array(batch)] = parts.as_slice() else {
            break;
        };
        keys.extend(batch.iter().filter_map(|k| match k {
            Reply::Bulk(Some(name)) => Some(name.clone()),
            _ => None,
        }));
        cursor = next.clone();
        if cursor == "0" || keys.len() >= max {
            break;
        }
    }
    keys.sort();
    keys.truncate(max);
    Ok(keys)
}

/// A reply as grid cells: one per scalar, nested arrays are flattened.
fn flatten(reply: &Reply, out: &mut Vec<Value>) {
    match reply {
        Reply::Status(s) => out.push(json!(s)),
        Reply::Int(n) => out.push(json!(n)),
        Reply::Bulk(Some(s)) => out.push(json!(s)),
        Reply::Bulk(None) => out.push(Value::Null),
        Reply::Array(items) => items.iter().for_each(|i| flatten(i, out)),
    }
}
