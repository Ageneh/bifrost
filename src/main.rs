use std::io::Write;
use std::sync::Arc;

use camino::Utf8PathBuf;
use clap::Parser;

use bifrost::config::validate;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::server;
use bifrost::server::appstate::AppState;
use bifrost::service_registry::{self, AppServiceFactory};
use svc::manager::ServiceManager;
use svc::manager::SvmClient;
use tokio::signal;
use tokio::signal::unix::SignalKind;

const DEFAULT_CONFIG_FILE: &str = "config.yaml";
const CONFIG_FILE_ENV: &str = "BIFROST_CONFIG";

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Configuration file [default: `$BIFROST_CONFIG`, or `config.yaml`]
    #[arg(long)]
    config: Option<Utf8PathBuf>,

    /// Serve the web UI from this directory instead of the embedded copy
    #[arg(long)]
    ui_dir: Option<Utf8PathBuf>,
}

impl Args {
    fn config_file(self) -> Utf8PathBuf {
        self.config
            .or_else(|| std::env::var(CONFIG_FILE_ENV).ok().map(Utf8PathBuf::from))
            .unwrap_or_else(|| Utf8PathBuf::from(DEFAULT_CONFIG_FILE))
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

async fn run(config_file: Utf8PathBuf) -> ApiResult<()> {
    init_logging()?;

    #[cfg(feature = "server-banner")]
    server::banner::print()?;

    let store = ConfigStore::load(&config_file)?;
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

    build_tasks(&appstate).await?;

    future.await??;

    Ok(())
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if let Some(dir) = args.ui_dir.clone() {
        bifrost::routes::ui::serve_from_directory(dir);
    }
    let config_file = args.config_file();

    if let Err(err) = run(config_file).await {
        log::error!("Bifrost error: {err}");
        log::error!("Fatal error encountered, cannot continue.");
    }
}
