use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    extract::{Path, State},
    routing::get,
    Router,
};
use deno_core::{
    error::AnyError, op, serde_json, serde_v8, v8, JsRuntime, OpState, RuntimeOptions,
};
use rusqlite::Connection;
use tokio::task::spawn_blocking;
use tracing::info;
use tracing_subscriber::{
    filter::{EnvFilter, LevelFilter},
    prelude::*,
};

async fn handle_root() -> &'static str {
    "Hello rustau!"
}

async fn handle_fn_submit(State(state): State<AppState>, Path(name): Path<String>, body: String) {
    let db_file = format!("{name}.db");
    let db = Connection::open(&db_file).unwrap();
    db.execute("create table if not exists kv (key unique, value)", [])
        .unwrap();
    let mut state = state.lock().unwrap();
    state.insert(name.clone(), (body, Arc::new(Mutex::new(db))));
    info!("added new function: {name}");
}

#[op]
fn op_log(state: &mut OpState, msg: String) {
    info!("[{}]: {}", state.borrow::<String>(), msg);
}

#[op]
fn op_kv_set(state: &mut OpState, key: String, value: String) -> Result<(), AnyError> {
    state
        .borrow_mut::<DB>()
        .lock()
        .unwrap()
        .execute("replace into kv (key, value) values (?1, ?2)", [key, value])?;
    Ok(())
}

#[op]
fn op_kv_get(state: &mut OpState, key: String) -> Result<Option<String>, AnyError> {
    let db = state.borrow_mut::<DB>().lock().unwrap();
    let result = db
        .prepare("select value from kv where key = ?1")?
        .query_map([key], |row| row.get(0))?
        .next();
    match result {
        Some(value) => Ok(value?),
        None => Ok(None),
    }
}

const RUNTIME_BOOTSTRAP: &str = r#"
globalThis.console = {
    log: (...args) => Deno.core.opSync("op_log", args.join(", "))
}
globalThis.set = (key, value) => Deno.core.opSync("op_kv_set", key, JSON.stringify(value))
globalThis.get = (key) => JSON.parse(Deno.core.opSync("op_kv_get", key))
"#;

fn run_js(name: &str, body: &str, db: DB) -> Result<String, AnyError> {
    let mut runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![deno_core::Extension::builder()
            .ops(vec![op_log::decl(), op_kv_set::decl(), op_kv_get::decl()])
            .js(vec![("[runtime]", RUNTIME_BOOTSTRAP)])
            .build()],
        ..Default::default()
    });
    let state = runtime.op_state();
    state.borrow_mut().put::<String>(name.to_owned());
    state.borrow_mut().put(db);
    let global = runtime.execute_script(name, body)?;

    let scope = &mut runtime.handle_scope();

    let local = v8::Local::new(scope, global);

    let deserialized_value = serde_v8::from_v8::<serde_json::Value>(scope, local)?;

    info!("result from \"{name}\": {:#?}", deserialized_value);

    Ok(deserialized_value.to_string())
}

type DB = Arc<Mutex<Connection>>;
type AppState = Arc<Mutex<HashMap<String, (String, DB)>>>;

async fn handle_fn_execute(State(state): State<AppState>, Path(name): Path<String>) -> String {
    let (fn_body, _db) = state.lock().unwrap().get(&name).cloned().unwrap();

    info!("invoking stored fn: {}", &name);

    let fn_body = fn_body.clone();

    spawn_blocking(move || run_js(&name, &fn_body.clone(), _db).unwrap())
        .await
        .unwrap()
}

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

fn register_trace_stdout_listener() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer().with_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::INFO.into())
                    .from_env_lossy(),
            ),
        )
        .init();
}
