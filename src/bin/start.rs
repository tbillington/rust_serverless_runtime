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
    error::AnyError,
    op,
    v8::{Global, Value},
    Extension, JsRuntime, OpState, RuntimeOptions,
};
use rusqlite::Connection;
use tracing::info;
use tracing_subscriber::{
    filter::{EnvFilter, LevelFilter},
    prelude::*,
};

/*
   HTTP Handlers
*/

async fn handle_root() -> &'static str {
    "Hello RustAU!"
}

async fn handle_fn_execute(State(state): State<AppState>, Path(name): Path<String>) -> String {
    info!("invoking stored function: \"{name}\"");

    // unlock state
    // get body, db

    // run js

    unimplemented!();
}

async fn handle_fn_submit(State(state): State<AppState>, Path(name): Path<String>, body: String) {
    info!("adding new function: \"{name}\"");

    // format db file
    // open connection to db
    // execute "create table if not exists kv (key unique, value)", []

    // lock state
    // insert fn into state

    unimplemented!();
}

/*
    Deno Ops
*/

#[op]
fn op_log() {}

#[op]
fn op_kv_set(state: &mut OpState, key: String, value: String) -> Result<(), AnyError> {
    // borrow & lock state
    // execute replace into kv (key, value) values (?1, ?2)", [key, value]

    unimplemented!();
}

#[op]
fn op_kv_get(state: &mut OpState, key: String) -> Result<Option<String>, AnyError> {
    // borrow and lock state
    // prepare "select value from kv where key = ?1"
    // query_map [key], |row| row.get(0)
    // next
    // unpack the optional

    unimplemented!();
}

/*
    Runtime
*/
const RUNTIME: &'static str = r#"

"#;
fn run_js(name: &str, body: &str, db: DB) -> Result<String, AnyError> {
    // build runtime
    // JsRuntime::new(RuntimeOptions { extensions: vec![Extension::builder().ops(vec![]).js(vec![])],..Default::default() })

    // get op_state, push db into state

    // execute script

    // return last value with string_from_v8_value

    unimplemented!();
}

type DB = (); //Arc<Mutex<Connection>>;
type AppState = Arc<Mutex<HashMap<String, (String, DB)>>>;

#[tokio::main]
async fn main() {
    register_trace_stdout_listener();

    let state: AppState = Default::default();

    let router = Router::with_state(state)
        .route("/", get(handle_root))
        .route("/fn/:name", get(handle_fn_execute).post(handle_fn_submit));

    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, 8080));

    info!("listening on {}", addr);

    axum::Server::bind(&addr)
        .serve(router.into_make_service())
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

fn string_from_v8_value(runtime: &mut JsRuntime, value: Global<Value>) -> Result<String, AnyError> {
    let scope = &mut runtime.handle_scope();
    let local = deno_core::v8::Local::new(scope, value);
    let value = deno_core::serde_v8::from_v8::<deno_core::serde_json::Value>(scope, local).unwrap();
    Ok(value.to_string())
}
