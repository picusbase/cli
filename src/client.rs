use anyhow::Result;
use reqwest::blocking::{multipart, Client};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Phase 4 slice A — optional fields the CLI can attach to a publish call.
/// Every field is `Option<String>`; the control plane (Phase 2 slice B/C)
/// treats all of them as nullable on the receiving side, so older CLIs
/// continue to work and newer CLIs can opt in field-by-field.
#[derive(Debug, Default, Clone)]
pub struct PublishExtras {
    pub language: Option<String>,
    pub runtime_class: Option<String>,
    pub entrypoint: Option<String>,
    pub manifest_checksum: Option<String>,
    pub allowed_capabilities: Option<String>,
    pub signature: Option<String>,
    pub signer_id: Option<String>,
    pub signature_algorithm: Option<String>,
    pub sbom_checksum: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, tabled::Tabled)]
pub struct Function {
    pub id: String,
    pub name: String,
    #[tabled(display_with = "display_option")]
    pub created_at: Option<String>,
    #[tabled(display_with = "display_option")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ComputeFunction {
    pub id: String,
    pub function_key: String,
    pub name: String,
    pub project_ref: String,
}

#[derive(Debug, Serialize, Deserialize, tabled::Tabled)]
pub struct RouteBinding {
    pub id: String,
    #[tabled(skip)]
    pub project_ref: String,
    pub path_pattern: String,
    pub http_method: String,
    pub function_id: String,
    #[tabled(skip)]
    pub version_id: Option<String>,
    #[tabled(skip)]
    pub policy_id: Option<String>,
    pub is_active: bool,
    #[serde(default)]
    pub auth_mode: String,
    #[serde(default)]
    #[tabled(display_with = "display_list")]
    pub required_permissions: Vec<String>,
    #[serde(default)]
    pub source: String,
}

#[derive(Serialize)]
struct CreateRouteBindingRequest {
    pub path_pattern: String,
    pub http_method: Option<String>,
    pub function_id: String,
    pub version_id: Option<String>,
    pub policy_id: Option<String>,
    pub auth_mode: Option<String>,
    pub required_permissions: Option<Vec<String>>,
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, tabled::Tabled)]
pub struct Job {
    pub id: String,
    pub status: String,
    pub function_id: String,
    #[serde(default)]
    #[tabled(display_with = "display_option")]
    pub runtime_class: Option<String>,
    #[tabled(skip)]
    pub params: serde_json::Value,
    #[tabled(skip)]
    pub result: Option<serde_json::Value>,
    #[tabled(display_with = "display_option")]
    pub error_message: Option<String>,
    #[serde(default)]
    #[tabled(display_with = "display_option")]
    pub error_class: Option<String>,
    #[tabled(display_with = "display_option")]
    pub created_at: Option<String>,
    #[tabled(display_with = "display_option")]
    pub scheduled_at: Option<String>,
    #[tabled(display_with = "display_option")]
    pub started_at: Option<String>,
    #[tabled(display_with = "display_option")]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub attempt_count: i32,
    #[serde(default)]
    pub max_retries: i32,
    #[serde(default)]
    pub retry_backoff_ms: i32,
}

#[derive(Debug, Serialize, Deserialize, tabled::Tabled)]
pub struct JobSchedule {
    pub id: String,
    pub name: String,
    pub cron_expression: String,
    pub function_id: String,
    #[tabled(skip)]
    pub default_params: serde_json::Value,
    pub is_active: bool,
    #[tabled(display_with = "display_option")]
    pub last_run_at: Option<String>,
    #[tabled(display_with = "display_option")]
    pub next_run_at: Option<String>,
}

fn display_option(o: &Option<String>) -> String {
    o.as_deref().unwrap_or("-").to_string()
}

fn display_list(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_string()
    } else {
        items.join(", ")
    }
}

#[derive(Serialize)]
struct CreateJobRequest {
    function_id: String,
    wasm_module: String,
    params: serde_json::Value,
}

#[derive(Serialize)]
struct RunFunctionRequest {
    params: serde_json::Value,
}

