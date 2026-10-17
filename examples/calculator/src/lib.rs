use picus_wasm_sdk::{
    export, host, Error, ExecutionContext, HttpHandler, HttpRequest, HttpResponse,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

struct Calculator;

#[derive(Deserialize)]
struct Input {
    operation: String,
    a: f64,
    b: f64,
}

fn request_body(req: &HttpRequest) -> Value {
    req.body
        .clone()
        .and_then(|body| body.get("params").cloned().or(Some(body)))
        .unwrap_or(Value::Null)
}

impl HttpHandler for Calculator {
    fn handle(req: HttpRequest, ctx: ExecutionContext) -> Result<HttpResponse, Error> {
        host::log_info(&format!("Calculator started. Execution: {}", ctx.execution_id));

        let raw_body = request_body(&req);
        let input: Input = serde_json::from_value(raw_body).map_err(|_| {
            Error::from("Invalid parameters. Expected {operation: string, a: number, b: number}")
        })?;

        let result = match input.operation.as_str() {
            "add" => input.a + input.b,
            "sub" => input.a - input.b,
            "mul" => input.a * input.b,
            "div" => {
                if input.b == 0.0 {
                    return Ok(HttpResponse {
                        status: 400,
                        headers: HashMap::new(),
                        body: json!({
                            "error": true,
                            "message": "Division by zero",
                        }),
                    });
                }
                input.a / input.b
            }
            _ => {
                return Ok(HttpResponse {
                    status: 400,
                    headers: HashMap::new(),
                    body: json!({
                        "error": true,
                        "message": "Unknown operation. Supported: add, sub, mul, div",
                    }),
                })
            }
        };

        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: json!({
                "result": result,
                "operation": input.operation,
                "method": req.method,
            }),
        })
    }
}

export!(Calculator);
