use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;

use camino::Utf8PathBuf;
use clap::Parser;

use bifrost::auth;
use bifrost::config::validate;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::routes::bifrost::setup::{self, SetupOutcome};
use bifrost::server;
use bifrost::server::appstate::AppState;
use bifrost::service_registry::{self, AppServiceFactory};
use svc::manager::ServiceManager;
use svc::manager::SvmClient;
use tokio::signal;
use tokio::signal::unix::SignalKind;

const DEFAULT_CONFIG_FILE: &str = "config.yaml";
const CONFIG_FILE_ENV: &str = "BIFROST_CONFIG";
const SETUP_LISTEN_ENV: &str = "BIFROST_SETUP_LISTEN";

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Configuration file [default: `$BIFROST_CONFIG`, or `config.yaml`]
    #[arg(long)]
    config: Option<Utf8PathBuf>,

    /// Address setup mode listens on while the configuration file is
    /// missing or unreadable [default: `$BIFROST_SETUP_LISTEN`, or the
    /// default route's interface on port 80]
    #[arg(long)]
    setup_listen: Option<SocketAddr>,
}

impl Args {
    fn config_file(&self) -> Utf8PathBuf {
        self.config
            .clone()
            .or_else(|| std::env::var(CONFIG_FILE_ENV).ok().map(Utf8PathBuf::from))
            .unwrap_or_else(|| Utf8PathBuf::from(DEFAULT_CONFIG_FILE))
    }

    fn setup_listen(&self) -> std::io::Result<SocketAddr> {
        if let Some(addr) = self.setup_listen {
            return Ok(addr);
        }
        std::env::var(SETUP_LISTEN_ENV).map_or_else(
            |_| setup::default_listen(),
            |addr| {
                addr.parse().map_err(|_| {
                    std::io::Error::other(format!("{SETUP_LISTEN_ENV} is not an address"))
                })
            },
        )
    }
}

/*
 * Formatter function to output in syslog format. This makes sense when running
 * as a service (where output might go to a log file, or the system journal)
 */
#[allow(clippy::match_same_arms)]
fn syslog_format(
    buf: &mut pretty_env_logger::env_logger::fmt::Formatter,
    record: &log::Record,
) -> std::io::Result<()> {
    writeln!(
        buf,
        "<{}>{}: {}",
        match record.level() {
            log::Level::Error => 3,
            log::Level::Warn => 4,
            log::Level::Info => 6,
            log::Level::Debug => 7,
            log::Level::Trace => 7,
        },
        record.target(),
        record.args()
    )
}

fn init_logging() -> ApiResult<()> {
    /* Try to provide reasonable default filters, when RUST_LOG is not specified */
    const DEFAULT_LOG_FILTERS: &[&str] = &[
        "debug",
        "mdns_sd=off",
        "tokio_ssdp=info",
        "tower_http::trace::on_request=info",
        "h2=info",
        "axum::rejection=trace",
    ];

    let log_filters = std::env::var("RUST_LOG").unwrap_or_else(|_| DEFAULT_LOG_FILTERS.join(","));

    /* Detect if we need syslog or human-readable formatting */
    if std::env::var("SYSTEMD_EXEC_PID").is_ok_and(|pid| pid == std::process::id().to_string()) {
        Ok(pretty_env_logger::env_logger::builder()
            .format(syslog_format)
            .parse_filters(&log_filters)
            .try_init()?)
    } else {
        Ok(pretty_env_logger::formatted_timed_builder()
            .parse_filters(&log_filters)
            .try_init()?)
    }
}

#[allow(clippy::similar_names)]
async fn build_tasks(appstate: &AppState) -> ApiResult<()> {
    let mut mgr = appstate.manager();

    log::info!("Serving mac [{}]", appstate.config().bridge.mac);

    // register and start every configuration-driven service
    let factory = AppServiceFactory::new(appstate.clone());
    service_registry::reconcile(&appstate.store(), &mut mgr, &factory, &[]).await?;

    // register config writer
    let svc = server::config_writer(appstate.res.clone(), appstate.store().paths().state.clone());
    mgr.register_function("config-writer", svc).await?;

    // register version updater
    let svc = server::version_updater(appstate.res.clone(), appstate.updater());
    mgr.register_function("version-updater", svc).await?;

    // finally, iterate over all services and start them
    for (id, _name) in mgr.list().await? {
        mgr.start(id).await?;
    }

    Ok(())
}

/// Without an admin password, print a fresh setup code. A lost password is
/// reset by deleting `bifrost.auth.password_hash` from the configuration
/// file and restarting, which lands here again.
fn announce_setup(appstate: &AppState) {
    let config = appstate.config();
    match &config.bifrost.auth.password_hash {
        Some(hash) if !auth::is_valid_hash(hash) => log::error!(
            "The admin password hash in the configuration cannot be read, so no login can succeed. \
             To reset it, stop Bifrost, delete bifrost.auth.password_hash, and start Bifrost again."
        ),
        Some(_) => {}
        None => {
            let code = appstate.auth().issue_setup_code();
            // stdout only, never the log: the code is the only control over
            // who sets the first password, and log lines get collected and
            // served elsewhere
            println!(
                "No admin password is set. Open Bifrost in a browser and enter this setup code to choose one: {code}"
            );
            let _ = std::io::stdout().flush();
            log::warn!(
                "No admin password is set; a one-time setup code was printed to standard output"
            );
        }
    }
}

fn install_signal_handlers(appstate: &AppState) -> ApiResult<()> {
    async fn shutdown(msg: &str, mut mgr: SvmClient) {
        log::warn!("{msg}");
        let _ = std::io::stderr().flush();
        let _ = mgr.shutdown().await;
    }

    let mgr = appstate.manager();
    tokio::spawn(async move {
        if matches!(signal::ctrl_c().await, Ok(())) {
            shutdown("Ctrl-C pressed, exiting..", mgr).await;
        }
    });

    let mgr = appstate.manager();
    let mut signal = signal::unix::signal(SignalKind::terminate())?;
    tokio::spawn(async move {
        if matches!(signal.recv().await, Some(())) {
            shutdown("SIGTERM received, exiting..", mgr).await;
        }
    });

    Ok(())
}

async fn run(args: Args) -> ApiResult<()> {
    init_logging()?;

    #[cfg(feature = "server-banner")]
    server::banner::print()?;

    let config_file = args.config_file();
    let store = match ConfigStore::load(&config_file) {
        Ok(store) => store,
        Err(err) => {
            // a missing or broken file is answered with setup mode, which
            // writes a new one and then hands over to the bridge here
            match setup::run(&config_file, err.to_string(), args.setup_listen()?).await? {
                SetupOutcome::Configured => ConfigStore::load(&config_file)?,
                SetupOutcome::Interrupted => return Ok(()),
            }
        }
    };
    log::debug!("Configuration loaded successfully from {config_file}");

    // findings are reported, never enforced: a file that loads keeps loading
    for finding in validate::validate(&store.active()) {
        log::warn!(
            "Configuration {:?} at {:?}: {}",
            finding.severity,
            finding.path,
            finding.message
        );
    }

    let (client, future) = ServiceManager::spawn();

    let appstate = AppState::from_store(Arc::new(store), client).await?;

    install_signal_handlers(&appstate)?;

    announce_setup(&appstate);

    build_tasks(&appstate).await?;

    future.await??;

    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(err) = run(Args::parse()).await {
        log::error!("Bifrost error: {err}");
        log::error!("Fatal error encountered, cannot continue.");
    }
}
