use picus_wasm_sdk::{
    export, host, Error, ExecutionContext, HttpHandler, HttpRequest, HttpResponse,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

struct HelloWorld;

#[derive(Default, Deserialize)]
struct Input {
    name: Option<String>,
}

fn request_body(req: &HttpRequest) -> Value {
    req.body
        .clone()
        .and_then(|body| body.get("params").cloned().or(Some(body)))
        .unwrap_or(Value::Null)
}

impl HttpHandler for HelloWorld {
    fn handle(req: HttpRequest, ctx: ExecutionContext) -> Result<HttpResponse, Error> {
        host::log_info(&format!("Function started via CLI: {}", ctx.execution_id));

        let raw_body = request_body(&req);
        let input = if raw_body.is_null() {
            Input::default()
        } else {
            serde_json::from_value::<Input>(raw_body.clone())
                .map_err(|e| Error::from(format!("Invalid body: {e}")))?
        };

        let message = format!(
            "Hello from WASM (SDK), {}!",
            input.name.unwrap_or_else(|| "World".to_string())
        );

        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: json!({
                "message": message,
                "method": req.method,
                "path": req.path,
                "execution_id": ctx.execution_id,
                "received_body": raw_body,
            }),
        })
    }
}

export!(HelloWorld);
