//! Realistic user data — project trees a genuine customer would snapshot. NOTHING here
//! signals "test": real-looking filenames, real-looking source, a real project name. The
//! backend that clw talks to sees exactly what a real developer's `clw snapshot` sends.
//!
//! This is the "indistinguishable from a real user" half of the story-driven suite: the
//! *identity* comes from provisioning (signup), the *data* comes from here.

use std::path::{Path, PathBuf};

/// Write a realistic backend-service project tree under `root/<dir>` and return its path.
/// It's a small but plausible Rust microservice (an orders API): a manifest, a few source
/// modules, a test, a Dockerfile, config, a README, a lockfile-sized blob. Enough distinct
/// files to exercise multi-chunk snapshot/dedup, with one module (`pricing.rs`) designed to
/// be edited mid-story so `status`/re-snapshot see a real, localized change.
pub fn orders_api(root: &Path, dir: &str) -> PathBuf {
    let base = root.join(dir);
    let write = |rel: &str, body: &str| {
        let p = base.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).ok();
        std::fs::write(&p, body).expect("write project file");
    };

    write(
        "Cargo.toml",
        "[package]\nname = \"orders-api\"\nversion = \"0.3.1\"\nedition = \"2021\"\n\n\
         [dependencies]\naxum = \"0.7\"\ntokio = { version = \"1\", features = [\"full\"] }\n\
         serde = { version = \"1\", features = [\"derive\"] }\nrust_decimal = \"1\"\n",
    );
    write(
        "README.md",
        "# orders-api\n\nOrder intake + pricing service for the storefront.\n\n\
         ## Run\n```\ncargo run\n```\nListens on `:8080`. See `config/default.toml`.\n\n\
         ## Endpoints\n- `POST /orders` — create an order\n- `GET /orders/:id` — fetch one\n\
         - `GET /healthz` — liveness\n",
    );
    write(".gitignore", "/target\n*.log\n.env\n.DS_Store\n");
    write(
        "Dockerfile",
        "FROM rust:1.83-slim AS build\nWORKDIR /app\nCOPY . .\nRUN cargo build --release\n\n\
         FROM debian:bookworm-slim\nCOPY --from=build /app/target/release/orders-api /usr/local/bin/\n\
         EXPOSE 8080\nENTRYPOINT [\"orders-api\"]\n",
    );
    write(
        "config/default.toml",
        "[server]\nbind = \"0.0.0.0:8080\"\nworkers = 4\n\n\
         [pricing]\ncurrency = \"USD\"\ntax_rate = 0.0875\nfree_shipping_over = 50.00\n",
    );
    write(
        "src/main.rs",
        "mod routes;\nmod pricing;\n\n#[tokio::main]\nasync fn main() {\n    \
         let app = routes::router();\n    let listener = \
         tokio::net::TcpListener::bind(\"0.0.0.0:8080\").await.unwrap();\n    \
         println!(\"orders-api listening on :8080\");\n    \
         axum::serve(listener, app).await.unwrap();\n}\n",
    );
    write(
        "src/routes.rs",
        "use axum::{routing::{get, post}, Router, Json};\nuse crate::pricing;\n\n\
         pub fn router() -> Router {\n    Router::new()\n        \
         .route(\"/healthz\", get(|| async { \"ok\" }))\n        \
         .route(\"/orders\", post(create_order))\n}\n\n\
         async fn create_order(Json(items): Json<Vec<pricing::LineItem>>) -> Json<pricing::Quote> {\n    \
         Json(pricing::quote(&items))\n}\n",
    );
    write(
        "src/pricing.rs",
        // The module Marina edits mid-story. v1: flat tax, no free-shipping rule.
        "use rust_decimal::Decimal;\nuse serde::{Deserialize, Serialize};\n\n\
         #[derive(Deserialize)]\npub struct LineItem {\n    pub sku: String,\n    \
         pub qty: u32,\n    pub unit_price: Decimal,\n}\n\n\
         #[derive(Serialize)]\npub struct Quote {\n    pub subtotal: Decimal,\n    \
         pub tax: Decimal,\n    pub total: Decimal,\n}\n\n\
         const TAX_RATE: &str = \"0.0875\";\n\n\
         pub fn quote(items: &[LineItem]) -> Quote {\n    \
         let subtotal: Decimal = items.iter().map(|i| i.unit_price * Decimal::from(i.qty)).sum();\n    \
         let tax = subtotal * TAX_RATE.parse::<Decimal>().unwrap();\n    \
         Quote { subtotal, tax, total: subtotal + tax }\n}\n",
    );
    write(
        "tests/integration.rs",
        "// Smoke: a two-line order quotes with tax.\n#[test]\nfn quote_has_tax() {\n    \
         // (wired against the running service in CI; unit-level here)\n    assert!(true);\n}\n",
    );
    // A real repo has awkward-but-legitimate shapes clw must round-trip faithfully:
    // a unicode-named design note (en-dash + accents), and an empty migrations dir.
    write(
        "docs/design–café.md",
        "# Pricing design — café\n\nNotes on tax + free-shipping thresholds.\n",
    );
    std::fs::create_dir_all(base.join("migrations")).ok(); // an empty dir must survive round-trip

    // An executable deploy script — its 0755 mode must be preserved through
    // snapshot→hydrate, or a hydrated checkout is broken (script won't run).
    let deploy = base.join("scripts/deploy.sh");
    std::fs::create_dir_all(deploy.parent().unwrap()).ok();
    std::fs::write(
        &deploy,
        "#!/usr/bin/env bash\nset -euo pipefail\ncargo build --release\n\
         docker build -t orders-api:latest .\necho deployed\n",
    )
    .expect("write deploy.sh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&deploy, std::fs::Permissions::from_mode(0o755)).ok();
    }
    base
}

/// The exec bit of `scripts/deploy.sh` under a (possibly hydrated) project root — used to
/// assert 0755 survived the round-trip. `None` off-unix.
#[cfg(unix)]
pub fn deploy_script_is_executable(project_root: &Path) -> Option<bool> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(project_root.join("scripts/deploy.sh"))
        .ok()
        .map(|m| m.permissions().mode() & 0o111 != 0)
}
#[cfg(not(unix))]
pub fn deploy_script_is_executable(_project_root: &Path) -> Option<bool> {
    None
}

/// The edit a developer makes mid-story: add the free-shipping rule to `pricing.rs`. This
/// changes exactly one file (one localized chunk), so `status` reports precisely it and a
/// re-snapshot uploads only the delta — a realistic incremental change, not a synthetic
/// byte flip.
pub fn add_free_shipping_rule(project: &Path) {
    let pricing = project.join("src/pricing.rs");
    let mut body = std::fs::read_to_string(&pricing).expect("read pricing.rs");
    body = body.replace(
        "Quote { subtotal, tax, total: subtotal + tax }",
        "let shipping = if subtotal >= Decimal::from(50) { Decimal::ZERO } \
         else { Decimal::from(5) };\n    \
         Quote { subtotal, tax, total: subtotal + tax + shipping }",
    );
    std::fs::write(&pricing, body).expect("write pricing.rs");
}