#[derive(Serialize)]
struct CreateScheduleRequest {
    name: String,
    cron_expression: String,
    function_id: String,
    default_params: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OpenApiFragment {
    pub id: String,
    pub function_id: String,
    pub domain: String,
    pub fragment: serde_json::Value,
}

#[derive(Serialize)]
struct CreateOpenApiFragmentRequest {
    function_id: String,
    domain: String,
    fragment: serde_json::Value,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct AggregatedSpec {
    pub domain: String,
    pub spec: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct PublishedVersion {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct PublishResult {
    pub function: Function,
    pub version: PublishedVersion,
}

pub struct PicusClient {
    client: Client,
    base_url: String,
    project_ref: Option<String>,
    executor_url: Option<String>,
    auth_token: Option<String>,
    user_id: Option<String>,
    user_role: Option<String>,
    personal_token: Option<String>,
    console_token: std::sync::OnceLock<String>,
}

impl PicusClient {
    pub fn new(
        base_url: String,
        project_ref: Option<String>,
        executor_url: Option<String>,
        auth_token: Option<String>,
        user_id: Option<String>,
        user_role: Option<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            base_url,
            project_ref,
            executor_url,
            auth_token,
            user_id,
            user_role,
            personal_token: None,
            console_token: std::sync::OnceLock::new(),
        }
    }

    pub fn with_personal_token(mut self, personal_token: Option<String>) -> Self {
        self.personal_token = personal_token;
        self
    }

    /// With a personal access token `base_url` is the Console URL and requests go through
    /// console-api's authenticated `/platform/projects/:ref/compute` proxy; otherwise straight to
    /// the Compute Control Plane. `console_path: None` marks an endpoint console-api does not proxy.
    fn url(&self, cp_path: &str, console_path: Option<&str>) -> Result<String> {
        if self.personal_token.is_none() {
            return Ok(format!("{}/compute{}", self.base_url, cp_path));
        }
        let console_path = console_path.ok_or_else(|| {
            anyhow::anyhow!("This command is not available through console-api yet")
        })?;
        let project_ref = self.project_ref.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "project_ref is required: --project-ref, PICUS_PROJECT_REF or `picusbase config set --default-project-ref`"
            )
        })?;
        Ok(format!(
            "{}/platform/projects/{}/compute{}",
            self.base_url, project_ref, console_path
        ))
    }

    fn console_token(&self, personal_token: &str) -> Result<String> {
        if let Some(token) = self.console_token.get() {
            return Ok(token.clone());
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        let res = self
            .client
            .post(format!("{}/api/auth/oauth/token", self.base_url))
            .json(&serde_json::json!({
                "grant_type": "personal_access_token",
                "personal_access_token": personal_token,
            }))
            .send()?;
        if !res.status().is_success() {
            anyhow::bail!(
                "Personal access token rejected by {} ({}): token revoked, expired, or wrong --url",
                self.base_url,
                res.status()
            );
        }
        let token = res.json::<TokenResponse>()?.access_token;
        Ok(self.console_token.get_or_init(|| token).clone())
    }

    fn request_builder(
        &self,
        method: reqwest::Method,
        url: &str,
    ) -> Result<reqwest::blocking::RequestBuilder> {
        let mut req = self.client.request(method, url);
        if let Some(ref p_ref) = self.project_ref {
            req = req.header("x-project-ref", p_ref);
        }
        if let Some(ref personal_token) = self.personal_token {
            req = req.bearer_auth(self.console_token(personal_token)?);
        }
        Ok(req)
    }

    // Functions
    pub fn list_functions(&self) -> Result<Vec<Function>> {
        let url = self.url("/registry/functions", Some("/functions"))?;
        let res = self.request_builder(reqwest::Method::GET, &url)?.send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to list functions: {}", text);
        }

        Ok(res.json()?)
    }

    #[allow(dead_code)]
    pub fn upload_function(&self, id: String, name: String, file_path: &Path) -> Result<Function> {
        self.upload_function_with_extras(id, name, file_path, &PublishExtras::default())
    }

    pub fn upload_function_publish_result_with_extras(
        &self,
        id: String,
        name: String,
        file_path: &Path,
        extras: &PublishExtras,
    ) -> Result<PublishResult> {
        let url = self.url("/registry/publish", Some("/functions"))?;

        let mut form = multipart::Form::new()
            .text("function_key", id)
            .text("name", name)
            .file("wasm_file", file_path)?;

        if let Some(ref v) = extras.runtime_class {
            form = form.text("runtime_class", v.clone());
        }
        if let Some(ref v) = extras.language {
            form = form.text("language", v.clone());
        }
        if let Some(ref v) = extras.entrypoint {
            form = form.text("entrypoint", v.clone());
        }
        if let Some(ref v) = extras.manifest_checksum {
            form = form.text("manifest_checksum", v.clone());
        }
        if let Some(ref v) = extras.allowed_capabilities {
            form = form.text("allowed_capabilities", v.clone());
        }
        if let Some(ref v) = extras.signature {
            form = form.text("signature", v.clone());
        }
        if let Some(ref v) = extras.signer_id {
            form = form.text("signer_id", v.clone());
        }
        if let Some(ref v) = extras.signature_algorithm {
            form = form.text("signature_algorithm", v.clone());
        }
        if let Some(ref v) = extras.sbom_checksum {
            form = form.text("sbom_checksum", v.clone());
        }

        let res = self
            .request_builder(reqwest::Method::POST, &url)?
            .multipart(form)
            .send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to upload function: {}", text);
        }

