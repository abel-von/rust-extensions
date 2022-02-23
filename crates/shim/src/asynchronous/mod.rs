#![cfg(feature = "async")]

use containerd_shim_protos::shim_async::{create_task, Client, Task};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::{env, process};

use crate::asynchronous::monitor::monitor_notify_by_pid;
use crate::asynchronous::publisher::RemotePublisher;
use crate::asynchronous::utils::{asyncify, read_file_to_str, write_str_to_file};
use crate::error::Error;
use crate::error::Result;
use crate::{
    args, logger, parse_sockaddr, reap, setup_signals, socket_address, Config, StartOpts,
    SOCKET_FD, TTRPC_ADDRESS,
};
use async_trait::async_trait;
use command_fds::{CommandFdExt, FdMapping};
use containerd_shim_protos::api::DeleteResponse;
use containerd_shim_protos::protobuf::Message;
use containerd_shim_protos::ttrpc::r#async::Server;
use futures::StreamExt;
use libc::{c_int, pid_t, SIGCHLD, SIGINT, SIGPIPE, SIGTERM};
use log::{debug, error, info, warn};
use signal_hook_tokio::Signals;
use tokio::io::AsyncWriteExt;

mod monitor;
mod publisher;
mod utils;

/// Asynchronous Main shim interface that must be implemented by all async shims.
///
/// Start and delete routines will be called to handle containerd's shim lifecycle requests.
#[async_trait]
pub trait Shim {
    /// Type to provide task service for the shim.
    type T: Task + Send + Sync;

    /// Create a new instance of  async Shim.
    ///
    /// # Arguments
    /// - `runtime_id`: identifier of the container runtime.
    /// - `id`: identifier of the shim/container, passed in from Containerd.
    /// - `namespace`: namespace of the shim/container, passed in from Containerd.
    /// - `publisher`: publisher to send events to Containerd.
    /// - `config`: for the shim to pass back configuration information
    async fn new(
        runtime_id: &str,
        id: &str,
        namespace: &str,
        publisher: RemotePublisher,
        config: &mut Config,
    ) -> Self;

    /// Start shim will be called by containerd when launching new shim instance.
    ///
    /// It expected to return TTRPC address containerd daemon can use to communicate with
    /// the given shim instance.
    /// See https://github.com/containerd/containerd/tree/master/runtime/v2#start
    /// this is an asynchronous call
    async fn start_shim(&mut self, opts: StartOpts) -> Result<String>;

    /// Delete shim will be called by containerd after shim shutdown to cleanup any leftovers.
    /// this is an asynchronous call
    async fn delete_shim(&mut self) -> Result<DeleteResponse>;

    /// Wait for the shim to exit asynchronously.
    async fn wait(&mut self);

    /// Get the task service object asynchronously.
    async fn get_task_service(&self) -> Self::T;
}

/// Async Shim entry point that must be invoked from tokio `main`.
pub async fn run<T>(runtime_id: &str, opts: Option<Config>)
where
    T: Shim + Send + Sync + 'static,
{
    if let Some(err) = bootstrap::<T>(runtime_id, opts).await.err() {
        eprintln!("{}: {:?}", runtime_id, err);
        process::exit(1);
    }
}

async fn bootstrap<T>(runtime_id: &str, opts: Option<Config>) -> Result<()>
where
    T: Shim + Send + Sync + 'static,
{
    // Parse command line
    let os_args: Vec<_> = env::args_os().collect();
    let flags = args::parse(&os_args[1..])?;

    let ttrpc_address = env::var(TTRPC_ADDRESS)?;
    let publisher = publisher::RemotePublisher::new(&ttrpc_address).await?;

    // Create shim instance
    let mut config = opts.unwrap_or_else(Config::default);

    // Setup signals
    let signals = setup_signals_tokio(&config);

    if !config.no_sub_reaper {
        reap::set_subreaper().map_err(io_error!(e, "set subreaper"))?;
    }

    let mut shim = T::new(
        runtime_id,
        &flags.id,
        &flags.namespace,
        publisher,
        &mut config,
    )
    .await;

    match flags.action.as_str() {
        "start" => {
            let args = StartOpts {
                id: flags.id,
                publish_binary: flags.publish_binary,
                address: flags.address,
                ttrpc_address,
                namespace: flags.namespace,
                debug: flags.debug,
            };

            let address = shim.start_shim(args).await?;

            tokio::io::stdout()
                .write_all(format!("{}", address).as_bytes())
                .await
                .map_err(io_error!(e, "write stdout"))?;

            Ok(())
        }
        "delete" => {
            tokio::spawn(async move {
                handle_signals(signals).await;
            });
            let response = shim.delete_shim().await?;
            let resp_bytes = response.write_to_bytes()?;
            tokio::io::stdout()
                .write_all(resp_bytes.as_slice())
                .await
                .map_err(io_error!(e, "failed to write response"))?;

            Ok(())
        }
        _ => {
            if !config.no_setup_logger {
                logger::init(flags.debug)?;
            }

            let task = shim.get_task_service().await;
            let task_service = create_task(Arc::new(Box::new(task)));
            let mut server = Server::new().register_service(task_service);
            server = server.add_listener(SOCKET_FD)?;
            server.start().await?;

            info!("Shim successfully started, waiting for exit signal...");
            tokio::spawn(async move {
                handle_signals(signals).await;
            });
            shim.wait().await;

            info!("Shutting down shim instance");
            server.shutdown().await.unwrap_or_default();

            // NOTE: If the shim server is down(like oom killer), the address
            // socket might be leaking.
            if let Ok(address) = read_file_to_str("address").await {
                remove_socket_silently(&address).await;
            }
            Ok(())
        }
    }
}

