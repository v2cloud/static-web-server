use hyper::server::Server as HyperServer;
use hyper::service::{make_service_fn, service_fn};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use structopt::StructOpt;
use tokio::signal;

use crate::{
    config::{Config, CONFIG},
    error_page,
};
use crate::{controller::handle, fs::ArcPath};
use crate::{error, helpers, logger, Result};

/// Define a multi-thread HTTP or HTTP/2 web server.
pub struct Server {
    threads: usize,
}

impl Server {
    /// Create new multi-thread server instance.
    pub fn new() -> Self {
        // Initialize global config
        CONFIG.set(Config::from_args()).unwrap();
        let opts = Config::global();

        let threads = match opts.threads_multiplier {
            0 | 1 => 1,
            _ => num_cpus::get() * opts.threads_multiplier,
        };

        Self { threads }
    }

    /// Build and run the multi-thread `Server`.
    pub fn run(self) -> Result {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("static-web-server")
            .worker_threads(self.threads)
            .build()?
            .block_on(async {
                let r = self.start_server().await;
                if r.is_err() {
                    panic!("Server error during start up: {:?}", r.unwrap_err())
                }
            });

        Ok(())
    }

    /// Run the inner Hyper `HyperServer` forever on the current thread with the given configuration.
    async fn start_server(self) -> Result {
        let opts = Config::global();

        logger::init(&opts.log_level)?;

        tracing::info!("runtime worker threads {}", self.threads);
        tracing::info!("runtime max blocking threads {}", self.threads);

        let ip = opts.host.parse::<IpAddr>()?;
        let addr = SocketAddr::from((ip, opts.port));

        // Check for a valid root directory
        let root_dir = helpers::get_valid_dirpath(&opts.root)?;

        // Custom error pages content
        error_page::PAGE_404
            .set(helpers::read_file_content(opts.page404.as_ref()))
            .expect("page 404 is not initialized");
        error_page::PAGE_50X
            .set(helpers::read_file_content(opts.page50x.as_ref()))
            .expect("page 50x is not initialized");

        // TODO: CORS support

        // TODO: HTTP/2 + TLS

        // Spawn a new Tokio asynchronous server task determined by the given options

        let span = tracing::info_span!("Server::run", ?addr, threads = ?self.threads);
        tracing::info!(parent: &span, "listening on http://{}", addr);

        let root_dir = ArcPath(Arc::new(root_dir));
        let create_service = make_service_fn(move |_| {
            let root_dir = root_dir.clone();
            async move {
                Ok::<_, error::Error>(service_fn(move |req| {
                    let root_dir = root_dir.clone();
                    async move { handle(root_dir.as_ref(), req).await }
                }))
            }
        });
        HyperServer::bind(&addr)
            .serve(create_service)
            .with_graceful_shutdown(async {
                signal::ctrl_c()
                    .await
                    .expect("failed to install CTRL+C signal handler");
                tracing::warn!(parent: &span, "CTRL+C signal caught and execution exited");
            })
            .await?;

        Ok(())
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}
