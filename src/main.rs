use anyhow::Result;
use router::ws::websocket_handler;
use tracing::info;

mod config;
mod error;
mod router;
mod state;
mod utils;

use axum::routing::{get, post};
use axum::Router;
use state::{load_fedimint_client, AppState};

use router::handlers::*;
// use tower_http::cors::{Any, CorsLayer};
use tower_http::validate_request::ValidateRequestHeaderLayer;

use crate::config::CONFIG;

#[derive(Parser)]
#[clap(version = "1.0", author = "Your Name")]
struct Cli {
    /// Federation invite code
    #[clap(long, env = "FEDERATION_INVITE_CODE")]
    federation_invite_code: String,

    /// Secret key
    #[clap(long, env = "SECRET_KEY")]
    secret_key: String,

    /// Path to FM database
    #[clap(long, env = "FM_DB_PATH")]
    fm_db_path: String,

    /// Password
    #[clap(long, env = "PASSWORD")]
    password: String,

    /// Domain
    #[clap(long, env = "DOMAIN", default_value = "localhost")]
    domain: String,

    /// Port
    #[clap(long, env = "PORT", default_value_t = 5000)]
    port: u16,

    /// Mode of operation
    #[clap(long, arg_enum)]
    mode: Mode,
}

#[derive(ArgEnum, Clone, Debug)]
enum Mode {
    Fedimint,
    Cashu,
    Ws,
}

#[derive(Subcommand)]
enum Commands {
    Start,
    Stop,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli: Cli = Cli::parse();

    let state = AppState {
        fm: load_fedimint_client(cli.federation_invite_code, cli.fm_db_path, cli.secret_key)
            .await?,
    };

    let app = match cli.mode {
        Mode::Fedimint => {
            Router::new()
                .route("/", get(handle_readme))
                .nest("/fedimint/v2", fedimint_v2_rest())
                .with_state(state)
                // .layer(cors)
                .layer(ValidateRequestHeaderLayer::bearer(&cli.password));
        }
        Mode::Cashu => {
            Router::new()
                .route("/", get(handle_readme))
                .nest("/cashu/v1", cashu_v1_rest())
                .with_state(state)
                // .layer(cors)
                .layer(ValidateRequestHeaderLayer::bearer(&cli.password));
        }
        Mode::Ws => {
            Router::new()
                .route("/fedimint/v2/ws", get(websocket_handler))
                .with_state(state)
                // .layer(cors)
                .layer(ValidateRequestHeaderLayer::bearer(&cli.password));
        }
        _ => {
            create_default_router(state, &cli.password).await?;
        }
    };

    let listener = tokio::net::TcpListener::bind(format!("{}:{}", CONFIG.host, CONFIG.port))
        .await
        .unwrap();
    info!("fedimint-http Listening on {}", CONFIG.port);
    axum::serve(listener, app).await.unwrap();

    Ok(())
}

pub async fn create_default_router(state: AppState, password: &str) -> Result<Router> {
    // TODO: Allow CORS? Probably not, since this should just interact with the local machine.
    // let cors = CorsLayer::new()
    //     .allow_methods([Method::GET, Method::POST])
    //     .allow_origin(Any);

    let app = Router::new()
        .route("/", get(handle_readme))
        .route("/fedimint/v2/ws", get(websocket_handler))
        .nest("/fedimint/v2", fedimint_v2_rest())
        .nest("/cashu/v1", cashu_v1_rest())
        .with_state(state)
        // .layer(cors)
        .layer(ValidateRequestHeaderLayer::bearer(password));

    Ok(app)
}