/// Spawn is a helper func to launch shim process asynchronously.
/// Typically this expected to be called from `StartShim`.
pub async fn spawn(opts: StartOpts, grouping: &str, vars: Vec<(&str, &str)>) -> Result<String> {
    let cmd = env::current_exe().map_err(io_error!(e, ""))?;
    let cwd = env::current_dir().map_err(io_error!(e, ""))?;
    let address = socket_address(&opts.address, &opts.namespace, grouping);

    // Create socket and prepare listener.
    // We'll use `add_listener` when creating TTRPC server.
    let listener = match start_listener(&address).await {
        Ok(l) => l,
        Err(e) => {
            if let Error::IoError {
                context: ref c,
                err: ref io_err,
            } = e
            {
                if io_err.kind() != std::io::ErrorKind::AddrInUse {
                    return Err(e);
                };
            }
            if let Ok(()) = wait_socket_working(&address, 5, 200).await {
                write_str_to_file("address", &address).await?;
                return Ok(address);
            }
            remove_socket(&address).await?;
            start_listener(&address).await?
        }
    };

    // tokio::process::Command do not have method `fd_mappings`,
    // and the `spawn()` is also not an async method,
    // so we use the std::process::Command here
    let mut command = Command::new(cmd);

    command
        .current_dir(cwd)
        .stdout(Stdio::null())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .args(&[
            "-namespace",
            &opts.namespace,
            "-id",
            &opts.id,
            "-address",
            &opts.address,
        ])
        .fd_mappings(vec![FdMapping {
            parent_fd: listener.as_raw_fd(),
            child_fd: SOCKET_FD,
        }])?;
    if opts.debug {
        command.arg("-debug");
    }
    command.envs(vars);

    command
        .spawn()
        .map_err(io_error!(e, "spawn shim"))
        .map(|_| {
            // Ownership of `listener` has been passed to child.
            std::mem::forget(listener);
            address
        })
}

fn setup_signals_tokio(config: &Config) -> Signals {
    let signals = if config.no_reaper {
        Signals::new(&[SIGTERM, SIGINT, SIGPIPE]).expect("new signal failed")
    } else {
        Signals::new(&[SIGTERM, SIGINT, SIGPIPE, SIGCHLD]).expect("new signal failed")
    };
    signals
}

async fn handle_signals(signals: Signals) {
    let mut signals = signals.fuse();
    while let Some(sig) = signals.next().await {
        match sig {
            SIGTERM | SIGINT => {
                debug!("received {}", sig);
                return;
            }
            SIGCHLD => {
                let mut status: c_int = 0;
                let options: c_int = libc::WNOHANG;
                let res_pid = asyncify(move || -> Result<pid_t> {
                    Ok(unsafe { libc::waitpid(-1, &mut status, options) })
                })
                .await
                .unwrap_or(-1);
                let status = unsafe { libc::WEXITSTATUS(status) };
                if res_pid > 0 {
                    monitor_notify_by_pid(res_pid, status)
                        .await
                        .unwrap_or_else(|e| {
                            error!("failed to send pid exit event {}", e);
                        })
                }
            }
            _ => {}
        }
    }
}

async fn remove_socket_silently(address: &str) {
    remove_socket(address)
        .await
        .unwrap_or_else(|e| warn!("failed to remove socket: {}", e))
}

async fn remove_socket(address: &str) -> Result<()> {
    let path = parse_sockaddr(address);
    if let Ok(md) = Path::new(path).metadata() {
        if md.file_type().is_socket() {
            tokio::fs::remove_file(path).await.map_err(io_error!(
                e,
                "failed to remove socket {}",
                address
            ))?;
        }
    }
    Ok(())
}

async fn start_listener(address: &str) -> Result<UnixListener> {
    let addr = address.to_string();
    asyncify(move || -> Result<UnixListener> {
        crate::start_listener(&addr).map_err(|e| Error::IoError {
            context: format!("failed to start listener {}", addr),
            err: e,
        })
    })
    .await
}

async fn wait_socket_working(address: &str, interval_in_ms: u64, count: u32) -> Result<()> {
    for _i in 0..count {
        match Client::connect(address) {
            Ok(_) => {
                return Ok(());
            }
            Err(e) => {
                tokio::time::sleep(std::time::Duration::from_millis(interval_in_ms)).await;
            }
        }
    }
    Err(other!(address, "time out waiting for socket"))
}
