use std::sync::Arc;

use axum::extract::Path;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::Router;
use http::{header, StatusCode};
use rust_embed::Embed;
use tokio::net::TcpListener;
use tower_http::cors::{AllowHeaders, AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::handlers;
use crate::sabnzbd_compat;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(info(title = "rustnzbd API", version = env!("CARGO_PKG_VERSION")))]
struct ApiDoc;

/// Embed the static/ directory at compile time.
#[derive(Embed)]
#[folder = "static/"]
struct StaticAssets;

/// Serve the root page (index.html) from embedded static assets.
async fn h_root() -> Response {
    serve_embedded_file("index.html")
}

/// Serve any file from the embedded static assets by path.
async fn h_static(Path(path): Path<String>) -> Response {
    serve_embedded_file(&path)
}

/// Look up an embedded file and return it with the correct Content-Type.
fn serve_embedded_file(path: &str) -> Response {
    match StaticAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                content.data.into_owned(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Build the axum Router with all API routes.
pub fn build_router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::default()
        .allow_origin(AllowOrigin::any())
        .allow_headers(AllowHeaders::any());

    // Native REST API
    let api_routes = Router::new()
        // Status
        .route("/status", get(handlers::h_status))
        // Logs
        .route("/logs", get(handlers::h_logs))
        // Queue
        .route("/queue", get(handlers::h_queue_list))
        .route("/queue/add", post(handlers::h_queue_add))
        .route("/queue/pause", post(handlers::h_queue_pause_all))
        .route("/queue/resume", post(handlers::h_queue_resume_all))
        .route("/queue/pause-for", post(handlers::h_queue_pause_for))
        .route("/queue/{id}/pause", post(handlers::h_queue_pause))
        .route("/queue/{id}/resume", post(handlers::h_queue_resume))
        .route("/queue/{id}", delete(handlers::h_queue_delete))
        // History
        .route("/history", get(handlers::h_history_list))
        .route("/history/{id}", delete(handlers::h_history_delete))
        .route("/history/{id}/retry", post(handlers::h_history_retry))
        .route("/history/{id}/logs", get(handlers::h_history_logs))
        .route("/history", delete(handlers::h_history_clear))
        // Config
        .route("/config", get(handlers::h_config_get))
        .route("/config/servers", get(handlers::h_servers_list))
        .route("/config/servers", post(handlers::h_server_add))
        .route("/config/servers/{id}", put(handlers::h_server_update))
        .route("/config/servers/{id}", delete(handlers::h_server_delete))
        .route(
            "/config/servers/test-config",
            post(handlers::h_server_test_inline),
        )
        .route(
            "/config/servers/{id}/test",
            post(handlers::h_server_test),
        )
        .route("/config/categories", get(handlers::h_categories_list))
        .route(
            "/config/history-retention",
            get(handlers::h_history_retention_get),
        )
        .route(
            "/config/history-retention",
            put(handlers::h_history_retention_set),
        )
        .route(
            "/config/max-active-downloads",
            get(handlers::h_max_active_downloads_get),
        )
        .route(
            "/config/max-active-downloads",
            put(handlers::h_max_active_downloads_set),
        );

    // Arr-compatible API (Sonarr/Radarr)
    let sabnzbd_route = Router::new()
        .route("/sabnzbd/api", get(sabnzbd_compat::h_sabnzbd_api_get))
        .route("/sabnzbd/api", post(sabnzbd_compat::h_sabnzbd_api_post));

    Router::new()
        .route("/", get(h_root))
        .route("/static/{*path}", get(h_static))
        .nest("/api", api_routes)
        .merge(sabnzbd_route)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
        .merge(
            SwaggerUi::new("/swagger-ui")
                .url("/api-docs/openapi.json", ApiDoc::openapi()),
        )
}

/// Start the HTTP server.
pub async fn run(state: Arc<AppState>) -> anyhow::Result<()> {
    let config = state.config();
    let addr = format!("{}:{}", config.general.listen_addr, config.general.port);

    let router = build_router(state);
    let listener = TcpListener::bind(&addr).await?;

    info!("HTTP server listening on http://{addr}");
    info!("Web GUI: http://{addr}/");
    info!("Arr API: http://{addr}/sabnzbd/api?mode=version");

    axum::serve(listener, router).await?;
    Ok(())
}