/// Implements Fedimint V0.2 API Route matching against CLI commands:
/// - `/fedimint/v2/admin/info`: Display wallet info (holdings, tiers).
/// - `/fedimint/v2/admin/backup`: Upload the (encrypted) snapshot of mint notes to federation.
/// - `/fedimint/v2/admin/discover-version`: Discover the common api version to use to communicate with the federation.
/// - `/fedimint/v2/admin/restore`: Restore the previously created backup of mint notes (with `backup` command).
/// - `/fedimint/v2/admin/list-operations`: List operations.
/// - `/fedimint/v2/admin/module`: Call a module subcommand.
/// - `/fedimint/v2/admin/config`: Returns the client config.
///
/// Mint related commands:
/// - `/fedimint/v2/mint/reissue`: Reissue notes received from a third party to avoid double spends.
/// - `/fedimint/v2/mint/spend`: Prepare notes to send to a third party as a payment.
/// - `/fedimint/v2/mint/validate`: Verifies the signatures of e-cash notes, but *not* if they have been spent already.
/// - `/fedimint/v2/mint/split`: Splits a string containing multiple e-cash notes (e.g. from the `spend` command) into ones that contain exactly one.
/// - `/fedimint/v2/mint/combine`: Combines two or more serialized e-cash notes strings.
///
/// Lightning network related commands:
/// - `/fedimint/v2/ln/invoice`: Create a lightning invoice to receive payment via gateway.
/// - `/fedimint/v2/ln/await-invoice`: Wait for incoming invoice to be paid.
/// - `/fedimint/v2/ln/pay`: Pay a lightning invoice or lnurl via a gateway.
/// - `/fedimint/v2/ln/await-pay`: Wait for a lightning payment to complete.
/// - `/fedimint/v2/ln/list-gateways`: List registered gateways.
/// - `/fedimint/v2/ln/switch-gateway`: Switch active gateway.
///
/// Onchain related commands:
/// - `/fedimint/v2/onchain/deposit-address`: Generate a new deposit address, funds sent to it can later be claimed.
/// - `/fedimint/v2/onchain/await-deposit`: Wait for deposit on previously generated address.
/// - `/fedimint/v2/onchain/withdraw`: Withdraw funds from the federation.
fn fedimint_v2_rest() -> Router<AppState> {
    let mint_router = Router::new()
        .route("/reissue", post(fedimint::mint::reissue::handle_rest))
        .route("/spend", post(fedimint::mint::spend::handle_rest))
        .route("/validate", post(fedimint::mint::validate::handle_rest))
        .route("/split", post(fedimint::mint::split::handle_rest))
        .route("/combine", post(fedimint::mint::combine::handle_rest));

    let ln_router = Router::new()
        .route("/invoice", post(fedimint::ln::invoice::handle_rest))
        .route(
            "/await-invoice",
            post(fedimint::ln::await_invoice::handle_rest),
        )
        .route("/pay", post(fedimint::ln::pay::handle_rest))
        .route("/await-pay", post(fedimint::ln::await_pay::handle_rest))
        .route(
            "/list-gateways",
            get(fedimint::ln::list_gateways::handle_rest),
        )
        .route(
            "/switch-gateway",
            post(fedimint::ln::switch_gateway::handle_rest),
        );

    let wallet_router = Router::new()
        .route(
            "/deposit-address",
            post(fedimint::wallet::deposit_address::handle_rest),
        )
        .route(
            "/await-deposit",
            post(fedimint::wallet::await_deposit::handle_rest),
        )
        .route("/withdraw", post(fedimint::wallet::withdraw::handle_rest));

    let admin_router = Router::new()
        .route("/info", get(fedimint::admin::info::handle_rest))
        .route("/backup", post(fedimint::admin::backup::handle_rest))
        .route(
            "/discover-version",
            get(fedimint::admin::discover_version::handle_rest),
        )
        .route("/restore", post(fedimint::admin::restore::handle_rest))
        // .route("/printsecret", get(fedimint::handle_printsecret)) TODO: should I expose this under admin?
        .route(
            "/list-operations",
            post(fedimint::admin::list_operations::handle_rest),
        )
        .route("/module", post(fedimint::admin::module::handle_rest))
        .route("/config", get(fedimint::admin::config::handle_rest));

    let base_router = Router::new()
        .nest("/admin", admin_router)
        .nest("/mint", mint_router)
        .nest("/ln", ln_router)
        .nest("/wallet", wallet_router);

    base_router
}

/// Implements Cashu V1 API Routes:
///
/// REQUIRED
/// NUT-01 Mint Public Key Exchange && NUT-02 Keysets and Keyset IDs
/// - `/cashu/v1/keys`
/// - `/cashu/v1/keys/{keyset_id}`
/// - `/cashu/v1/keysets`
/// NUT-03 Swap Tokens (Equivalent to `reissue` command)
/// - `/cashu/v1/swap`
/// NUT-04 Mint Tokens: supports `bolt11` and `onchain` methods
/// - `/cashu/v1/mint/quote/{method}`
/// - `/cashu/v1/mint/quote/{method}/{quote_id}`
/// - `/cashu/v1/mint/{method}`
/// NUT-05 Melting Tokens: supports `bolt11` and `onchain` methods
/// - `/cashu/v1/melt/quote/{method}`
/// - `/cashu/v1/melt/quote/{method}/{quote_id}`
/// - `/cashu/v1/melt/{method}`
/// NUT-06 Mint Information
/// - `/cashu/v1/info`
///
/// OPTIONAL
/// NUT-07 Token State Check
/// - `/cashu/v1/check`
/// NUT-08 Lightning Fee Return
/// - Modification of NUT-05 Melt
/// NUT-10 Spending Conditions
/// NUT-11 Pay to Public Key (P2PK)
/// - Fedimint already does this
/// NUT-12 Offline Ecash Signature Validation
/// - DLEQ in BlindedSignature for Mint to User
fn cashu_v1_rest() -> Router<AppState> {
    let cashu_router = Router::new()
        .route("/keys", get(cashu::keys::handle_keys))
        .route("/keys/:keyset_id", get(cashu::keys::handle_keys_keyset_id))
        .route("/keysets", get(cashu::keysets::handle_keysets))
        .route("/swap", post(cashu::swap::handle_swap))
        .route(
            "/mint/quote/:method",
            get(cashu::mint::quote::handle_method),
        )
        .route(
            "/mint/quote/:method/:quote_id",
            get(cashu::mint::quote::handle_method_quote_id),
        )
        .route("/mint/:method", post(cashu::mint::method::handle_method))
        .route(
            "/melt/quote/:method",
            get(cashu::melt::quote::handle_method),
        )
        .route(
            "/melt/quote/:method/:quote_id",
            get(cashu::melt::quote::handle_method_quote_id),
        )
        .route("/melt/:method", post(cashu::melt::method::handle_method))
        .route("/info", get(cashu::info::handle_info))
        .route("/check", post(cashu::check::handle_check));
    cashu_router
}
