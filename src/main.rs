use axum::{
    Json, Router,
    extract::{ConnectInfo, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use std::{collections::HashMap, net::SocketAddr, str::FromStr};
use tower_http::cors::{Any, CorsLayer};
use http::HeaderValue;

#[derive(Deserialize)]
struct GetParams {
    slug: String,
}

#[derive(Deserialize)]
struct PostBody {
    slug: String,
    target: String,
    reacted: bool,
}

const DEFAULT_EMOJIS: &[&str] = &["👍", "👎", "❤️", "🔥", "👀", "😂"];

fn hash_ip(ip: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(ip.as_bytes());
    hex::encode(&hasher.finalize()[..16])
}

type AppError = (StatusCode, String);

fn db_error<E: std::fmt::Display>(e: E) -> AppError {
    eprintln!("db error: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
}

async fn get_reactions(
    State(pool): State<SqlitePool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<GetParams>,
) -> Result<Json<HashMap<String, (i64, bool)>>, AppError> {
    println!("GET / slug={}", params.slug);

    let uid = hash_ip(&addr.ip().to_string());

    let counts = sqlx::query!(
        "SELECT emoji, count FROM counts WHERE slug = ?",
        params.slug
    )
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;

    let reacted = sqlx::query!(
        "SELECT emoji FROM reactions WHERE slug = ? AND uid = ?",
        params.slug,
        uid
    )
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;

    let reacted_set: std::collections::HashSet<String> =
        reacted.into_iter().map(|r| r.emoji).collect();

    let counts_map: HashMap<String, i64> = counts
        .into_iter()
        .map(|r| (r.emoji, r.count))
        .collect();

    let results = DEFAULT_EMOJIS
        .iter()
        .map(|&emoji| {
            let count = counts_map.get(emoji).copied().unwrap_or(0);
            let has_reacted = reacted_set.contains(emoji);
            (emoji.to_string(), (count, has_reacted))
        })
        .collect();

    Ok(Json(results))
}

async fn post_reaction(
    State(pool): State<SqlitePool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<PostBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    println!(
        "POST / slug={} target={} reacted={}",
        body.slug, body.target, body.reacted
    );

    if !DEFAULT_EMOJIS.contains(&body.target.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "invalid emoji target".to_string()));
    }

    let uid = hash_ip(&addr.ip().to_string());

    let already_reacted = sqlx::query!(
        "SELECT count(*) as cnt FROM reactions WHERE slug = ? AND uid = ? AND emoji = ?",
        body.slug,
        uid,
        body.target
    )
    .fetch_one(&pool)
    .await
    .map_err(db_error)?
    .cnt > 0;

    if body.reacted {
        if already_reacted {
            return Ok(Json(serde_json::json!({"error": "already reacted"})));
        }

        sqlx::query!(
            "INSERT INTO reactions (slug, uid, emoji) VALUES (?, ?, ?)",
            body.slug,
            uid,
            body.target
        )
        .execute(&pool)
        .await
        .map_err(db_error)?;

        sqlx::query!(
            "INSERT INTO counts (slug, emoji, count) VALUES (?, ?, 1)
             ON CONFLICT(slug, emoji) DO UPDATE SET count = count + 1",
            body.slug,
            body.target
        )
        .execute(&pool)
        .await
        .map_err(db_error)?;
    } else {
        if !already_reacted {
            return Ok(Json(serde_json::json!({"error": "not reacted"})));
        }

        sqlx::query!(
            "DELETE FROM reactions WHERE slug = ? AND uid = ? AND emoji = ?",
            body.slug,
            uid,
            body.target
        )
        .execute(&pool)
        .await
        .map_err(db_error)?;

        sqlx::query!(
            "UPDATE counts SET count = count - 1 WHERE slug = ? AND emoji = ?",
            body.slug,
            body.target
        )
        .execute(&pool)
        .await
        .map_err(db_error)?;
    }

    Ok(Json(serde_json::json!({"success": true})))
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://data/reactions.db".to_string());

    let opts = SqliteConnectOptions::from_str(&database_url)
        .expect("invalid DATABASE_URL")
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts)
        .await
        .expect("failed to connect to database");

    sqlx::migrate!().run(&pool).await.expect("failed to run migrations");

    let cors = CorsLayer::new()
        .allow_origin("https://smarniw.com".parse::<HeaderValue>().unwrap())
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(get_reactions))
        .route("/", post(post_reaction))
        .layer(cors)
        .with_state(pool)
        .into_make_service_with_connect_info::<SocketAddr>();

    let port = std::env::var("PORT").unwrap_or_else(|_| "6969".to_string());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind TCP listener");
    println!("Listening on http://{addr}");
    axum::serve(listener, app).await.expect("server error");
}
