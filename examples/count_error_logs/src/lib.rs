use picus_wasm_sdk::{
    export, host, Error, ExecutionContext, HttpHandler, HttpRequest, HttpResponse,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

struct CountErrorLogs;

#[derive(Deserialize)]
struct Input {
    #[serde(default)]
    project_ref: Option<String>,
}

fn request_body(req: &HttpRequest) -> Value {
    req.body
        .clone()
        .and_then(|body| body.get("params").cloned().or(Some(body)))
        .unwrap_or(Value::Null)
}

impl HttpHandler for CountErrorLogs {
    fn handle(req: HttpRequest, ctx: ExecutionContext) -> Result<HttpResponse, Error> {
        host::log_info(&format!("CountErrorLogs started. Execution: {}", ctx.execution_id));

        let raw_body = request_body(&req);
        let input: Input = if raw_body.is_null() {
            Input { project_ref: None }
        } else {
            serde_json::from_value(raw_body)
                .map_err(|_| Error::from("Invalid parameters. Expected {project_ref?: string}"))?
        };

        let schema_name = input.project_ref.unwrap_or_else(|| ctx.project_ref.clone());

        let sql = format!(
            "SELECT count(*) as count FROM \"{}\".logs WHERE level = 'ERROR'",
            schema_name
        );

        host::log_info(&format!("Executing SQL: {}", sql));

        let rows = host::query_read_only(&sql, &[])?;

        let count = if let Some(row) = rows.first() {
            match row.get("count") {
                Some(val) => val.as_i64().unwrap_or(0),
                None => 0,
            }
        } else {
            0
        };

        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: json!({
                "error_count": count,
                "schema": schema_name,
                "method": req.method,
            }),
        })
    }
}

export!(CountErrorLogs);
