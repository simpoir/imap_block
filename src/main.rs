use async_std::net::TcpStream;
use async_std::task::sleep;
use log::{self, debug, error};
use std::process::exit;
use std::time::Duration;

mod backoff;
mod creds;
mod errors;

const POLL: u64 = 300;
const KEEP_ALIVE: u64 = 1700;

macro_rules! fatal {
    ($val: literal, $msg: literal) => {
        |e| {
            error!($msg, e);
            std::process::exit($val);
        }
    };
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum OutputMode {
    I3,
    Waybar,
}

impl OutputMode {
    /// Write json block status to stdout, setting percentage as 100 if any unread.
    fn dump_status(&self, new_count: usize, count: u32) {
        let flagged = new_count > 0;
        match self {
            OutputMode::I3 => {
                println!(
                    "{{\"full_text\": \"({}) {}\", \"color\": \"{}\"}}",
                    new_count,
                    count,
                    if flagged { "#00cc00" } else { "" }
                )
            }
            OutputMode::Waybar => println!(
                "{{\"text\": \"({}) {}\", \"alt\": \"{}\"}}",
                new_count, count, flagged
            ),
        }
    }
}

#[derive(clap::Parser, Debug)]
struct Args {
    #[clap(short, long, default_value = "i3")]
    mode: OutputMode,

    /// Credentials file, in muttrc format (default: stdin)
    cred_file: Option<std::path::PathBuf>,
}

#[async_std::main]
async fn main() {
    // env RUST_LOG=debug
    env_logger::init();

    let args: Args = clap::Parser::parse();

    let cred_res = match &args.cred_file {
        Some(conf_path) => creds::Creds::from_mutt(async_std::path::Path::new(conf_path)).await,
        None => creds::Creds::from_stdin(),
    };
    let cred = cred_res.unwrap_or_else(fatal!(1, "Problem reading config: {}"));

    let host = cred.host.as_str();
    let mut backoff = backoff::Backoff::new(&[0, 60, 120, 500, 600]);
    'retrying: loop {
        sleep(Duration::from_secs(backoff.next())).await;
        let stream = TcpStream::connect((host, cred.port))
            .await
            .unwrap_or_else(fatal!(2, "Failure connecting: {}"));
        let tls = async_native_tls::TlsConnector::new();
        let tls_stream = tls
            .connect(host, stream)
            .await
            .unwrap_or_else(fatal!(2, "Error establishing TLS: {}"));
        let c = async_imap::Client::new(tls_stream);
        let mut s = match c.login(cred.user.as_str(), cred.pass.as_str()).await {
            Ok(s) => s,
            Err((e, _)) => {
                error!("Failure logging in: {}", e);
                std::process::exit(2);
            }
        };
        debug!("logged in successfully");

        let can_idle = match s.capabilities().await {
            Ok(cap) => cap.has_str("IDLE"),
            Err(e) => {
                error!("Failure listing caps: {}", e);
                exit(2);
            }
        };
        debug!("Server can IDLE: {}", can_idle);

        'poll: loop {
            let count = match s.examine("INBOX").await {
                Ok(mb) => mb.exists,
                Err(e) => {
                    debug!("Failure listing mailbox: {}", e);
                    continue 'retrying;
                }
            };

            let new_count = match s.search("UNSEEN").await {
                Ok(ids) => ids.len(),
                Err(e) => {
                    debug!("Failure searching unread: {}", e);
                    continue 'retrying;
                }
            };

            args.mode.dump_status(new_count, count);
            backoff.reset();

            if !can_idle {
                sleep(Duration::from_secs(POLL)).await;
                continue 'poll;
            }

            debug!("idling");
            let mut idle = s.idle();
            if let Err(e) = idle.init().await {
                debug!("Failed to idle: {}", e);
                continue 'retrying;
            };
            let (fut, stopper) = idle.wait_with_timeout(Duration::from_secs(KEEP_ALIVE));
            if let Err(e) = fut.await {
                debug!("Failed while idle: {}", e);
                continue 'retrying;
            };
            s = match idle.done().await {
                Ok(s) => s,
                Err(e) => {
                    debug!("Failed to end idle: {}", e);
                    continue 'retrying;
                }
            };
            drop(stopper); // drop only after waiting to avoid early return
            debug!("done idling");
        }
    }
}
