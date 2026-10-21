use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::ServiceForm;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorefrontOptions {
    pub os_images: Vec<OsImage>,
    pub ssh_keys: Vec<SshKey>,
    pub service_forms: Vec<ServiceForm>,
}

#[derive(Serialize)]
pub struct OsImage {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct SshKey {
    pub id: String,
    pub name: String,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(get_options))
}

async fn get_options(
    _auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<StorefrontOptions>, ApiError> {
    let os_images = vec![
        OsImage { id: "ubuntu-22.04".into(), name: "Ubuntu 22.04".into() },
        OsImage { id: "ubuntu-24.04".into(), name: "Ubuntu 24.04".into() },
        OsImage { id: "debian-11".into(), name: "Debian 11".into() },
        OsImage { id: "debian-12".into(), name: "Debian 12".into() },
        OsImage { id: "centos-stream-9".into(), name: "CentOS Stream 9".into() },
        OsImage { id: "almalinux-9".into(), name: "AlmaLinux 9".into() },
        OsImage { id: "rockylinux-9".into(), name: "Rocky Linux 9".into() },
    ];

    let service_forms: Vec<ServiceForm> = sqlx::query_as("SELECT * FROM service_forms ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    Ok(Json(StorefrontOptions {
        os_images,
        ssh_keys: vec![],
        service_forms,
    }))
}
