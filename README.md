# Picus CLI

CLI để scaffold, build, deploy và quản lý domain Compute của Picus. Đây là lớp thao tác developer/operator cho function registry, direct run, background jobs và schedules.

## Cài đặt

### Yêu cầu

- Rust và Cargo đã có trên máy

### Cài đặt từ source

```bash
cd picusbase/cli
cargo install --path .
```

Nếu không muốn cài vào `$PATH`, có thể chạy trực tiếp qua Cargo:

```bash
cargo run -p picus -- <command>
```

## Khái niệm nhanh

- Rust hoặc guest WASM lane dùng HTTP ABI v2: `HttpHandler::handle(HttpRequest, ExecutionContext) -> Result<HttpResponse, Error>`
- `picus init functions` tạo scaffold theo lane tương ứng: Rust `.wasm`, JavaScript source dưới `src/index.mjs`, hoặc Python source dưới `src/handler.py`
- `picus function run` là lane direct execution tiện lợi cho compatibility/quick smoke test
- App consumer thực tế như dashboard nên gọi function qua `picus_sdk::functions::FunctionClient`, không qua CLI

## Cấu trúc lệnh

```bash
picus [OPTIONS] <COMMAND> <SUBCOMMAND> [ARGS]
```

### Tùy chọn toàn cục

- `--url <URL>`: base URL của Compute Control Plane API. Mặc định `http://localhost:4080/api`
- `--executor-url <URL>`: base URL của HTTP worker route `/functions/<id>/execute`
- `--project-ref <PROJECT_REF>`: project ref được forward qua header `x-project-ref`
- `--auth-token <TOKEN>`: Bearer token cho HTTP worker route có auth
- `--user-id <USER_ID>`: actor user id cho smoke backend lane
- `--user-role <ROLE>`: actor role cho smoke backend lane

### Config file user-level

CLI hỗ trợ config file tại `~/.config/picus/config.toml`.

Xem giá trị đã lưu và giá trị effective hiện tại:

```bash
picus config show
```

Ghi default:

```bash
picus config set \
	--default-url https://console.picusbase.com/api \
	--default-executor-url https://console.picusbase.com \
	--default-project-ref p_8308d170
```

Thứ tự ưu tiên resolve hiện tại là:

- cờ command line
- biến môi trường `PICUS_*`
- config file user-level
- default local hardcoded

### Lưu ý về tên command

- Command canonical là `function`, `job`, `schedule`
- Alias plural `functions`, `jobs`, `schedules` vẫn hoạt động để giữ backward compatibility

## Workflow phổ biến

### 1. Scaffold một function mới

```bash
picus init functions inventory inventory-suppliers --name "Inventory Suppliers"
```

CLI scaffold hiện có ba surface:

- `--language rust` hoặc bỏ cờ: tạo `picus.toml`, `Cargo.toml`, `src/lib.rs`, `openapi.yaml`
- `--language javascript|js`: tạo `picus.toml`, `src/index.mjs`, `openapi.yaml`
- `--language python|py`: tạo `picus.toml`, `src/handler.py`, `pyproject.toml`, `openapi.yaml`

Ví dụ:

```bash
cd /Users/tiennguyen/Documents/my-project/cybutek/project/picus/picus-erp/functions/inventory

picus init functions inventory inventory-products-list-js \
	--language javascript \
	--name "Inventory Products List JS"

picus init functions inventory inventory-products-report-py \
	--language python \
	--name "Inventory Products Report Python"
```

`picus init functions` tạo function
mới dưới current working directory theo mặc định. Flow chuẩn cho business
function là đứng trong parent dir đích như `picus-erp/functions/<domain>/`, còn
nếu chạy từ đúng `picus-erp/functions/` root CLI vẫn giữ layout
`<domain>/<function-id>/`. Ngoài cây ERP này, CLI không tự cập nhật
`picus-erp/functions/Cargo.toml`.

Build và deploy đi qua cùng lớp adapter: `picus function build` và
`picus deploy functions` sẽ tự phát hiện lane từ layout của thư mục function.

