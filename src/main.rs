//! Querya Desktop talks to a driver extension over stdio: one JSON-RPC 2.0
//! message per line on stdin, one response per line on stdout. Anything else
//! on stdout breaks the protocol, so logs go to stderr.

mod driver;
mod resp;

use driver::{Driver, DriverError};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

fn main() {
    let mut driver = Driver::default();
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                respond(
                    &mut stdout,
                    Value::Null,
                    Err(DriverError::parse_error(e.to_string())),
                );
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        let result = driver.handle(method, &params);
        respond(&mut stdout, id, result);

        if method == "system.shutdown" {
            // The answer is already flushed; exit so the host sees code 0.
            break;
        }
    }
}

fn respond(out: &mut impl Write, id: Value, result: Result<Value, DriverError>) {
    let message = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(e) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": e.code, "message": e.message } })
        }
    };
    // A closed stdout means the host is gone; there is nobody left to answer.
    let _ = writeln!(out, "{message}").and_then(|_| out.flush());
}
