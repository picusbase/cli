use function_kit::{error_response, json_response, parse_body, Error, DEFAULT_LIMIT, DEFAULT_PAGE};
use picus_wasm_sdk::{export, ExecutionContext, HttpHandler, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    #[serde(default = "default_page")]
    page: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_page() -> i64 {
    DEFAULT_PAGE
}

fn default_limit() -> i64 {
    DEFAULT_LIMIT
}

#[derive(Debug, Serialize)]
struct {{STRUCT_NAME}}Item {
    id: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct {{STRUCT_NAME}}ListResponse {
    data: Vec<{{STRUCT_NAME}}Item>,
    page: i64,
    limit: i64,
    count: usize,
}

struct {{STRUCT_NAME}};

impl HttpHandler for {{STRUCT_NAME}} {
    fn handle(req: HttpRequest, _ctx: ExecutionContext) -> Result<HttpResponse, Error> {
        let params: Params = parse_body(&req)?;

        match params.action.as_str() {
            "list" => Ok(json_response(
                200,
                &{{STRUCT_NAME}}ListResponse {
                    data: Vec::new(),
                    page: params.page,
                    limit: params.limit,
                    count: 0,
                },
            )),
            other => Ok(error_response(
                400,
                "UNKNOWN_ACTION",
                &format!("Unknown action: {other}"),
            )),
        }
    }
}

export!({{STRUCT_NAME}});
