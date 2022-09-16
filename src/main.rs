use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    async_trait,
    extract::{FromRequestParts, Path, State},
    http::{request::Parts, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use deno_core::{
    error::{AnyError, JsError},
    op, serde_json, serde_v8, v8, JsRuntime, OpState, RuntimeOptions,
};
use rusqlite::{Connection, OptionalExtension};
use tracing::{error, info};
use tracing_subscriber::prelude::*;

async fn handle_root() -> &'static str {
    "Hello rustau!"
}

// HTTP POST /fn/:name        curl -d @fn.js localhost:8080/fn/hello
async fn handle_fn_submit(
    State(state): State<AppState>,
    FunctionName(name): FunctionName,
    body: String,
) -> Result<(), AppError> {
    let db_file = format!("{name}.db");
    let db = Connection::open(&db_file)?;

    db.execute("create table if not exists kv (key unique, value)", [])?;

    state
        .lock()?
        .insert(name.clone(), (body, Arc::new(Mutex::new(db))));

    info!("added new function: {name}");

    Ok(())
}

// HTTP GET /fn/:name        curl localhost:8080/fn/hello
async fn handle_fn_execute(
    State(state): State<AppState>,
    FunctionName(name): FunctionName,
) -> Result<String, AppError> {
    let (fn_body, db) = state
        .lock()?
        .get(&name)
        .cloned()
        .ok_or_else(|| AppError::UnknownFunction(name.clone()))?;

    info!("invoking stored fn: {}", &name);

    run_js(&name, &fn_body, db)
}

#[op]
fn op_log(state: &mut OpState, msg: String) {
    // emit the log message prefixed with the name of the function
    info!("[{}]: {}", state.borrow::<String>(), msg);
}

#[op]
fn op_kv_set(state: &mut OpState, key: String, value: String) -> Result<(), AnyError> {
    state
        .borrow_mut::<DB>()
        .lock()
        // the error from a poisoned lock can't be sent between threads
        // so we take it's msg contents and wrap them in an error that is Send
        .map_err(|err| AnyError::msg(err.to_string()))?
        .execute("replace into kv (key, value) values (?1, ?2)", [key, value])?;

    Ok(())
}

#[op]
fn op_kv_get(state: &mut OpState, key: String) -> Result<Option<String>, AnyError> {
    let db = state
        .borrow_mut::<DB>()
        .lock()
        // the error from a poisoned lock can't be sent between threads
        // so we take it's msg contents and wrap them in an error that is Send
        .map_err(|err| AnyError::msg(err.to_string()))?;

    let result = db
        .prepare("select value from kv where key = ?1")?
        .query_row([key], |row| row.get(0))
        .optional()?;

    Ok(result)
}

const RUNTIME_BOOTSTRAP: &str = r#"
globalThis.console = {
    log: (...args) => Deno.core.opSync("op_log", args.join(", "))
}
globalThis.set = (key, value) => (Deno.core.opSync("op_kv_set", key, JSON.stringify(value)), value)
globalThis.get = (key) => JSON.parse(Deno.core.opSync("op_kv_get", key))
"#;

fn run_js(name: &str, body: &str, db: DB) -> Result<String, AppError> {
    let mut runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![deno_core::Extension::builder()
            .ops(vec![op_log::decl(), op_kv_set::decl(), op_kv_get::decl()])
            .js(vec![("[runtime]", RUNTIME_BOOTSTRAP)])
            .build()],
        ..Default::default()
    });

    let state = runtime.op_state();

    // inject the name of the function and access to the DB so ops have access
    state.borrow_mut().put::<String>(name.to_owned());
    state.borrow_mut().put(db);

    let last_value = runtime.execute_script(name, body)?;

    // parse out the last evaluated expression from the function execution
    let scope = &mut runtime.handle_scope();
    let local = v8::Local::new(scope, last_value);
    let deserialized_value = serde_v8::from_v8::<serde_json::Value>(scope, local)?;

    info!("result from \"{name}\": {:#?}", deserialized_value);

    Ok(deserialized_value.to_string())
}

/// Threadsafe lock around a sqlite database connection
type DB = Arc<Mutex<Connection>>;
/// Threadsafe lock around a map of function name -> body & db connection
type AppState = Arc<Mutex<HashMap<String, (String, DB)>>>;

#[tokio::main]
async fn main() {
    register_trace_stdout_listener();

    let state: AppState = Default::default();

    let app = Router::with_state(state)
        .route("/", get(handle_root))
        .route("/fn/:name", get(handle_fn_execute).post(handle_fn_submit));

    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, 8080));
    info!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

/// Register logging provider and emit to stdout anything matching INFO or above
fn register_trace_stdout_listener() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .event_format(
                    tracing_subscriber::fmt::format()
                        .with_timer(tracing_subscriber::fmt::time::UtcTime::new(
                            time::format_description::parse("[hour]:[minute]:[second]").unwrap(),
                        ))
                        .compact(),
                )
                .with_filter(
                    tracing_subscriber::EnvFilter::builder()
                        .with_default_directive(tracing::metadata::LevelFilter::INFO.into())
                        .from_env_lossy(),
                ),
        )
        .init();
}

/// Type for all errors that can bubble up to the http level
///
/// Implements From for various error types, and IntoResponse to build an HTTP response
#[derive(Debug)]
enum AppError {
    SqliteError(String),
    LockPoisoned(String),
    UnknownFunction(String),
    JsError(JsError),
    DenoError(String),
    V8SerialisationError(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        match self {
            AppError::JsError(js_error) => {
                format!("error evaluating function: {js_error}").into_response()
            }
            AppError::UnknownFunction(e) => {
                (StatusCode::BAD_REQUEST, format!("unknown function: {e}")).into_response()
            }
            err => {
                error!("internal error: {err:?}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
            }
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        AppError::SqliteError(err.to_string())
    }
}

impl<T> From<std::sync::PoisonError<T>> for AppError {
    fn from(e: std::sync::PoisonError<T>) -> Self {
        AppError::LockPoisoned(e.to_string())
    }
}

impl From<deno_core::anyhow::Error> for AppError {
    fn from(err: deno_core::anyhow::Error) -> Self {
        match err.downcast::<JsError>() {
            Ok(js_error) => AppError::JsError(js_error),
            Err(err) => AppError::DenoError(err.to_string()),
        }
    }
}

impl From<serde_v8::Error> for AppError {
    fn from(err: serde_v8::Error) -> Self {
        AppError::V8SerialisationError(err.to_string())
    }
}

/// Extractor that also validates a function name from the URL
struct FunctionName(String);

#[async_trait]
impl<S> FromRequestParts<S> for FunctionName
where
    S: Send + Sync,
{
    type Rejection = axum::response::Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(name) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;

        if name.chars().any(|c| !c.is_ascii_alphabetic()) {
            let error_msg = format!(
                "invalid function name: \"{name}\", only a-z and A-Z characters are allowed"
            );

            return Err((StatusCode::BAD_REQUEST, error_msg).into_response());
        }

        Ok(FunctionName(name))
    }
}