        Ok(res.json()?)
    }

    pub fn upload_function_with_extras(
        &self,
        id: String,
        name: String,
        file_path: &Path,
        extras: &PublishExtras,
    ) -> Result<Function> {
        Ok(self
            .upload_function_publish_result_with_extras(id, name, file_path, extras)?
            .function)
    }

    pub fn activate_version(&self, version_id: &str) -> Result<()> {
        let url = self.url(
            &format!("/registry/versions/{version_id}/activate"),
            Some(&format!("/versions/{version_id}/activate")),
        )?;
        let res = self.request_builder(reqwest::Method::POST, &url)?.send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to activate version: {}", text);
        }

        Ok(())
    }

    pub fn delete_function(&self, id: &str) -> Result<bool> {
        let url = self.url(&format!("/registry/functions/{id}"), Some(&format!("/functions/{id}")))?;
        let res = self.request_builder(reqwest::Method::DELETE, &url)?.send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to delete function: {}", text);
        }

        Ok(res.json()?)
    }

    pub fn run_function(&self, id: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        if let Some(ref executor_url) = self.executor_url {
            let url = format!(
                "{}/functions/{}/execute",
                executor_url.trim_end_matches('/'),
                id
            );
            let mut req = self.client.post(&url).json(&params);

            if let Some(ref project_ref) = self.project_ref {
                req = req.header("x-project-ref", project_ref);
            }

            if let Some(ref auth_token) = self.auth_token {
                req = req.header("authorization", format!("Bearer {}", auth_token));
            }

            if let Some(ref user_id) = self.user_id {
                req = req.header("x-picus-user-id", user_id);
            }

            if let Some(ref user_role) = self.user_role {
                req = req.header("x-picus-user-role", user_role);
            }

            let res = req.send()?;

            if !res.status().is_success() {
                let text = res.text()?;
                anyhow::bail!("Failed to run function: {}", text);
            }

            return Ok(res.json()?);
        }

        let url = self.url(&format!("/functions/{id}/run"), None)?;
        let payload = RunFunctionRequest { params };

        let res = self
            .request_builder(reqwest::Method::POST, &url)?
            .json(&payload)
            .send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to run function: {}", text);
        }

        Ok(res.json()?)
    }

    // Jobs
    pub fn list_jobs(&self) -> Result<Vec<Job>> {
        let url = self.url("/jobs", Some("/jobs"))?;
        let res = self.request_builder(reqwest::Method::GET, &url)?.send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to list jobs: {}", text);
        }

        Ok(res.json()?)
    }

    pub fn create_job(&self, function_id: String, params: serde_json::Value) -> Result<Job> {
        let url = self.url("/jobs", Some("/jobs"))?;
        let payload = CreateJobRequest {
            function_id: function_id.clone(),
            wasm_module: function_id,
            params,
        };

        let res = self
            .request_builder(reqwest::Method::POST, &url)?
            .json(&payload)
            .send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to create job: {}", text);
        }

        Ok(res.json()?)
    }

    pub fn delete_job(&self, id: &str) -> Result<bool> {
        let url = self.url(&format!("/jobs/{id}"), Some(&format!("/jobs/{id}")))?;
        let res = self.request_builder(reqwest::Method::DELETE, &url)?.send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to delete job: {}", text);
        }

        Ok(res.json()?)
    }

    // Schedules
    pub fn list_schedules(&self) -> Result<Vec<JobSchedule>> {
        let url = self.url("/schedules", None)?;
        let res = self.request_builder(reqwest::Method::GET, &url)?.send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to list schedules: {}", text);
        }

        Ok(res.json()?)
    }

    pub fn create_schedule(
        &self,
        name: String,
        cron: String,
        function_id: String,
        params: serde_json::Value,
    ) -> Result<String> {
        let url = self.url("/schedules", None)?;
        let payload = CreateScheduleRequest {
            name,
            cron_expression: cron,
            function_id,
            default_params: params,
        };

        let res = self.client.post(&url).json(&payload).send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to create schedule: {}", text);
        }

        // The API returns the ID as a string (Json<String>)
        Ok(res.json()?)
    }

    pub fn delete_schedule(&self, id: &str) -> Result<bool> {
        let url = self.url(&format!("/schedules/{id}"), None)?;
        let res = self.client.delete(&url).send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to delete schedule: {}", text);
        }

        Ok(res.json()?)
    }

    // OpenAPI Fragments
    #[allow(dead_code)]
    pub fn list_openapi_fragments(&self, domain: Option<&str>) -> Result<Vec<OpenApiFragment>> {
        let mut url = self.url("/openapi/fragments", None)?;
        if let Some(d) = domain {
            url = format!("{}?domain={}", url, d);
        }
        let res = self.request_builder(reqwest::Method::GET, &url)?.send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to list OpenAPI fragments: {}", text);
        }

        Ok(res.json()?)
    }

    pub fn upsert_openapi_fragment(
        &self,
        function_id: String,
        domain: String,
        fragment: serde_json::Value,
    ) -> Result<OpenApiFragment> {
        let url = self.url("/openapi/fragments", Some("/openapi/fragments"))?;
        let payload = CreateOpenApiFragmentRequest {
            function_id,
            domain,
            fragment,
        };
        let res = self
            .request_builder(reqwest::Method::POST, &url)?
            .json(&payload)
            .send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to upsert OpenAPI fragment: {}", text);
        }

        Ok(res.json()?)
    }

    #[allow(dead_code)]
    pub fn delete_openapi_fragment(&self, id: &str) -> Result<bool> {
        let url = self.url(&format!("/openapi/fragments/{id}"), None)?;
        let res = self.request_builder(reqwest::Method::DELETE, &url)?.send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to delete OpenAPI fragment: {}", text);
        }

        Ok(res.json()?)
    }

    #[allow(dead_code)]
    pub fn get_aggregated_spec(&self, domain: &str) -> Result<AggregatedSpec> {
        let url = self.url(&format!("/openapi/{domain}.json"), None)?;
        let res = self.request_builder(reqwest::Method::GET, &url)?.send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!(
                "Failed to get aggregated spec for domain '{}': {}",
                domain,
                text
            );
        }

        Ok(res.json()?)
    }

    // Control Plane: list functions (returns UUID + function_key)
    pub fn list_compute_functions(&self) -> Result<Vec<ComputeFunction>> {
        let url = self.url("/registry/functions", Some("/functions"))?;
        let res = self.request_builder(reqwest::Method::GET, &url)?.send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to list compute functions: {}", text);
        }

        Ok(res.json()?)
    }

    // Route Bindings
    pub fn list_route_bindings(&self) -> Result<Vec<RouteBinding>> {
        let url = self.url("/bindings/routes", Some("/route-bindings"))?;
        let res = self.request_builder(reqwest::Method::GET, &url)?.send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to list route bindings: {}", text);
        }

        Ok(res.json()?)
    }

    pub fn create_route_binding(
        &self,
        path_pattern: String,
        http_method: String,
        function_id: String,
        auth_mode: Option<String>,
        required_permissions: Option<Vec<String>>,
        source: Option<String>,
    ) -> Result<RouteBinding> {
        let url = self.url("/bindings/routes", Some("/route-bindings"))?;
        let payload = CreateRouteBindingRequest {
            path_pattern,
            http_method: Some(http_method),
            function_id,
            version_id: None,
            policy_id: None,
            auth_mode,
            required_permissions,
            source,
        };
        let res = self
            .request_builder(reqwest::Method::POST, &url)?
            .json(&payload)
            .send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to create route binding: {}", text);
        }

        Ok(res.json()?)
    }

    pub fn delete_route_binding(&self, id: &str) -> Result<bool> {
        let url = self.url(&format!("/bindings/routes/{id}"), Some(&format!("/route-bindings/{id}")))?;
        let res = self.request_builder(reqwest::Method::DELETE, &url)?.send()?;

        if !res.status().is_success() {
            let text = res.text()?;
            anyhow::bail!("Failed to delete route binding: {}", text);
        }

        Ok(res.json()?)
    }
}
