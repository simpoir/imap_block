use async_std::net::TcpStream;
use async_std::task::sleep;
use log::{self, debug, error};
use std::env::args;
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

/// Write json block status to stdout, setting percentage as 100 if any unread.
fn dump_status(new_count: usize, count: u32) {
    if new_count > 0 {
        println!(
            "{{\"full_text\": \"({}) {}\", \"color\": \"#00cc00\"}}",
            new_count, count
        );
    } else {
        println!("{{\"full_text\": \"{}\", \"color\": \"\"}}", count);
    };
}

#[async_std::main]
async fn main() {
    // env RUST_LOG=debug
    env_logger::init();

    let mut argv = args();
    let _prog = argv.next().unwrap();
    let cred_res = match argv.next() {
        Some(conf_path) => creds::Creds::from_mutt(&conf_path).await,
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

            dump_status(new_count, count);
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