Lane JavaScript hiện dùng `src/index.mjs` làm authoring source root; CLI sẽ
normalize source này sang contract bundle active của `javascript-v1`
(`payload/index.mjs` trong artifact) khi chạy `picus function build` hoặc
trực tiếp qua `picus bundle js`.

### 2. Build function artifact

Từ thư mục function:

```bash
picus function build --release
```

Hoặc dùng Cargo trực tiếp:

```bash
cargo build --target wasm32-unknown-unknown --release
```

Kết quả build phụ thuộc lane:

```text
Nếu function nằm dưới một parent tên `functions/`: <that-functions-root>/target/...
Nếu không: <function-dir>/target/...

Ví dụ thường gặp:
Rust dưới picus-erp/functions/<domain>/<id>: picus-erp/functions/target/wasm32-unknown-unknown/release/<crate_name>.wasm
```

### 3. Deploy function

Từ thư mục function:

```bash
picus deploy functions
```

Deploy nhiều function từ một thư mục cha:

```bash
picus deploy functions inventory-warehouses inventory-suppliers
```

Upload thủ công artifact đã build hoặc package sẵn:

```bash
picus function upload \
	--id hello-world \
	--file ./target/wasm32-unknown-unknown/release/hello_world.wasm \
	--name "Hello World"
```

`--file` chấp nhận artifact đúng theo lane, ví dụ `.wasm` cho Rust và `.tar.gz`
cho bundle JavaScript hoặc Python.

### 4. Chạy nhanh một function

```bash
picus \
	--executor-url http://localhost:4396 \
	--project-ref p_8308d170 \
	--user-id smoke-test-operator \
	--user-role authenticated \
	function run --id inventory-products-list-js --params '{"page":1,"limit":5}'
```

Nếu `--executor-url` được set, CLI sẽ gọi HTTP worker route `POST /functions/<id>/execute` với body params JSON raw. Nếu không có `executor_url`, CLI sẽ fallback về control-plane convenience endpoint cũ `POST /compute/functions/<id>/run` để giữ backward compatibility cho lane cũ.

Ví dụ trên dùng sample business-parity JavaScript `inventory-products-list-js` hiện có trong repo và bắn vào worker mixed local `http://localhost:4396`.

Quy ước nhanh:

- Dùng `http://localhost:4396` cho worker mixed local phục vụ cả Wasm `wasm-core-v1`, JavaScript và Python.
- `function run` là smoke/debug surface; app consumer thực tế vẫn nên dùng `picus_sdk::functions::FunctionClient`.

Raw HTTP tương đương cho cùng sample:

```bash
curl -sS -X POST http://localhost:4396/functions/inventory-products-list-js/execute \
	-H 'content-type: application/json' \
	-H 'x-project-ref: p_8308d170' \
	-H 'x-picus-user-id: smoke-test-operator' \
	-H 'x-picus-user-role: authenticated' \
	-d '{"page":1,"limit":5}'
```

### 5. Quản lý jobs

Liệt kê jobs:

```bash
picus job list
```

Tạo background job:

```bash
picus job create --function-id hello-world --params '{"action":"process_data"}'
```

Xóa job:

```bash
picus job delete --id <JOB_ID>
```

### 6. Quản lý schedules

Liệt kê schedules:

```bash
picus schedule list
```

Tạo schedule:

```bash
picus schedule create \
	--name "Sync Data 5m" \
	--cron "0 */5 * * * *" \
	--function-id sync-service
```

Xóa schedule:

```bash
picus schedule delete --id <SCHEDULE_ID>
```

## Examples đi kèm

Thư mục `examples/` chứa các sample function đã được cập nhật sang `HttpHandler`:

- `hello-world/`: đọc body JSON và trả về `HttpResponse` đơn giản
- `calculator/`: parse body input, validate và trả kết quả phép tính
- `count_error_logs/`: ví dụ guest function dùng `host::query_read_only()`

Các sample này có mục đích minh họa guest ABI v2. Chúng không thay thế pattern app-side function calling qua `picus_sdk::functions`.
